//! python.org's embeddable Python, pinned by SHA-256. The Windows setup ships
//! it; dev builds can download it into `<data>/search/python`.
//!
//! To update, take the hashes from python.org's release API
//! (`/api/v2/downloads/release_file/?release=<id>`) and check a fresh download.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{ensure, Context};
use futures::StreamExt;
use sha2::{Digest, Sha256};

pub const VERSION: &str = "3.14.8";

pub struct Release {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub const RELEASE: Option<Release> = Some(Release {
    url: "https://www.python.org/ftp/python/3.14.8/python-3.14.8-embed-amd64.zip",
    sha256: "a93abe456ab01bd96d7a085b3cdb6566b3063f4241360d114142fbdb07f0a310",
    size: 12_601_679,
});
#[cfg(all(windows, target_arch = "aarch64"))]
pub const RELEASE: Option<Release> = Some(Release {
    url: "https://www.python.org/ftp/python/3.14.8/python-3.14.8-embed-arm64.zip",
    sha256: "155be84ccb57c6331cf0e39001c78a1dfac3be62f403f3ff5f2e29b80dda7ebe",
    size: 11_923_092,
});
#[cfg(not(all(windows, any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub const RELEASE: Option<Release> = None;

const MAX_ZIP_BYTES: usize = 32 * 1024 * 1024;
const MAX_UNPACKED: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 500;

pub fn dir(search_dir: &Path) -> PathBuf {
    search_dir.join("python")
}

pub fn exe(search_dir: &Path) -> PathBuf {
    dir(search_dir).join(if cfg!(windows) {
        "python.exe"
    } else {
        "bin/python3"
    })
}

pub fn installed(search_dir: &Path) -> bool {
    exe(search_dir).is_file()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn install(search_dir: &Path) -> anyhow::Result<()> {
    let rel = RELEASE
        .as_ref()
        .context("Riptor can only set up Python by itself on Windows")?;
    // The shared client's 30 s timeout is too short for 12 MB on a slow line.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5 || a.url().scheme() != "https" {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .user_agent(concat!("Riptor/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let work = async {
        let resp = http.get(rel.url).send().await?.error_for_status()?;
        ensure!(resp.url().scheme() == "https", "download left https");
        let mut body = Vec::with_capacity(rel.size as usize);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            ensure!(
                body.len() + chunk.len() <= MAX_ZIP_BYTES,
                "download is too large"
            );
            body.extend_from_slice(&chunk);
        }
        anyhow::Ok(body)
    };
    let body = tokio::time::timeout(Duration::from_secs(300), work)
        .await
        .context("downloading Python timed out")
        .and_then(|r| r);
    riptor_net::ledger::record(riptor_net::ledger::Purpose::Python, rel.url, body.is_ok());
    let body = body?;
    let got = hex(&Sha256::digest(&body));
    ensure!(
        got == rel.sha256,
        "the Python download did not match its expected SHA-256 fingerprint; nothing was installed"
    );
    let search_dir = search_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let tmp = search_dir.join("python.tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        extract(&body, &tmp)?;
        let dest = dir(&search_dir);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .context("could not replace the old Python (is a search running?)")?;
        }
        std::fs::rename(&tmp, &dest)?;
        anyhow::Ok(())
    })
    .await?
}

pub fn remove(search_dir: &Path) -> anyhow::Result<()> {
    let d = dir(search_dir);
    if d.exists() {
        std::fs::remove_dir_all(&d)
            .context("could not remove Riptor's Python (is a search running?)")?;
    }
    Ok(())
}

/// The hash already vouches for the archive; this is belt and braces.
pub fn extract(zip_bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    ensure!(zip.len() <= MAX_FILES, "archive has too many files");
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let rel = f
            .enclosed_name()
            .with_context(|| format!("unsafe path in archive: {:?}", f.name()))?;
        ensure!(!f.is_symlink(), "archive contains a link");
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        let left = MAX_UNPACKED - total;
        let n = std::io::copy(&mut (&mut f).take(left + 1), &mut w)?;
        total += n;
        ensure!(total <= MAX_UNPACKED, "archive unpacks to too much data");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_safely() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("py");
        extract(&zip_of(&[("python.exe", b"MZ"), ("Lib/x.py", b"x")]), &dest).unwrap();
        assert!(dest.join("python.exe").is_file());
        assert!(dest.join("Lib/x.py").is_file());

        let evil = d.path().join("evil");
        for name in ["../escape.txt", "a/../../escape.txt", "..\\escape.txt"] {
            assert!(extract(&zip_of(&[(name, b"x")]), &evil).is_err(), "{name}");
        }
        assert!(!d.path().join("escape.txt").exists());
        assert!(extract(b"not a zip", &evil).is_err());
    }

    #[test]
    fn pinned_release_looks_right() {
        if let Some(r) = RELEASE {
            assert!(r.url.starts_with("https://www.python.org/ftp/python/"));
            assert!(r.url.contains(VERSION));
            assert_eq!(r.sha256.len(), 64);
            assert!(r.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}
