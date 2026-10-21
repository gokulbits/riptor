//! Refreshes feeds in the background and adds what the rules pick.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::ensure;
use riptor_core::{AddParams, Engine, Event, NoticeLevel, Source};
use riptor_rss::{feed::MAX_FEED_BYTES, Download, Rss};
use tauri::{AppHandle, Emitter};
use tracing::warn;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn spawn(app: AppHandle, engine: Arc<Engine>, rss: Arc<Rss>) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&app, &engine, &rss, false, None).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

pub async fn refresh(
    app: &AppHandle,
    engine: &Arc<Engine>,
    rss: &Arc<Rss>,
    force: bool,
    only: Option<Vec<u32>>,
) {
    let due = rss.due(now(), force, only.as_deref());
    if due.is_empty() {
        return;
    }
    for (id, url) in due {
        let fetched = fetch(engine, &url).await;
        match rss.apply(id, now(), fetched) {
            Ok(picked) => {
                for d in picked {
                    let (app, engine) = (app.clone(), engine.clone());
                    tauri::async_runtime::spawn(async move { add(&app, &engine, d).await });
                }
            }
            Err(e) => warn!("saving feed {id} failed: {e:#}"),
        }
    }
    let _ = app.emit("riptor://rss", ());
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

fn notice(app: &AppHandle, level: NoticeLevel, message: String) {
    let _ = app.emit("riptor://event", Event::Notice { level, message });
}

async fn add(app: &AppHandle, engine: &Engine, d: Download) {
    let src = if d.torrent.starts_with("magnet:") {
        Source::Magnet(d.torrent.clone())
    } else {
        Source::Url(d.torrent.clone())
    };
    let result = async {
        let preview = engine.preview(src).await?;
        if preview.already_added {
            return Ok(None);
        }
        engine
            .add(AddParams {
                info_hash: preview.info_hash,
                save_path: d.rule.save_path.clone(),
                category: d.rule.category.clone(),
                tags: Vec::new(),
                // Programs never start on their own.
                start_paused: d.rule.start_paused
                    || preview.has_executables
                    || engine.settings().add_stopped,
                only_files: None,
                create_subfolder: None,
                allow_suspicious: false,
                reuse_existing: true,
            })
            .await?;
        anyhow::Ok(Some(preview.has_executables))
    }
    .await;
    match result {
        Ok(None) => {}
        Ok(Some(true)) => notice(
            app,
            NoticeLevel::Warning,
            format!("RSS added \"{}\" stopped: it contains programs", d.title),
        ),
        Ok(Some(false)) => notice(app, NoticeLevel::Info, format!("RSS added \"{}\"", d.title)),
        Err(e) => {
            warn!(
                "RSS rule \"{}\" couldn't add {}: {e:#}",
                d.rule.name, d.title
            );
            notice(
                app,
                NoticeLevel::Warning,
                format!("RSS couldn't add \"{}\": {e:#}", d.title),
            )
        }
    }
}
