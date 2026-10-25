//! The listening side: who may connect, the self-signed certificate, and TLS handshakes kept off
//! the accept loop.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_util::either::Either;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HANDSHAKES: usize = 32;

/// Private, link-local and loopback ranges: the local network, as far as an address can tell.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local_network(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

/// Never through the VPN tunnel; outside the local network only when allowed.
pub fn allowed(remote: IpAddr, local: IpAddr, allow_public: bool, vpn: Option<Ipv4Addr>) -> bool {
    refusal(remote, local, allow_public, vpn).is_none()
}

/// Why a client may not connect, if it may not.
pub fn refusal(
    remote: IpAddr,
    local: IpAddr,
    allow_public: bool,
    vpn: Option<Ipv4Addr>,
) -> Option<&'static str> {
    let local = match local {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(local, IpAddr::V4),
        v4 => v4,
    };
    if vpn.is_some_and(|v| IpAddr::V4(v) == local) {
        return Some("it came in through the VPN tunnel");
    }
    if remote.is_loopback() || allow_public || is_local_network(remote) {
        None
    } else {
        Some("it isn't on the local network (turn on \"Accept connections from outside the local network\" to allow it)")
    }
}

/// Logs a connection problem, at most once a minute for each address and kind.
pub fn note(ip: IpAddr, kind: &'static str, text: std::fmt::Arguments<'_>) {
    use std::collections::HashMap;
    use std::time::Instant;
    static SEEN: parking_lot::Mutex<Option<HashMap<(IpAddr, &'static str), Instant>>> =
        parking_lot::Mutex::new(None);
    {
        let mut g = SEEN.lock();
        let seen = g.get_or_insert_with(HashMap::new);
        if seen.len() > 1024 {
            seen.retain(|_, t| t.elapsed() < Duration::from_secs(60));
        }
        let now = Instant::now();
        match seen.get(&(ip, kind)) {
            Some(t) if now.duration_since(*t) < Duration::from_secs(60) => return,
            _ => {
                seen.insert((ip, kind), now);
            }
        }
    }
    tracing::info!(target: "riptor::remote", "{text}");
}

pub struct Cert {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    /// SHA-256 of the certificate, as `AB:CD:...`.
    pub fingerprint: String,
}

fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Riptor's own certificate in `dir`, made on first use; the key file is owner-only.
pub fn load_or_create_cert(dir: &Path) -> anyhow::Result<Cert> {
    let (cert_path, key_path) = (dir.join("webui-cert.der"), dir.join("webui-key.der"));
    if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        if cert.len() < 64 * 1024 && key.len() < 64 * 1024 {
            return Ok(Cert {
                fingerprint: fingerprint(&cert),
                chain: vec![CertificateDer::from(cert)],
                key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            });
        }
    }
    let made = rcgen::generate_simple_self_signed(vec!["riptor.local".into(), "localhost".into()])
        .context("making the remote control certificate")?;
    let cert = made.cert.der().to_vec();
    let key = made.signing_key.serialize_der();
    riptor_core::state_file::save_bytes(&key_path, &key)?;
    riptor_core::state_file::save_bytes(&cert_path, &cert)?;
    Ok(Cert {
        fingerprint: fingerprint(&cert),
        chain: vec![CertificateDer::from(cert)],
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    })
}

/// The user's own PEM certificate chain and private key.
pub fn load_pem(cert: &Path, key: &Path) -> anyhow::Result<Cert> {
    use rustls::pki_types::pem::PemObject;
    let read = |p: &Path| -> anyhow::Result<Vec<u8>> {
        let meta = std::fs::metadata(p).with_context(|| format!("reading {}", p.display()))?;
        anyhow::ensure!(meta.len() <= 1024 * 1024, "{} is too large", p.display());
        Ok(std::fs::read(p)?)
    };
    let chain = CertificateDer::pem_slice_iter(&read(cert)?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("{} isn't a PEM certificate: {e:?}", cert.display()))?;
    anyhow::ensure!(!chain.is_empty(), "{} has no certificate", cert.display());
    let key = PrivateKeyDer::from_pem_slice(&read(key)?)
        .map_err(|e| anyhow::anyhow!("{} isn't a PEM private key: {e:?}", key.display()))?;
    Ok(Cert {
        fingerprint: fingerprint(&chain[0]),
        chain,
        key,
    })
}

pub fn tls_acceptor(cert: &Cert) -> anyhow::Result<TlsAcceptor> {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(cert.chain.clone(), cert.key.clone_key())?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub type Io = Either<TcpStream, TlsStream<TcpStream>>;

/// The client's address, for handlers (axum only provides it for its own listener).
#[derive(Clone, Copy, Debug)]
pub struct Client(pub SocketAddr);

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Gate>> for Client {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Gate>) -> Self {
        Client(*stream.remote_addr())
    }
}

/// A listener that only hands axum connections that passed `policy` (and TLS, if on).
pub struct Gate {
    rx: mpsc::Receiver<(Io, SocketAddr)>,
    local: SocketAddr,
    _task: tokio::task::JoinHandle<()>,
}

impl Gate {
    pub fn new(
        listener: TcpListener,
        tls: Option<TlsAcceptor>,
        policy: impl Fn(SocketAddr, SocketAddr) -> Option<&'static str> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let local = listener.local_addr()?;
        let (tx, rx) = mpsc::channel(16);
        let slots = Arc::new(Semaphore::new(MAX_HANDSHAKES));
        let task = tokio::spawn(async move {
            loop {
                let (tcp, remote) = match listener.accept().await {
                    Ok(c) => c,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let Ok(here) = tcp.local_addr() else { continue };
                if let Some(why) = policy(remote, here) {
                    note(
                        remote.ip(),
                        "refused",
                        format_args!("remote control refused {}: {why}", remote.ip()),
                    );
                    continue;
                }
                let Some(acceptor) = tls.clone() else {
                    if tx.send((Either::Left(tcp), remote)).await.is_err() {
                        return;
                    }
                    continue;
                };
                // Busy: turn new connections away rather than queue them.
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    continue;
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
                        Ok(Ok(stream)) => {
                            let _ = tx.send((Either::Right(stream), remote)).await;
                        }
                        // Usually an app that doesn't trust the certificate, or plain http to an https port.
                        Ok(Err(e)) => note(
                            remote.ip(),
                            "tls",
                            format_args!(
                                "remote control: TLS handshake with {} failed: {e}",
                                remote.ip()
                            ),
                        ),
                        Err(_) => note(
                            remote.ip(),
                            "tls",
                            format_args!(
                                "remote control: TLS handshake with {} timed out",
                                remote.ip()
                            ),
                        ),
                    }
                });
            }
        });
        Ok(Self {
            rx,
            local,
            _task: task,
        })
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self._task.abort();
    }
}

impl axum::serve::Listener for Gate {
    type Io = Io;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(c) => c,
            // The accept task only ends when the gate is dropped.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_network_and_vpn_rules() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for lan in [
            "192.168.1.20",
            "10.0.0.5",
            "172.16.3.4",
            "127.0.0.1",
            "169.254.1.1",
            "fe80::1",
            "fd00::5",
            "::1",
            "::ffff:192.168.0.9",
        ] {
            assert!(allowed(ip(lan), ip("192.168.1.2"), false, None), "{lan}");
        }
        for public in ["8.8.8.8", "100.64.0.1", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(
                !allowed(ip(public), ip("192.168.1.2"), false, None),
                "{public}"
            );
            assert!(
                allowed(ip(public), ip("192.168.1.2"), true, None),
                "{public}"
            );
        }
        // Anything arriving on the tunnel's own address is refused, even from a "private" peer.
        let vpn = Some(Ipv4Addr::new(10, 8, 0, 2));
        assert!(!allowed(ip("10.8.0.1"), ip("10.8.0.2"), true, vpn));
        assert!(!allowed(ip("10.8.0.1"), ip("::ffff:10.8.0.2"), true, vpn));
        assert!(allowed(ip("192.168.1.20"), ip("192.168.1.2"), false, vpn));

        // The log says which rule refused it.
        let tunnel = refusal(ip("10.8.0.1"), ip("10.8.0.2"), true, vpn).unwrap();
        assert!(tunnel.contains("VPN tunnel"));
        let outside = refusal(ip("100.64.0.1"), ip("192.168.1.2"), false, None).unwrap();
        assert!(outside.contains("Accept connections from outside"));
    }

    #[test]
    fn the_certificate_is_made_once_and_reused() {
        let d = tempfile::tempdir().unwrap();
        let a = load_or_create_cert(d.path()).unwrap();
        let b = load_or_create_cert(d.path()).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 32 * 3 - 1);
        assert!(tls_acceptor(&a).is_ok());
    }
}
