//! Torrent input from untrusted sources, size-capped and scheme-checked.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

pub const MAX_TORRENT_BYTES: usize = 16 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Source {
    Magnet(String),
    Url(String),
    File(std::path::PathBuf),
}

pub enum Resolved {
    Magnet(String),
    Bytes(Bytes),
}

pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if matches!(attempt.url().scheme(), "http" | "https") {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent(concat!("Riptor/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

pub fn validate_magnet(m: &str) -> anyhow::Result<()> {
    ensure!(m.len() <= 64 * 1024, "magnet link too long");
    ensure!(m.starts_with("magnet:?"), "not a magnet link");
    let parsed = librqbit::Magnet::parse(m).context("invalid magnet link")?;
    ensure!(
        parsed.as_id20().is_some(),
        "magnet link has no BitTorrent v1 info hash"
    );
    Ok(())
}

pub async fn resolve(src: &Source, client: &reqwest::Client) -> anyhow::Result<Resolved> {
    match src {
        Source::Magnet(m) => {
            let m = m.trim();
            validate_magnet(m)?;
            Ok(Resolved::Magnet(m.to_string()))
        }
        Source::Url(u) => {
            let url = url::Url::parse(u.trim()).context("invalid URL")?;
            if url.scheme() == "magnet" {
                validate_magnet(url.as_str())?;
                return Ok(Resolved::Magnet(url.to_string()));
            }
            ensure!(
                matches!(url.scheme(), "http" | "https"),
                "only http(s) and magnet links are supported"
            );
            Ok(Resolved::Bytes(fetch_capped(client, url).await?))
        }
        Source::File(p) => Ok(Resolved::Bytes(read_torrent_file(p)?)),
    }
}

async fn fetch_capped(client: &reqwest::Client, url: url::Url) -> anyhow::Result<Bytes> {
    let resp = client.get(url).send().await?.error_for_status()?;
    if let Some(len) = resp.content_length() {
        ensure!(
            len as usize <= MAX_TORRENT_BYTES,
            "torrent file is too large ({len} bytes)"
        );
    }
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            body.len() + chunk.len() <= MAX_TORRENT_BYTES,
            "torrent file is too large"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body.into())
}

pub fn read_torrent_file(p: &Path) -> anyhow::Result<Bytes> {
    let ext_ok = p
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("torrent"));
    ensure!(ext_ok, "only .torrent files can be opened");
    let meta = std::fs::metadata(p).with_context(|| format!("opening {p:?}"))?;
    ensure!(meta.is_file(), "{p:?} is not a file");
    if meta.len() as usize > MAX_TORRENT_BYTES {
        bail!("torrent file is too large ({} bytes)", meta.len());
    }
    Ok(std::fs::read(p)?.into())
}

const EXECUTABLE_EXTS: &[&str] = &[
    "exe", "scr", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh",
    "msi", "msp", "lnk", "pif", "cpl", "hta", "jar", "reg", "dll", "sys", "appimage", "sh", "run",
    "desktop", "deb", "rpm", "apk",
];

pub fn is_executable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((_, ext)) => EXECUTABLE_EXTS.contains(&ext),
        None => false,
    }
}

/// Types Windows would run or mount rather than just show.
const NEVER_OPEN_EXTS: &[&str] = &[
    "url",
    "msc",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "application",
    "appref-ms",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
    "xll",
    "ws",
    "gadget",
    "inf",
    "scf",
    "chm",
    "docm",
    "xlsm",
    "pptm",
];

/// Files the user may open with their default app from inside Riptor.
pub fn safe_to_open(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((_, ext)) => !EXECUTABLE_EXTS.contains(&ext) && !NEVER_OPEN_EXTS.contains(&ext),
        None => false,
    }
}

pub fn magnet_link(info_hash: &str, name: &str, trackers: &[String]) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let mut m = format!("magnet:?xt=urn:btih:{info_hash}");
    if !name.is_empty() {
        m.push_str("&dn=");
        m.push_str(&enc(name));
    }
    for t in trackers {
        m.push_str("&tr=");
        m.push_str(&enc(t));
    }
    m
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_harmless_files_open() {
        for ok in ["movie.MKV", "a/b/song.flac", "book.pdf", "notes.txt"] {
            assert!(super::safe_to_open(ok), "{ok}");
        }
        for bad in [
            "setup.exe",
            "x.Lnk",
            "disk.iso",
            "site.url",
            "macro.docm",
            "run.ps1",
            "README",
            "evil.pdf.scr",
        ] {
            assert!(!super::safe_to_open(bad), "{bad}");
        }
    }

    #[test]
    fn magnet_links_round_trip() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let m = super::magnet_link(
            hash,
            "Tom & Jerry?.mkv",
            &["udp://tracker.example:1337/announce".into()],
        );
        assert_eq!(
            m,
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567\
             &dn=Tom+%26+Jerry%3F.mkv&tr=udp%3A%2F%2Ftracker.example%3A1337%2Fannounce"
        );
        assert!(super::validate_magnet(&m).is_ok());
        assert_eq!(
            super::magnet_link(hash, "", &[]),
            format!("magnet:?xt=urn:btih:{hash}")
        );
    }

    use super::*;

    #[test]
    fn magnets() {
        assert!(
            validate_magnet("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567").is_ok()
        );
        assert!(validate_magnet("magnet:?dn=no-hash").is_err());
        assert!(validate_magnet("http://example.com/x.torrent").is_err());
        assert!(validate_magnet(&format!("magnet:?xt=urn:btih:{}", "a".repeat(70000))).is_err());
    }

    #[test]
    fn files_need_torrent_extension_and_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"password").unwrap();
        assert!(read_torrent_file(&secret).is_err());
        let big = dir.path().join("big.torrent");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_TORRENT_BYTES as u64 + 1).unwrap();
        assert!(read_torrent_file(&big).is_err());
        let ok = dir.path().join("ok.TORRENT");
        std::fs::write(&ok, b"d4:infod4:name1:xee").unwrap();
        assert!(read_torrent_file(&ok).is_ok());
        assert!(read_torrent_file(dir.path()).is_err());
    }

    #[tokio::test]
    async fn url_schemes() {
        let c = http_client().unwrap();
        for bad in [
            "file:///etc/passwd",
            "ftp://x/y.torrent",
            "javascript:alert(1)",
        ] {
            assert!(
                resolve(&Source::Url(bad.into()), &c).await.is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn executables() {
        assert!(is_executable_name("Setup.EXE"));
        assert!(is_executable_name("movie.mkv.scr"));
        assert!(is_executable_name("tool.AppImage"));
        assert!(!is_executable_name("movie.mkv"));
        assert!(!is_executable_name("README"));
    }
}
