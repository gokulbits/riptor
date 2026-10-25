//! Seed/peer counts from tracker scrapes. Public addresses only.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use futures::StreamExt;
use librqbit_bencode::ByteBuf;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use url::Url;

use crate::SearchResult;

pub const MAX_RESULTS: usize = 60;
const MAX_TRACKERS: usize = 3;
const CONCURRENCY: usize = 6;
const TRACKER_TIMEOUT: Duration = Duration::from_secs(6);
const UDP_ATTEMPT: Duration = Duration::from_secs(3);
const MAX_SCRAPE_BYTES: usize = 64 * 1024;
pub const MAX_TORRENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_COUNT: u32 = 10_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub info_hash: [u8; 20],
    pub trackers: Vec<Url>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub seeders: u32,
    pub leechers: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResultCounts {
    pub link: String,
    pub seeders: u32,
    pub leechers: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Magnet { link: String, target: Target },
    Torrent { link: String, url: Url },
}

pub fn job_for(r: &SearchResult) -> Option<Job> {
    if r.seeders.is_some() || r.leechers.is_some() || r.via_plugin {
        return None;
    }
    if r.link.starts_with("magnet:") {
        let target = magnet_target(&r.link)?;
        return (!target.trackers.is_empty()).then(|| Job::Magnet {
            link: r.link.clone(),
            target,
        });
    }
    let url = Url::parse(&r.link).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| Job::Torrent {
        link: r.link.clone(),
        url,
    })
}

fn tracker_url(s: &str) -> Option<Url> {
    if s.len() > 512 {
        return None;
    }
    let u = Url::parse(s.trim()).ok()?;
    (matches!(u.scheme(), "http" | "https" | "udp") && u.host_str().is_some_and(|h| !h.is_empty()))
        .then_some(u)
}

fn push_tracker(list: &mut Vec<Url>, u: Url) {
    if list.len() < 20 && !list.contains(&u) {
        list.push(u);
    }
}

pub fn parse_btih(s: &str) -> Option<[u8; 20]> {
    let mut out = [0u8; 20];
    match s.len() {
        40 => {
            for (i, pair) in s.as_bytes().chunks(2).enumerate() {
                let hex = std::str::from_utf8(pair).ok()?;
                out[i] = u8::from_str_radix(hex, 16).ok()?;
            }
        }
        32 => {
            let mut bits: u64 = 0;
            let (mut n, mut o) = (0u32, 0usize);
            for c in s.bytes() {
                let v = match c.to_ascii_uppercase() {
                    c @ b'A'..=b'Z' => c - b'A',
                    c @ b'2'..=b'7' => c - b'2' + 26,
                    _ => return None,
                };
                bits = (bits << 5) | u64::from(v);
                n += 5;
                if n >= 8 {
                    n -= 8;
                    out[o] = (bits >> n) as u8;
                    o += 1;
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

pub fn magnet_target(link: &str) -> Option<Target> {
    if link.len() > 64 * 1024 {
        return None;
    }
    let u = Url::parse(link).ok()?;
    if u.scheme() != "magnet" {
        return None;
    }
    let mut info_hash = None;
    let mut trackers = Vec::new();
    for (k, v) in u.query_pairs() {
        match &*k {
            "xt" => {
                if let Some(h) = v.strip_prefix("urn:btih:") {
                    info_hash = info_hash.or_else(|| parse_btih(h));
                }
            }
            "tr" => {
                if let Some(t) = tracker_url(&v) {
                    push_tracker(&mut trackers, t);
                }
            }
            _ => {}
        }
    }
    Some(Target {
        info_hash: info_hash?,
        trackers,
    })
}

pub fn torrent_target(bytes: &[u8]) -> Option<Target> {
    if bytes.len() > MAX_TORRENT_BYTES {
        return None;
    }
    let t = librqbit_core::torrent_metainfo::torrent_from_bytes(bytes).ok()?;
    let mut trackers = Vec::new();
    for a in t.iter_announce() {
        if let Some(u) = std::str::from_utf8(a.as_ref()).ok().and_then(tracker_url) {
            push_tracker(&mut trackers, u);
        }
    }
    Some(Target {
        info_hash: t.info_hash.0,
        trackers,
    })
}

/// `.../announce*` becomes `.../scrape*`.
pub fn http_scrape_url(announce: &Url, info_hash: &[u8; 20]) -> Option<Url> {
    let mut u = announce.clone();
    let last = u.path_segments()?.next_back()?.to_string();
    let rest = last.strip_prefix("announce")?;
    u.path_segments_mut()
        .ok()?
        .pop()
        .push(&format!("scrape{rest}"));
    let enc: String = info_hash
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(b) {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let q = match u.query() {
        Some(q) if !q.is_empty() => format!("{q}&info_hash={enc}"),
        _ => format!("info_hash={enc}"),
    };
    u.set_query(Some(&q));
    u.set_fragment(None);
    Some(u)
}

#[derive(Deserialize)]
struct ScrapeReply<'a> {
    #[serde(borrow, default)]
    files: HashMap<ByteBuf<'a>, FileStats>,
    #[serde(borrow, default, rename = "failure reason")]
    failure: Option<ByteBuf<'a>>,
}

#[derive(Deserialize)]
struct FileStats {
    #[serde(default)]
    complete: i64,
    #[serde(default)]
    incomplete: i64,
}

fn count(n: i64) -> u32 {
    n.clamp(0, i64::from(MAX_COUNT)) as u32
}

/// A valid reply that doesn't mention the hash means 0 and 0.
pub fn parse_http_reply(body: &[u8], info_hash: &[u8; 20]) -> anyhow::Result<Counts> {
    ensure!(body.len() <= MAX_SCRAPE_BYTES, "scrape reply too large");
    let reply: ScrapeReply = librqbit_bencode::from_bytes(body)
        .map_err(|e| anyhow::anyhow!("bad scrape reply: {}", e.kind()))?;
    if reply.failure.is_some() {
        bail!("tracker refused the scrape");
    }
    Ok(reply
        .files
        .get(&ByteBuf(&info_hash[..]))
        .map(|f| Counts {
            seeders: count(f.complete),
            leechers: count(f.incomplete),
        })
        .unwrap_or_default())
}

pub use riptor_net::is_public;

/// Resolves `host` (the port is passed through), e.g. through a VPN tunnel.
pub type Lookup = Arc<
    dyn Fn(String, u16) -> futures::future::BoxFuture<'static, std::io::Result<Vec<SocketAddr>>>
        + Send
        + Sync,
>;

/// Send everything through `end` and resolve with `lookup` (the VPN tunnel).
#[derive(Clone)]
pub struct Tunnel {
    pub end: riptor_net::TunnelEnd,
    pub lookup: Lookup,
}

async fn lookup(
    tunnel: Option<&Tunnel>,
    host: &str,
    port: u16,
) -> std::io::Result<Vec<SocketAddr>> {
    match tunnel {
        Some(t) => Ok((t.lookup)(host.to_string(), port)
            .await?
            .into_iter()
            .filter(|a| a.is_ipv4() == t.end.ip.is_ipv4())
            .collect()),
        None => Ok(tokio::net::lookup_host((host, port)).await?.collect()),
    }
}

/// `allow_local` is for tests.
#[derive(Clone)]
pub struct Net {
    pub http: reqwest::Client,
    pub allow_local: bool,
    tunnel: Option<Tunnel>,
}

struct PublicOnly {
    allow_local: bool,
    tunnel: Option<Tunnel>,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow = self.allow_local;
        let tunnel = self.tunnel.clone();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = lookup(tunnel.as_ref(), name.as_str(), 0)
                .await?
                .into_iter()
                .filter(|a| allow || is_public(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("refusing to contact a local or private address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn host_allowed(u: &Url, allow_local: bool) -> bool {
    allow_local || riptor_net::host_allowed(u)
}

impl Net {
    pub fn new(allow_local: bool) -> anyhow::Result<Self> {
        Self::build(allow_local, None)
    }

    pub fn through(tunnel: Tunnel, allow_local: bool) -> anyhow::Result<Self> {
        Self::build(allow_local, Some(tunnel))
    }

    fn build(allow_local: bool, tunnel: Option<Tunnel>) -> anyhow::Result<Self> {
        let mut b = reqwest::Client::builder();
        if let Some(t) = &tunnel {
            b = t.end.http(b);
        }
        let http = b
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .dns_resolver(Arc::new(PublicOnly {
                allow_local,
                tunnel: tunnel.clone(),
            }))
            .redirect(reqwest::redirect::Policy::custom(move |a| {
                if a.previous().len() >= 5 {
                    a.error("too many redirects")
                } else if !matches!(a.url().scheme(), "http" | "https")
                    || !host_allowed(a.url(), allow_local)
                {
                    a.stop()
                } else {
                    a.follow()
                }
            }))
            .user_agent(concat!("Riptor/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            allow_local,
            tunnel,
        })
    }

    async fn get_capped(&self, url: Url, cap: usize) -> anyhow::Result<Vec<u8>> {
        ensure!(
            host_allowed(&url, self.allow_local),
            "refusing to contact a local or private address"
        );
        let resp = self.http.get(url).send().await?.error_for_status()?;
        if let Some(len) = resp.content_length() {
            ensure!(len <= cap as u64, "response too large");
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            ensure!(body.len() + chunk.len() <= cap, "response too large");
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn http_scrape(&self, announce: &Url, info_hash: &[u8; 20]) -> anyhow::Result<Counts> {
        let url = http_scrape_url(announce, info_hash).context("tracker has no scrape URL")?;
        let host = url.host_str().unwrap_or("").to_string();
        let body = self.get_capped(url, MAX_SCRAPE_BYTES).await;
        riptor_net::ledger::record(riptor_net::ledger::Purpose::Scrape, &host, body.is_ok());
        parse_http_reply(&body?, info_hash)
    }

    async fn resolve_udp(&self, u: &Url) -> anyhow::Result<SocketAddr> {
        let host = u.host_str().context("no host")?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = u.port().context("UDP tracker has no port")?;
        let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) => lookup(self.tunnel.as_ref(), host, port).await?,
        };
        addrs
            .into_iter()
            .find(|a| self.allow_local || is_public(a.ip()))
            .context("refusing to contact a local or private address")
    }

    async fn udp_scrape(&self, tracker: &Url, info_hash: &[u8; 20]) -> anyhow::Result<Counts> {
        let got = self.udp_scrape_once(tracker, info_hash).await;
        riptor_net::ledger::record(
            riptor_net::ledger::Purpose::Scrape,
            tracker.host_str().unwrap_or(""),
            got.is_ok(),
        );
        got
    }

    async fn udp_scrape_once(&self, tracker: &Url, info_hash: &[u8; 20]) -> anyhow::Result<Counts> {
        let addr = self.resolve_udp(tracker).await?;
        let sock = match (&self.tunnel, addr.is_ipv4(), addr.ip().is_loopback()) {
            (Some(t), _, _) => t.end.udp_socket(0).await?,
            (None, true, true) => UdpSocket::bind("127.0.0.1:0").await?,
            (None, true, false) => UdpSocket::bind("0.0.0.0:0").await?,
            (None, false, true) => UdpSocket::bind("[::1]:0").await?,
            (None, false, false) => UdpSocket::bind("[::]:0").await?,
        };
        sock.connect(addr).await?;

        let tx = transaction_id();
        let mut req = Vec::with_capacity(16);
        req.extend_from_slice(&0x0417_2710_1980u64.to_be_bytes());
        req.extend_from_slice(&0u32.to_be_bytes());
        req.extend_from_slice(&tx.to_be_bytes());
        let resp = udp_exchange(&sock, &req, tx).await?;
        ensure!(
            resp.len() >= 16 && be32(&resp, 0) == 0,
            "bad UDP connect reply"
        );
        let conn: [u8; 8] = resp[8..16].try_into()?;

        let tx = transaction_id();
        let mut req = Vec::with_capacity(36);
        req.extend_from_slice(&conn);
        req.extend_from_slice(&2u32.to_be_bytes());
        req.extend_from_slice(&tx.to_be_bytes());
        req.extend_from_slice(info_hash);
        let resp = udp_exchange(&sock, &req, tx).await?;
        parse_udp_scrape(&resp)
    }

    async fn scrape_tracker(&self, tracker: &Url, info_hash: &[u8; 20]) -> anyhow::Result<Counts> {
        let work = async {
            match tracker.scheme() {
                "udp" => self.udp_scrape(tracker, info_hash).await,
                _ => self.http_scrape(tracker, info_hash).await,
            }
        };
        tokio::time::timeout(TRACKER_TIMEOUT, work)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("tracker timed out")))
    }

    /// The highest counts win.
    pub async fn scrape(&self, target: &Target) -> Option<Counts> {
        let asks = target
            .trackers
            .iter()
            .filter(|t| t.scheme() == "udp" || http_scrape_url(t, &target.info_hash).is_some())
            .take(MAX_TRACKERS)
            .map(|t| self.scrape_tracker(t, &target.info_hash));
        futures::future::join_all(asks)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .reduce(|a, b| Counts {
                seeders: a.seeders.max(b.seeders),
                leechers: a.leechers.max(b.leechers),
            })
    }

    async fn run_job(&self, job: Job) -> Option<ResultCounts> {
        let (link, target) = match job {
            Job::Magnet { link, target } => (link, target),
            Job::Torrent { link, url } => {
                let bytes = self.get_capped(url, MAX_TORRENT_BYTES).await.ok()?;
                (link, torrent_target(&bytes)?)
            }
        };
        let c = self.scrape(&target).await?;
        Some(ResultCounts {
            link,
            seeders: c.seeders,
            leechers: c.leechers,
        })
    }

    pub async fn run(self, rx: mpsc::UnboundedReceiver<Job>, emit: impl Fn(Vec<ResultCounts>)) {
        let jobs =
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|j| (j, rx)) })
                .take(MAX_RESULTS);
        let me = &self;
        let done = jobs
            .map(|j| me.run_job(j))
            .buffer_unordered(CONCURRENCY)
            .filter_map(futures::future::ready)
            .ready_chunks(16);
        let mut done = std::pin::pin!(done);
        while let Some(batch) = done.next().await {
            emit(batch);
        }
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn parse_udp_scrape(resp: &[u8]) -> anyhow::Result<Counts> {
    ensure!(resp.len() >= 8, "short UDP reply");
    match be32(resp, 0) {
        2 if resp.len() >= 20 => Ok(Counts {
            seeders: be32(resp, 8).min(MAX_COUNT),
            leechers: be32(resp, 16).min(MAX_COUNT),
        }),
        3 => bail!("tracker refused the scrape"),
        _ => bail!("bad UDP scrape reply"),
    }
}

/// Not a secret.
fn transaction_id() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish() as u32
}

async fn udp_exchange(sock: &UdpSocket, req: &[u8], tx: u32) -> anyhow::Result<Vec<u8>> {
    let mut buf = [0u8; 2048];
    for _ in 0..2 {
        sock.send(req).await?;
        let wait = async {
            loop {
                let n = sock.recv(&mut buf).await?;
                if n >= 8 && be32(&buf, 4) == tx {
                    return anyhow::Ok(buf[..n].to_vec());
                }
            }
        };
        if let Ok(r) = tokio::time::timeout(UDP_ATTEMPT, wait).await {
            return r;
        }
    }
    bail!("UDP tracker did not answer")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const HASH_HEX: &str = "93f9c262803e89862ae3479a612ee506ef2c096c";

    fn hash() -> [u8; 20] {
        parse_btih(HASH_HEX).unwrap()
    }

    fn reply_with(hash: &[u8; 20], complete: i64, incomplete: i64) -> Vec<u8> {
        let mut b = b"d5:filesd20:".to_vec();
        b.extend_from_slice(hash);
        b.extend_from_slice(
            format!("d8:completei{complete}e10:downloadedi5e10:incompletei{incomplete}eeee")
                .as_bytes(),
        );
        b
    }

    #[test]
    fn info_hashes_and_magnets() {
        assert_eq!(hash()[0], 0x93);
        assert_eq!(
            parse_btih("SP44EYUAH2EYMKXDI6NGCLXFA3XSYCLM"),
            Some(hash()),
            "base32 form"
        );
        assert!(parse_btih("zz").is_none());
        assert!(parse_btih(&"g".repeat(40)).is_none());
        let t = magnet_target(&format!(
            "magnet:?xt=urn:btih:{HASH_HEX}&tr=udp%3A%2F%2Ftracker.example%3A1337&tr=javascript%3Aalert(1)&tr=http%3A%2F%2Ft.example%2Fannounce"
        ))
        .unwrap();
        assert_eq!(t.info_hash, hash());
        assert_eq!(t.trackers.len(), 2, "non-tracker schemes are dropped");
        assert!(magnet_target("magnet:?dn=no-hash").is_none());
        assert!(magnet_target("https://example.org/?xt=urn:btih:00").is_none());
    }

    #[test]
    fn jobs_skip_results_with_counts_and_plugin_links() {
        let mut r = SearchResult {
            name: "x".into(),
            link: "https://archive.org/download/a/a_archive.torrent".into(),
            size: None,
            seeders: None,
            leechers: None,
            desc_link: None,
            pub_date: None,
            engine: "archive".into(),
            via_plugin: false,
        };
        assert!(matches!(job_for(&r), Some(Job::Torrent { .. })));
        r.via_plugin = true;
        assert!(job_for(&r).is_none());
        r.via_plugin = false;
        r.seeders = Some(0);
        assert!(job_for(&r).is_none(), "providers' own counts are kept");
        r.seeders = None;
        r.link = format!("magnet:?xt=urn:btih:{HASH_HEX}");
        assert!(job_for(&r).is_none(), "magnet without trackers");
    }

    #[test]
    fn scrape_urls() {
        let h = hash();
        let u = |s: &str| http_scrape_url(&Url::parse(s).unwrap(), &h).map(|u| u.to_string());
        assert_eq!(
            u("http://bt1.archive.org:6969/announce").unwrap(),
            "http://bt1.archive.org:6969/scrape?info_hash=%93%F9%C2b%80%3E%89%86%2A%E3G%9Aa.%E5%06%EF%2C%09l"
        );
        assert_eq!(
            u("https://t.example/x/announce.php?passkey=1").unwrap(),
            "https://t.example/x/scrape.php?passkey=1&info_hash=%93%F9%C2b%80%3E%89%86%2A%E3G%9Aa.%E5%06%EF%2C%09l"
        );
        assert!(u("http://t.example/a").is_none(), "no announce segment");
    }

    #[test]
    fn http_replies_are_parsed_strictly() {
        let h = hash();
        assert_eq!(
            parse_http_reply(&reply_with(&h, 12, 3), &h).unwrap(),
            Counts {
                seeders: 12,
                leechers: 3
            }
        );
        assert_eq!(
            parse_http_reply(b"d5:filesdee", &h).unwrap(),
            Counts::default()
        );
        let c = parse_http_reply(&reply_with(&h, -5, 99_999_999_999), &h).unwrap();
        assert_eq!((c.seeders, c.leechers), (0, MAX_COUNT));
        assert!(parse_http_reply(b"d14:failure reason6:nope!e", &h).is_err());
        assert!(parse_http_reply(b"<html>not bencode</html>", &h).is_err());
        assert!(parse_http_reply(b"d5:filesd", &h).is_err(), "truncated");
        assert!(parse_http_reply(b"d5:filesdee trailing", &h).is_err());
        assert!(parse_http_reply(&vec![b'l'; MAX_SCRAPE_BYTES + 1], &h).is_err());
        assert!(parse_http_reply(&[b'l'; 5000], &h).is_err(), "deep nesting");
    }

    #[test]
    fn udp_replies() {
        let mut ok = vec![0, 0, 0, 2, 1, 2, 3, 4];
        ok.extend_from_slice(&7u32.to_be_bytes());
        ok.extend_from_slice(&100u32.to_be_bytes());
        ok.extend_from_slice(&u32::MAX.to_be_bytes());
        let c = parse_udp_scrape(&ok).unwrap();
        assert_eq!((c.seeders, c.leechers), (7, MAX_COUNT));
        assert!(parse_udp_scrape(&ok[..12]).is_err());
        assert!(parse_udp_scrape(b"\0\0\0\x03abcdnope").is_err());
    }

    #[tokio::test]
    async fn local_trackers_are_not_contacted() {
        let net = Net::new(false).unwrap();
        let t = Target {
            info_hash: hash(),
            trackers: [
                "http://127.0.0.1:9/announce",
                "udp://192.168.1.1:80",
                "http://localhost:9/announce",
            ]
            .iter()
            .map(|s| Url::parse(s).unwrap())
            .collect(),
        };
        assert_eq!(net.scrape(&t).await, None);
        let job = Job::Torrent {
            link: "x".into(),
            url: Url::parse("http://127.0.0.1:9/x.torrent").unwrap(),
        };
        assert_eq!(net.run_job(job).await, None);
    }

    async fn http_server(body: Vec<u8>) -> SocketAddr {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&body).await;
                });
            }
        });
        addr
    }

    async fn udp_tracker() -> SocketAddr {
        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = s.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = s.recv_from(&mut buf).await {
                let tx = &buf[12..16];
                let mut out = Vec::new();
                if n == 16 && buf[..8] == 0x0417_2710_1980u64.to_be_bytes() {
                    out.extend_from_slice(&0u32.to_be_bytes());
                    out.extend_from_slice(tx);
                    out.extend_from_slice(&[7; 8]);
                } else if n >= 36 && buf[..8] == [7; 8] && be32(&buf, 8) == 2 {
                    out.extend_from_slice(&2u32.to_be_bytes());
                    out.extend_from_slice(tx);
                    for v in [42u32, 1000, 9] {
                        out.extend_from_slice(&v.to_be_bytes());
                    }
                }
                let _ = s.send_to(&out, from).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn tunnel_lookups_use_the_given_resolver() {
        let h = hash();
        let http = http_server(reply_with(&h, 5, 30)).await;
        let udp = udp_tracker().await;
        // Only this resolver knows these names; the system resolver would fail.
        let (hp, up) = (http.port(), udp.port());
        let lookup: Lookup = Arc::new(move |host: String, port: u16| {
            Box::pin(async move {
                match host.as_str() {
                    "http-tracker.riptor-test" | "udp-tracker.riptor-test" => {
                        Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
                    }
                    _ => Err(std::io::Error::other("unknown host")),
                }
            })
        });
        let net = Net::through(
            Tunnel {
                end: riptor_net::TunnelEnd::new(IpAddr::from([127, 0, 0, 1]), "lo"),
                lookup,
            },
            true,
        )
        .unwrap();
        let t = Target {
            info_hash: h,
            trackers: vec![
                Url::parse(&format!("http://http-tracker.riptor-test:{hp}/announce")).unwrap(),
                Url::parse(&format!("udp://udp-tracker.riptor-test:{up}")).unwrap(),
            ],
        };
        assert_eq!(
            net.scrape(&t).await,
            Some(Counts {
                seeders: 42,
                leechers: 30
            })
        );
    }

    #[tokio::test]
    async fn scrapes_http_and_udp_trackers() {
        let h = hash();
        let http = http_server(reply_with(&h, 5, 30)).await;
        let udp = udp_tracker().await;
        let net = Net::new(true).unwrap();
        let t = Target {
            info_hash: h,
            trackers: vec![
                Url::parse(&format!("http://{http}/announce")).unwrap(),
                Url::parse(&format!("udp://{udp}")).unwrap(),
            ],
        };
        assert_eq!(
            net.scrape(&t).await,
            Some(Counts {
                seeders: 42,
                leechers: 30
            })
        );

        let torrent = {
            let mut b = format!("d8:announce{}:", format!("udp://{udp}").len()).into_bytes();
            b.extend_from_slice(format!("udp://{udp}").as_bytes());
            b.extend_from_slice(
                b"4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
            );
            b
        };
        let file = http_server(torrent).await;
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(Job::Torrent {
            link: "L".into(),
            url: Url::parse(&format!("http://{file}/x.torrent")).unwrap(),
        })
        .unwrap();
        drop(tx);
        let got = parking_lot::Mutex::new(Vec::new());
        net.run(rx, |b| got.lock().extend(b)).await;
        assert_eq!(
            got.into_inner(),
            vec![ResultCounts {
                link: "L".into(),
                seeders: 42,
                leechers: 9
            }]
        );
    }
}
