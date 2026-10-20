//! Loopback HTTP proxy that sends plugin traffic out through the VPN tunnel.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::task::JoinSet;
use url::Url;

use crate::scrape::{is_public, Tunnel};

const MAX_HEAD: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 32;
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Stops, with its connections, when dropped.
pub struct Proxy {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    /// `allow_local` is for tests.
    pub async fn start(tunnel: Tunnel, allow_local: bool) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let mut conns = JoinSet::new();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                while conns.try_join_next().is_some() {}
                if conns.len() >= MAX_CONNECTIONS {
                    continue;
                }
                let t = tunnel.clone();
                conns.spawn(async move {
                    if let Err(e) = serve(stream, &t, allow_local).await {
                        tracing::debug!("plugin proxy: {e:#}");
                    }
                });
            }
        });
        Ok(Self { addr, task })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(stream: TcpStream, t: &Tunnel, allow_local: bool) -> anyhow::Result<()> {
    let mut client = BufReader::new(stream);
    let head = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client))
        .await
        .context("request head timed out")??;
    let (upstream, head) = match open(&head, t, allow_local).await {
        Ok(v) => v,
        Err(e) => {
            let _ = client
                .get_mut()
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
            return Err(e);
        }
    };
    let mut upstream = upstream;
    match head {
        Some(h) => upstream.write_all(h.as_bytes()).await?,
        None => {
            client
                .get_mut()
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await?
        }
    }
    // Bytes the client sent after the head.
    let early = client.buffer().to_vec();
    upstream.write_all(&early).await?;
    let mut client = client.into_inner();
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn read_head(r: &mut BufReader<TcpStream>) -> anyhow::Result<String> {
    let mut head = Vec::new();
    loop {
        let n = r.read_until(b'\n', &mut head).await?;
        ensure!(n > 0, "client closed early");
        ensure!(head.len() <= MAX_HEAD, "request head too large");
        if head.ends_with(b"\r\n\r\n") || head == b"\r\n" {
            break;
        }
    }
    Ok(String::from_utf8(head)?)
}

/// The upstream connection, and the head to send on it (none for CONNECT).
async fn open(
    head: &str,
    t: &Tunnel,
    allow_local: bool,
) -> anyhow::Result<(TcpStream, Option<String>)> {
    let (first, rest) = head.split_once("\r\n").context("bad request")?;
    let mut parts = first.split(' ');
    let (method, target, version) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v)) if parts.next().is_none() => (m, t, v),
        _ => bail!("bad request line"),
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = target.rsplit_once(':').context("CONNECT needs host:port")?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port: u16 = port.parse()?;
        return Ok((dial(t, host, port, allow_local).await?, None));
    }
    let url = Url::parse(target)?;
    ensure!(url.scheme() == "http", "only http:// can be forwarded");
    let host = url.host_str().context("no host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port_or_known_default().context("no port")?;
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let mut out = format!("{method} {path} {version}\r\n");
    for line in rest.split("\r\n").filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or("").trim();
        if name.to_ascii_lowercase().starts_with("proxy-")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    // One request per connection, so the next one can't go to another host unchecked.
    out.push_str("Connection: close\r\n\r\n");
    Ok((dial(t, host, port, allow_local).await?, Some(out)))
}

async fn dial(t: &Tunnel, host: &str, port: u16, allow_local: bool) -> anyhow::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => (t.lookup)(host.to_string(), port).await?,
    };
    let addrs: Vec<SocketAddr> = addrs
        .into_iter()
        .filter(|a| a.is_ipv4() == t.local.is_ipv4())
        .filter(|a| allow_local || is_public(a.ip()))
        .collect();
    ensure!(
        !addrs.is_empty(),
        "refusing to contact {host}: not a public address"
    );
    let mut last = None;
    for a in addrs {
        let sock = if a.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        sock.bind(SocketAddr::new(t.local, 0))?;
        match tokio::time::timeout(CONNECT_TIMEOUT, sock.connect(a)).await {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) => last = Some(anyhow::Error::from(e)),
            Err(_) => last = Some(anyhow::anyhow!("connecting to {a} timed out")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no address for {host}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;

    fn loopback_tunnel() -> Tunnel {
        Tunnel {
            local: IpAddr::V4(Ipv4Addr::LOCALHOST),
            lookup: Arc::new(|_, port| {
                Box::pin(async move { Ok(vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))]) })
            }),
        }
    }

    /// Answers once with what it received.
    async fn echo_server() -> (SocketAddr, tokio::task::JoinHandle<String>) {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = l.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let got = String::from_utf8_lossy(&buf[..n]).to_string();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
            got
        });
        (addr, task)
    }

    async fn ask(proxy: &Proxy, req: String) -> String {
        let mut s = TcpStream::connect(proxy.addr).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut out)).await;
        out
    }

    #[tokio::test]
    async fn forwards_plain_http_in_origin_form() {
        let (server, got) = echo_server().await;
        let proxy = Proxy::start(loopback_tunnel(), true).await.unwrap();
        let reply = ask(
            &proxy,
            format!(
                "GET http://example.test:{}/search?q=a%20b HTTP/1.1\r\nHost: example.test\r\nProxy-Authorization: x\r\nConnection: keep-alive\r\n\r\n",
                server.port()
            ),
        )
        .await;
        assert!(reply.ends_with("ok"), "{reply}");
        let got = got.await.unwrap();
        assert!(got.starts_with("GET /search?q=a%20b HTTP/1.1\r\n"), "{got}");
        assert!(got.contains("Host: example.test\r\n"));
        assert!(got.contains("Connection: close\r\n"));
        assert!(!got.contains("Proxy-Authorization"));
        assert!(!got.contains("keep-alive"));
    }

    #[tokio::test]
    async fn tunnels_connect() {
        let (server, got) = echo_server().await;
        let proxy = Proxy::start(loopback_tunnel(), true).await.unwrap();
        let reply = ask(
            &proxy,
            format!(
                "CONNECT example.test:{0} HTTP/1.1\r\nHost: example.test:{0}\r\n\r\nGET / HTTP/1.1\r\n\r\n",
                server.port()
            ),
        )
        .await;
        assert!(
            reply.starts_with("HTTP/1.1 200 Connection established\r\n\r\n"),
            "{reply}"
        );
        assert!(reply.ends_with("ok"));
        assert_eq!(got.await.unwrap(), "GET / HTTP/1.1\r\n\r\n");
    }

    #[tokio::test]
    async fn refuses_private_addresses_and_other_schemes() {
        let proxy = Proxy::start(loopback_tunnel(), false).await.unwrap();
        for req in [
            "CONNECT example.test:443 HTTP/1.1\r\n\r\n",
            "CONNECT 192.168.1.1:443 HTTP/1.1\r\n\r\n",
            "GET http://127.0.0.1:1/ HTTP/1.1\r\n\r\n",
            "GET ftp://example.test/ HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\n\r\n",
        ] {
            let reply = ask(&proxy, req.to_string()).await;
            assert!(reply.starts_with("HTTP/1.1 502"), "{req:?} -> {reply}");
        }
    }
}
