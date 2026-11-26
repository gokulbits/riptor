//! Refreshes feeds in the background and adds what the rules pick.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::ensure;
use riptor_core::{AddParams, Engine, NoticeLevel, Source};
use riptor_rss::{feed::MAX_FEED_BYTES, Download, Rss};
use tracing::warn;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Refreshes due feeds once a minute; `changed` runs after each refresh that did something.
pub fn spawn(engine: Arc<Engine>, rss: Arc<Rss>, changed: Arc<dyn Fn() + Send + Sync>) {
    tokio::spawn(async move {
        loop {
            if refresh(&engine, &rss, false, None).await {
                changed();
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

/// Returns whether any feed was fetched.
pub async fn refresh(
    engine: &Arc<Engine>,
    rss: &Arc<Rss>,
    force: bool,
    only: Option<Vec<u32>>,
) -> bool {
    let due = rss.due(now(), force, only.as_deref());
    if due.is_empty() {
        return false;
    }
    for (id, url) in due {
        let fetched = fetch(engine, &url).await;
        match rss.apply(id, now(), fetched) {
            Ok(picked) => {
                for d in picked {
                    let engine = engine.clone();
                    tokio::spawn(async move { add(&engine, d).await });
                }
            }
            Err(e) => warn!("saving feed {id} failed: {e:#}"),
        }
    }
    true
}

/// Goes through the VPN when it's on, like every other request.
async fn fetch(engine: &Engine, url: &str) -> anyhow::Result<Vec<u8>> {
    let got = fetch_once(engine, url).await;
    riptor_net::ledger::record(riptor_net::ledger::Purpose::Feed, url, got.is_ok());
    got
}

async fn fetch_once(engine: &Engine, url: &str) -> anyhow::Result<Vec<u8>> {
    let client = engine.web_client(url)?;
    let mut resp = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?;
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        ensure!(out.len() <= MAX_FEED_BYTES, "feed is too large");
    }
    Ok(out)
}

/// Adds one article's torrent the way a rule asks; also used for a manual download.
pub async fn add_article(
    engine: &Engine,
    torrent: &str,
    rule: Option<&riptor_rss::Rule>,
) -> anyhow::Result<Option<bool>> {
    let src = if torrent.starts_with("magnet:") {
        Source::Magnet(torrent.to_string())
    } else {
        Source::Url(torrent.to_string())
    };
    let preview = engine.preview(src).await?;
    if preview.already_added {
        return Ok(None);
    }
    engine
        .add(AddParams {
            info_hash: preview.info_hash,
            save_path: rule.and_then(|r| r.save_path.clone()),
            category: rule.and_then(|r| r.category.clone()),
            tags: Vec::new(),
            // Programs never start on their own.
            start_paused: rule.is_some_and(|r| r.start_paused)
                || preview.has_executables
                || engine.settings().add_stopped,
            only_files: None,
            create_subfolder: None,
            allow_suspicious: false,
            reuse_existing: true,
        })
        .await?;
    Ok(Some(preview.has_executables))
}

async fn add(engine: &Engine, d: Download) {
    match add_article(engine, &d.torrent, Some(&d.rule)).await {
        Ok(None) => {}
        Ok(Some(true)) => engine.notice(
            NoticeLevel::Warning,
            format!("RSS added \"{}\" stopped: it contains programs", d.title),
        ),
        Ok(Some(false)) => engine.notice(NoticeLevel::Info, format!("RSS added \"{}\"", d.title)),
        Err(e) => {
            warn!(
                "RSS rule \"{}\" couldn't add {}: {e:#}",
                d.rule.name, d.title
            );
            engine.notice(
                NoticeLevel::Warning,
                format!("RSS couldn't add \"{}\": {e:#}", d.title),
            )
        }
    }
}
