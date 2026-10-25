//! The IP blocklist download, made the way torrents connect (through the VPN when it's on).

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use tracing::{info, warn};

use crate::network::NetworkMode;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const REFRESH: Duration = Duration::from_secs(24 * 3600);

/// One file per URL, so changing the URL never reuses the old list.
pub fn cache_path(data_dir: &Path, url: &str) -> PathBuf {
    use sha1::Digest;
    let h = sha1::Sha1::digest(url.as_bytes());
    let name: String = h[..8].iter().map(|b| format!("{b:02x}")).collect();
    data_dir.join(format!("blocklist-{name}.cache"))
}

fn fresh(p: &Path) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < REFRESH)
}

/// A local copy of the list; the kill switch never lets it download.
pub async fn fetch(data_dir: &Path, url: &str, net: &NetworkMode) -> Option<PathBuf> {
    let cache = cache_path(data_dir, url);
    let have = cache.is_file();
    if (have && fresh(&cache)) || matches!(net, NetworkMode::Blocked) {
        return have.then_some(cache);
    }
    let local = match net {
        NetworkMode::Vpn(b) => Some(b.end()),
        _ => None,
    };
    let got = download(url, local.as_ref()).await;
    riptor_net::ledger::record(riptor_net::ledger::Purpose::Blocklist, url, got.is_ok());
    match got.and_then(|bytes| crate::state_file::save_bytes(&cache, &bytes)) {
        Ok(()) => {
            info!("blocklist downloaded");
            Some(cache)
        }
        Err(e) => {
            warn!("blocklist download failed: {e:#}");
            have.then_some(cache)
        }
    }
}

async fn download(url: &str, local: Option<&riptor_net::TunnelEnd>) -> anyhow::Result<Vec<u8>> {
    let u = url::Url::parse(url)?;
    anyhow::ensure!(
        u.scheme() == "https",
        "blocklists must be fetched over https"
    );
    let resp = crate::webseed::client(local, false)?
        .get(u)
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        anyhow::ensure!(body.len() <= MAX_BYTES, "blocklist is too large");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_kill_switch_never_downloads() {
        let dir = tempfile::tempdir().unwrap();
        // Unroutable on purpose: reaching the network would hang or fail differently.
        let url = "https://blocklist.invalid/list.gz";
        assert_eq!(fetch(dir.path(), url, &NetworkMode::Blocked).await, None);
        std::fs::write(cache_path(dir.path(), url), b"1.2.3.4-1.2.3.5\n").unwrap();
        assert_eq!(
            fetch(dir.path(), url, &NetworkMode::Blocked).await,
            Some(cache_path(dir.path(), url))
        );
        // A fresh copy is used without asking the network.
        assert_eq!(
            fetch(dir.path(), url, &NetworkMode::Normal).await,
            Some(cache_path(dir.path(), url))
        );
    }

    #[test]
    fn each_url_has_its_own_cache() {
        let d = Path::new("/data");
        assert_ne!(
            cache_path(d, "https://a.example/l"),
            cache_path(d, "https://b.example/l")
        );
        assert!(cache_path(d, "https://a.example/../../x").starts_with(d));
    }

    #[tokio::test]
    async fn plain_http_is_refused() {
        assert!(download("http://example.org/list", None).await.is_err());
    }
}
