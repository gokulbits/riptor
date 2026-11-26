//! Refreshes feeds in the background and tells the window when they change.

use std::sync::Arc;

use riptor_core::Engine;
use riptor_rss::Rss;
use tauri::{AppHandle, Emitter};

pub fn spawn(app: AppHandle, engine: Arc<Engine>, rss: Arc<Rss>) {
    let changed = Arc::new(move || {
        let _ = app.emit("riptor://rss", ());
    });
    riptor_services::rss::spawn(engine, rss, changed);
}

pub async fn refresh(
    app: &AppHandle,
    engine: &Arc<Engine>,
    rss: &Arc<Rss>,
    force: bool,
    only: Option<Vec<u32>>,
) {
    if riptor_services::rss::refresh(engine, rss, force, only).await {
        let _ = app.emit("riptor://rss", ());
    }
}
