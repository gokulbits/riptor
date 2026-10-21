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

    /// The update found by the last check, kept for install.
    #[derive(Default)]
    pub struct Pending(Mutex<Option<Update>>);

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
        let info = found.as_ref().map(|u| UpdateInfo {
            version: clean(&u.version, 40),
            notes: u.body.as_deref().map(|b| clean(b, 4000)),
            date: u.date.map(|d| d.to_string()),
        });
        *app.state::<Pending>().0.lock() = found;
        Ok(info)
    }

    /// Downloads, shuts Riptor down cleanly, then hands over to the installer.
    pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
        let update = app
            .state::<Pending>()
            .0
            .lock()
            .take()
            .ok_or_else(|| anyhow::anyhow!("check for updates first"))?;
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
        crate::shut_down(app).await;
        // Verified against the bundled public key; the installer then replaces Riptor.
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

    pub async fn install(_: &AppHandle) -> anyhow::Result<()> {
        anyhow::bail!("updates come from your package manager on Linux")
    }

    pub fn init(_: &AppHandle) {}
}

pub use imp::{check, init, install};

pub const SUPPORTED: bool = cfg!(windows);
