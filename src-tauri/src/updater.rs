//! In-app updates on Windows; Linux packages update through the package manager.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use parking_lot::Mutex;
    use tauri::{AppHandle, Emitter, Manager};
    use tauri_plugin_updater::{Update, UpdaterExt};

    #[derive(Clone, Debug, Serialize)]
    pub struct Progress {
        pub done: u64,
        pub total: Option<u64>,
    }

    /// The update found by the last check, and its package once downloaded (signature checked).
    #[derive(Default)]
    pub struct Pending {
        found: Mutex<Option<Update>>,
        ready: Mutex<Option<(Update, Vec<u8>)>>,
        downloading: std::sync::atomic::AtomicBool,
    }

    fn info(u: &Update) -> UpdateInfo {
        UpdateInfo {
            version: clean(&u.version, 40),
            notes: u.body.as_deref().map(|b| clean(b, 4000)),
            date: u.date.map(|d| d.to_string()),
        }
    }

    fn clean(s: &str, max: usize) -> String {
        s.chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(max)
            .collect()
    }

    pub async fn check(app: &AppHandle) -> anyhow::Result<Option<UpdateInfo>> {
        // The updater plugin has its own client, so this never goes through the VPN.
        let found = app.updater()?.check().await;
        riptor_net::ledger::record_via(
            riptor_net::ledger::Purpose::Update,
            "github.com",
            riptor_net::ledger::Route::Direct,
            found.is_ok(),
        );
        let found = found.map_err(|e| {
            anyhow::anyhow!("couldn't get update information from GitHub ({e}); you may be offline")
        })?;
        let pending = app.state::<Pending>();
        let found_info = found.as_ref().map(info);
        // A downloaded package for another version is stale now.
        let mut ready = pending.ready.lock();
        if ready.as_ref().map(|(u, _)| &u.version) != found.as_ref().map(|u| &u.version) {
            *ready = None;
        }
        drop(ready);
        *pending.found.lock() = found;
        Ok(found_info)
    }

    async fn fetch(app: &AppHandle, update: &Update) -> anyhow::Result<Vec<u8>> {
        let mut done = 0u64;
        let bytes = update
            .download(
                |chunk, total| {
                    done += chunk as u64;
                    let _ = app.emit("riptor://update-progress", Progress { done, total });
                },
                || {},
            )
            .await?;
        Ok(bytes)
    }

    /// Downloads the found update in the background and keeps it until it's installed.
    pub async fn download(app: &AppHandle) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        let pending = app.state::<Pending>();
        if pending.ready.lock().is_some() {
            return Ok(());
        }
        let update = pending
            .found
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("check for updates first"))?;
        if pending.downloading.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let result = fetch(app, &update).await;
        pending.downloading.store(false, Ordering::Release);
        let bytes = result?;
        let ready = info(&update);
        *pending.ready.lock() = Some((update, bytes));
        let _ = app.emit("riptor://update-ready", ready);
        Ok(())
    }

    /// Uses the downloaded package (or downloads it now), shuts Riptor down cleanly, then runs
    /// the installer.
    pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
        let pending = app.state::<Pending>();
        let ready = pending.ready.lock().take();
        let (update, bytes) = match ready {
            Some(r) => r,
            None => {
                let update = pending
                    .found
                    .lock()
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("check for updates first"))?;
                let bytes = fetch(app, &update).await?;
                (update, bytes)
            }
        };
        crate::shut_down(app).await;
        // Verified against the bundled public key again; the installer then replaces Riptor.
        update.install(bytes)?;
        Ok(())
    }

    pub fn init(app: &AppHandle) {
        app.manage(Pending::default());
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use tauri::AppHandle;

    pub async fn check(_: &AppHandle) -> anyhow::Result<Option<UpdateInfo>> {
        anyhow::bail!("updates come from your package manager on Linux")
    }

    pub async fn download(_: &AppHandle) -> anyhow::Result<()> {
        anyhow::bail!("updates come from your package manager on Linux")
    }

    pub async fn install(_: &AppHandle) -> anyhow::Result<()> {
        anyhow::bail!("updates come from your package manager on Linux")
    }

    pub fn init(_: &AppHandle) {}
}

pub use imp::{check, download, init, install};

pub const SUPPORTED: bool = cfg!(windows);
