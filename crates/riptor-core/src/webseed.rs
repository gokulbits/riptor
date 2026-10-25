//! Web seeds (BEP 19). The URLs come from the torrent, so treat them as hostile.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use futures::StreamExt;
use librqbit::limits::Limits;
use librqbit::{ByteBufOwned, TorrentStateLive, ValidatedTorrentMetaV1Info};
use librqbit_bencode::BencodeValue;
use url::Url;

/// Servers used per torrent.
pub const MAX_SEEDS: usize = 4;
/// Each worker holds a whole piece in memory.
pub const MAX_PIECE: u32 = 16 * 1024 * 1024;
/// Pieces fetched at once from each server.
const PER_SEED: usize = 2;
const MAX_URL_LEN: usize = 2048;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// A server is dropped after this many failed requests in a row...
const MAX_FAILURES: u32 = 8;
/// ...or this many pieces that fail the hash check.
const MAX_BAD_PIECES: u32 = 4;

/// The usable `url-list` entries of a .torrent, deduplicated.
pub fn url_list(torrent: &[u8]) -> Vec<Url> {
    let Ok(BencodeValue::Dict(top)) = librqbit_bencode::dyn_from_bytes::<Vec<u8>>(torrent) else {
        return Vec::new();
    };
    let raw: Vec<&Vec<u8>> = match top.get(&b"url-list"[..]) {
        Some(BencodeValue::Bytes(b)) => vec![b],
        Some(BencodeValue::List(l)) => l
            .iter()
            .filter_map(|v| match v {
                BencodeValue::Bytes(b) => Some(b),
                _ => None,
            })
            .collect(),
        _ => return Vec::new(),
    };
    let mut out: Vec<Url> = Vec::new();
    for b in raw {
        if out.len() == MAX_SEEDS {
            break;
        }
        if b.len() > MAX_URL_LEN {
            continue;
        }
        let Some(u) = std::str::from_utf8(b).ok().and_then(|s| Url::parse(s).ok()) else {
            continue;
        };
        let ok = matches!(u.scheme(), "http" | "https")
            && u.host().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none();
        if ok && !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

struct WebFile {
    /// `None` for padding files (all zeros).
    path: Option<Vec<Vec<u8>>>,
    offset: u64,
    len: u64,
}

/// Maps pieces to HTTP ranges.
pub struct Layout {
    name: Vec<u8>,
    multi_file: bool,
    files: Vec<WebFile>,
}

impl Layout {
    /// `None` when a file name can't be turned into a safe URL path.
    pub fn new(info: &ValidatedTorrentMetaV1Info<ByteBufOwned>, multi_file: bool) -> Option<Self> {
        let name = info.info().name.as_ref()?.as_ref().to_vec();
        if !component_ok(&name) {
            return None;
        }
        let mut files = Vec::new();
        let mut offset = 0u64;
        for d in info.iter_file_details() {
            let path = if d.attrs().padding {
                None
            } else {
                let parts: Vec<Vec<u8>> = d
                    .filename
                    .iter_components_bytes()
                    .map(<[u8]>::to_vec)
                    .collect();
                if !parts.iter().all(|p| component_ok(p)) {
                    return None;
                }
                Some(parts)
            };
            files.push(WebFile {
                path,
                offset,
                len: d.len,
            });
            offset = offset.checked_add(d.len)?;
        }
        Some(Self {
            name,
            multi_file,
            files,
        })
    }

    fn file_url(&self, base: &Url, f: &WebFile) -> Option<Url> {
        let path = f.path.as_ref()?;
        let mut s = base.as_str().to_string();
        if self.multi_file {
            if base.query().is_some() {
                return None;
            }
            if !s.ends_with('/') {
                s.push('/');
            }
            s.push_str(&encode(&self.name));
            for p in path {
                s.push('/');
                s.push_str(&encode(p));
            }
        } else if s.ends_with('/') && base.query().is_none() {
            s.push_str(&encode(&self.name));
        }
        let u = Url::parse(&s).ok()?;
        (u.host() == base.host() && u.scheme() == base.scheme()).then_some(u)
    }

    /// (file, start in file, length)
    fn spans(&self, offset: u64, len: u64) -> Vec<(usize, u64, u64)> {
        let end = offset + len;
        self.files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.len > 0 && f.offset < end && f.offset + f.len > offset)
            .map(|(i, f)| {
                let from = offset.max(f.offset);
                let to = end.min(f.offset + f.len);
                (i, from - f.offset, to - from)
            })
            .collect()
    }
}

/// URL parsers fold `..` (even `%2E%2E`), which would change the path.
fn component_ok(c: &[u8]) -> bool {
    !c.is_empty() && c != b"." && c != b".."
}

fn encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Checks addresses after DNS, so a name pointing at the LAN is refused too.
struct PublicOnly {
    allow_local: bool,
    tunnel: Option<crate::dns::TunnelResolver>,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow = self.allow_local;
        let tunnel = self.tunnel.clone();
        Box::pin(async move {
            let found: Vec<SocketAddr> = match &tunnel {
                Some(t) => t.lookup(name.as_str(), 0).await?,
                None => tokio::net::lookup_host((name.as_str(), 0)).await?.collect(),
            };
            let addrs: Vec<SocketAddr> = found
                .into_iter()
                .filter(|a| allow || riptor_net::is_public(a.ip()))
                .filter(|a| tunnel.is_none() || a.is_ipv4())
                .collect();
            if addrs.is_empty() {
                return Err("refusing to contact a local or private address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// `local` is the VPN tunnel when the VPN is on.
pub fn client(
    local: Option<&riptor_net::TunnelEnd>,
    allow_local: bool,
) -> anyhow::Result<reqwest::Client> {
    let tunnel = local
        .map(|end| crate::dns::TunnelResolver::new(end, &crate::dns::SERVERS))
        .transpose()?;
    let mut b = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("Riptor/", env!("CARGO_PKG_VERSION")))
        .dns_resolver(Arc::new(PublicOnly {
            allow_local,
            tunnel,
        }))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() >= 5 {
                a.error("too many redirects")
            } else if !matches!(a.url().scheme(), "http" | "https")
                || !(allow_local || riptor_net::host_allowed(a.url()))
            {
                a.stop()
            } else {
                a.follow()
            }
        }));
    if let Some(end) = local {
        b = end.http(b);
    }
    Ok(b.build()?)
}

/// Waits for `n` bytes of download allowance from every limiter.
async fn throttle(limits: &[&Limits], n: usize) {
    for l in limits {
        let Some(bps) = l.get_download_bps() else {
            continue;
        };
        let step = (bps.get() as usize).clamp(1, 16 * 1024);
        let mut left = n;
        while left > 0 {
            let take = left.min(step);
            let _ = l
                .prepare_for_download(NonZeroU32::new(take as u32).unwrap_or(NonZeroU32::MIN))
                .await;
            left -= take;
        }
    }
}

async fn fetch_range(
    http: &reqwest::Client,
    url: Url,
    start: u64,
    len: u64,
    limits: &[&Limits],
    out: &mut Vec<u8>,
) -> anyhow::Result<()> {
    let last = start + len - 1;
    let resp = http
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{last}"))
        .send()
        .await?;
    match resp.status().as_u16() {
        206 => {
            let cr = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .context("no Content-Range")?;
            ensure!(
                cr.starts_with(&format!("bytes {start}-{last}/")),
                "server sent a different range"
            );
        }
        // Range ignored: only usable when the range starts at 0.
        200 if start == 0 => {}
        s => bail!("HTTP {s}"),
    }
    let want = out.len() + len as usize;
    let mut body = resp.bytes_stream();
    while out.len() < want {
        let chunk = body.next().await.context("server closed early")??;
        let take = chunk.len().min(want - out.len());
        throttle(limits, take).await;
        out.extend_from_slice(&chunk[..take]);
    }
    Ok(())
}

impl Layout {
    async fn fetch_piece(
        &self,
        http: &reqwest::Client,
        base: &Url,
        allow_local: bool,
        offset: u64,
        len: u32,
        limits: &[&Limits],
    ) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len as usize);
        for (i, start, n) in self.spans(offset, len as u64) {
            let f = &self.files[i];
            match &f.path {
                None => out.resize(out.len() + n as usize, 0),
                Some(_) => {
                    let url = self.file_url(base, f).context("no URL for this file")?;
                    // IP literals never reach the resolver's check.
                    ensure!(
                        allow_local || riptor_net::host_allowed(&url),
                        "refusing to contact a local or private address"
                    );
                    let host = url.host_str().unwrap_or("").to_string();
                    let got = fetch_range(http, url, start, n, limits, &mut out).await;
                    riptor_net::ledger::record(
                        riptor_net::ledger::Purpose::WebSeed,
                        &host,
                        got.is_ok(),
                    );
                    got?;
                }
            }
        }
        ensure!(out.len() == len as usize, "piece came back short");
        Ok(out)
    }
}

/// Releases reserved pieces even if the task is aborted.
struct Release {
    live: Arc<TorrentStateLive>,
    handle: SocketAddr,
}

impl Drop for Release {
    fn drop(&mut self) {
        self.live.webseed_release(self.handle);
    }
}

/// Pseudo-peer addresses for web seeds. 0.0.0.0 is never a real peer.
fn handle(n: usize) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 1 + n as u16)
}

#[derive(Clone)]
pub struct Job {
    pub live: Arc<TorrentStateLive>,
    pub layout: Arc<Layout>,
    pub http: reqwest::Client,
    pub session: Arc<librqbit::Session>,
    /// Tests only: lets web seeds on 127.0.0.1 through.
    pub allow_local: bool,
}

async fn worker(job: Job, base: Url, n: usize) {
    let me = handle(n);
    let _release = Release {
        live: job.live.clone(),
        handle: me,
    };
    let (mut failures, mut bad) = (0u32, 0u32);
    loop {
        if job.live.webseed_done() {
            return;
        }
        let Some((piece, offset, len)) = job.live.webseed_acquire(me) else {
            // Everything left is with peers for now.
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        };
        let limits = [job.live.ratelimits(), &job.session.ratelimits];
        let fetched = job
            .layout
            .fetch_piece(&job.http, &base, job.allow_local, offset, len, &limits)
            .await;
        let data = match fetched {
            Ok(d) => d,
            Err(e) => {
                job.live.webseed_release(me);
                failures += 1;
                tracing::debug!("web seed {base}: {e:#}");
                if failures >= MAX_FAILURES {
                    tracing::info!("giving up on web seed {base} after {failures} failures");
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5u64 << failures.min(5))).await;
                continue;
            }
        };
        failures = 0;
        let live = job.live.clone();
        let ok = tokio::task::spawn_blocking(move || live.webseed_piece(me, piece, &data)).await;
        match ok {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                bad += 1;
                job.live.webseed_release(me);
                if bad >= MAX_BAD_PIECES {
                    tracing::info!("dropping web seed {base}: it sent bad data");
                    return;
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("web seed {base}: {e:#}");
                return;
            }
            Err(_) => return,
        }
    }
}

/// The running workers of one torrent.
pub struct Running {
    /// A resumed torrent gets a new live state.
    live: usize,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

pub fn start(job: Job, seeds: &[Url]) -> Running {
    let live = Arc::as_ptr(&job.live) as usize;
    let mut tasks = Vec::new();
    for (i, base) in seeds.iter().take(MAX_SEEDS).enumerate() {
        for k in 0..PER_SEED {
            tasks.push(tokio::spawn(worker(
                job.clone(),
                base.clone(),
                i * PER_SEED + k,
            )));
        }
    }
    Running { live, tasks }
}

impl Running {
    pub fn is_for(&self, live: &Arc<TorrentStateLive>) -> bool {
        self.live == Arc::as_ptr(live) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bencode_torrent(url_list: &str) -> Vec<u8> {
        format!("d8:url-list{url_list}4:infod4:name1:xee").into_bytes()
    }

    #[test]
    fn url_list_accepts_a_string_or_a_list() {
        let one = url_list(&bencode_torrent("22:http://example.org/a/b"));
        assert_eq!(one.len(), 1);
        let many = url_list(&bencode_torrent(
            "l20:https://a.example/x/20:https://a.example/x/12:ftp://h/file14:javascript:x()18:http://u:p@h.org/xe",
        ));
        assert_eq!(many, vec![Url::parse("https://a.example/x/").unwrap()]);
        assert!(url_list(b"not bencode").is_empty());
        assert!(url_list(&bencode_torrent("i5e")).is_empty());
    }

    #[test]
    fn url_list_is_capped() {
        let mut l = String::from("l");
        for i in 0..10 {
            let u = format!("http://h{i}.example/");
            l.push_str(&format!("{}:{u}", u.len()));
        }
        l.push('e');
        assert_eq!(url_list(&bencode_torrent(&l)).len(), MAX_SEEDS);
    }

    fn layout(multi: bool, files: &[(&[&str], u64, bool)]) -> Layout {
        let mut offset = 0;
        Layout {
            name: b"My Album".to_vec(),
            multi_file: multi,
            files: files
                .iter()
                .map(|(p, len, pad)| {
                    let f = WebFile {
                        path: (!pad).then(|| p.iter().map(|c| c.as_bytes().to_vec()).collect()),
                        offset,
                        len: *len,
                    };
                    offset += len;
                    f
                })
                .collect(),
        }
    }

    #[test]
    fn urls_follow_bep19() {
        let m = layout(true, &[(&["CD 1", "01 #1?.flac"], 10, false)]);
        let base = Url::parse("https://archive.example/download/item").unwrap();
        assert_eq!(
            m.file_url(&base, &m.files[0]).unwrap().as_str(),
            "https://archive.example/download/item/My%20Album/CD%201/01%20%231%3F.flac"
        );
        let s = layout(false, &[(&["My Album"], 10, false)]);
        let dir = Url::parse("http://h.example/files/").unwrap();
        assert_eq!(
            s.file_url(&dir, &s.files[0]).unwrap().as_str(),
            "http://h.example/files/My%20Album"
        );
        let exact = Url::parse("http://h.example/get?id=7").unwrap();
        assert_eq!(s.file_url(&exact, &s.files[0]).unwrap(), exact);
    }

    #[test]
    fn hostile_names_never_leave_the_folder() {
        for bad in ["..", ".", ""] {
            assert!(!component_ok(bad.as_bytes()));
        }
        // Slashes and backslashes are encoded, so they can't add segments.
        let m = layout(true, &[(&["a/../../etc", "x\\..\\y"], 1, false)]);
        let base = Url::parse("https://h.example/d/").unwrap();
        let u = m.file_url(&base, &m.files[0]).unwrap();
        assert!(u.path().starts_with("/d/My%20Album/"), "{u}");
        assert_eq!(u.path_segments().unwrap().count(), 4);
    }

    #[test]
    fn spans_cover_pieces_across_files_and_padding() {
        let m = layout(
            true,
            &[
                (&["a"], 5, false),
                (&[".pad", "3"], 3, true),
                (&["b"], 8, false),
            ],
        );
        assert_eq!(m.spans(0, 4), vec![(0, 0, 4)]);
        assert_eq!(m.spans(4, 8), vec![(0, 4, 1), (1, 0, 3), (2, 0, 4)]);
        assert_eq!(m.spans(12, 4), vec![(2, 4, 4)]);
    }

    #[test]
    fn handles_are_never_real_peers() {
        for n in 0..MAX_SEEDS * PER_SEED {
            assert!(handle(n).ip().is_unspecified());
            assert_ne!(handle(n).port(), 0);
        }
    }
}
