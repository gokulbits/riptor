//! Sleep or shut down once every download has finished. Resets after it fires.

use std::time::Duration;

use parking_lot::Mutex;
use riptor_core::{TorrentState, TorrentSummary};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

pub const COUNTDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    #[default]
    Nothing,
    Sleep,
    Shutdown,
}

#[derive(Default)]
pub struct WhenDone {
    action: Mutex<Action>,
    /// Something was downloading since the action was chosen.
    armed: Mutex<bool>,
    countdown: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Clone, Serialize)]
struct Countdown {
    action: Action,
    seconds: u64,
}

fn downloading(t: &TorrentSummary) -> bool {
    t.progress < 1.0
        && matches!(
            t.state,
            TorrentState::Downloading
                | TorrentState::StalledDl
                | TorrentState::Metadata
                | TorrentState::Checking
                | TorrentState::Queued
                | TorrentState::NoSpace
        )
}

impl WhenDone {
    pub fn get(&self) -> Action {
        *self.action.lock()
    }

    pub fn set(&self, a: Action) {
        *self.action.lock() = a;
        *self.armed.lock() = false;
        self.cancel();
    }

    pub fn cancel(&self) {
        if let Some(t) = self.countdown.lock().take() {
            t.abort();
        }
    }

    /// Called every tick.
    pub fn on_tick(&self, app: &AppHandle, torrents: &[TorrentSummary]) {
        let action = self.get();
        if action == Action::Nothing || self.countdown.lock().is_some() {
            return;
        }
        let busy = torrents.iter().any(downloading);
        let mut armed = self.armed.lock();
        if busy {
            *armed = true;
            return;
        }
        if !*armed {
            return;
        }
        *armed = false;
        drop(armed);
        info!("downloads finished; {action:?} in {}s", COUNTDOWN.as_secs());
        let _ = app.emit(
            "riptor://when-done",
            Countdown {
                action,
                seconds: COUNTDOWN.as_secs(),
            },
        );
        let app = app.clone();
        *self.countdown.lock() = Some(tauri::async_runtime::spawn(async move {
            tokio::time::sleep(COUNTDOWN).await;
            let wd = app.state::<WhenDone>();
            *wd.action.lock() = Action::Nothing;
            wd.countdown.lock().take();
            if let Err(e) = run(&app, action).await {
                warn!("{action:?} failed: {e:#}");
            }
        }));
    }
}

async fn run(app: &AppHandle, action: Action) -> anyhow::Result<()> {
    match action {
        Action::Nothing => Ok(()),
        Action::Sleep => sleep(),
        Action::Shutdown => {
            // Save state before Windows closes us.
            crate::shut_down(app).await;
            shutdown()
        }
    }
}

#[cfg(windows)]
fn system32(exe: &str) -> std::path::PathBuf {
    std::path::Path::new(&std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
        .join("System32")
        .join(exe)
}

#[cfg(windows)]
fn sleep() -> anyhow::Result<()> {
    // Suspend, not hibernate, and let apps veto as usual.
    let ps = system32(r"WindowsPowerShell\v1.0\powershell.exe");
    let ok = std::process::Command::new(ps)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; [void][System.Windows.Forms.Application]::SetSuspendState('Suspend', $false, $false)",
        ])
        .status()?
        .success();
    anyhow::ensure!(ok, "Windows refused to sleep");
    Ok(())
}

#[cfg(windows)]
fn shutdown() -> anyhow::Result<()> {
    let ok = std::process::Command::new(system32("shutdown.exe"))
        .args(["/s", "/t", "0", "/d", "p:0:0"])
        .status()?
        .success();
    anyhow::ensure!(ok, "Windows refused to shut down");
    std::process::exit(0);
}

#[cfg(not(windows))]
fn systemctl(verb: &str) -> anyhow::Result<()> {
    let ok = std::process::Command::new("/usr/bin/systemctl")
        .arg(verb)
        .status()?
        .success();
    anyhow::ensure!(ok, "systemctl {verb} failed");
    Ok(())
}

#[cfg(not(windows))]
fn sleep() -> anyhow::Result<()> {
    systemctl("suspend")
}

#[cfg(not(windows))]
fn shutdown() -> anyhow::Result<()> {
    systemctl("poweroff")?;
    std::process::exit(0);
}
