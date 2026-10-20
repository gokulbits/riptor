use std::sync::Arc;

use parking_lot::Mutex;
use riptor_core::{Engine, GlobalStats};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

pub const TRAY_ID: &str = "riptor-tray";

pub struct TrayItems {
    toggle: MenuItem<Wry>,
    alt_speed: CheckMenuItem<Wry>,
    last_alt: Mutex<Option<bool>>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let sep = || PredefinedMenuItem::separator(app);

    let toggle = item("toggle", "Hide Riptor")?;
    let add_file = item("add_file", "Add torrent file…")?;
    let add_magnet = item("add_magnet", "Add magnet link…")?;
    let resume_all = item("resume_all", "Resume all")?;
    let pause_all = item("pause_all", "Pause all")?;
    let alt_speed = CheckMenuItem::with_id(
        app,
        "alt_speed",
        "Alternative speed limits",
        true,
        false,
        None::<&str>,
    )?;
    let vpn = item("vpn", "VPN…")?;
    let stats = item("stats", "Statistics…")?;
    let settings = item("settings", "Settings…")?;
    let open_dir = item("open_dir", "Open download folder")?;
    let quit = item("quit", "Quit Riptor")?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &sep()?,
            &add_file,
            &add_magnet,
            &sep()?,
            &resume_all,
            &pause_all,
            &alt_speed,
            &sep()?,
            &vpn,
            &stats,
            &settings,
            &open_dir,
            &sep()?,
            &quit,
        ],
    )?;

    app.manage(TrayItems {
        toggle,
        alt_speed,
        last_alt: Mutex::new(None),
    });

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Riptor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn engine(app: &AppHandle) -> Option<Arc<Engine>> {
    app.state::<AppState>().engine.get().cloned()
}

fn ui_action(app: &AppHandle, action: &str) {
    show_main(app);
    let _ = app.emit("riptor://tray-action", action);
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle" => toggle_main(app),
        "add_file" | "add_magnet" | "vpn" | "stats" | "settings" => ui_action(app, id),
        "alt_speed" => {
            if let Some(e) = engine(app) {
                let on = !e.settings().alt_speed_enabled;
                if let Err(err) = e.set_alt_speed(on) {
                    tracing::warn!("alt speed: {err:#}");
                }
                sync_alt(app, on);
            }
        }
        "resume_all" | "pause_all" => {
            if let Some(e) = engine(app) {
                let pause = id == "pause_all";
                tauri::async_runtime::spawn(async move {
                    let ids = e.torrent_ids();
                    let r = if pause {
                        e.stop(&ids).await
                    } else {
                        e.resume(&ids)
                    };
                    if let Err(err) = r {
                        tracing::warn!(
                            "{}: {err:#}",
                            if pause { "pause all" } else { "resume all" }
                        );
                    }
                });
            }
        }
        "open_dir" => {
            if let Some(e) = engine(app) {
                let dir = e.settings().download_dir;
                if dir.is_dir() {
                    let _ = app.opener().open_path(dir.to_string_lossy(), None::<&str>);
                }
            }
        }
        "quit" => crate::quit(app),
        _ => {}
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    set_toggle_text(app, true);
}

fn toggle_main(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
    if visible {
        let _ = w.hide();
        set_toggle_text(app, false);
    } else {
        show_main(app);
    }
}

pub fn set_toggle_text(app: &AppHandle, visible: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.toggle.set_text(if visible {
            "Hide Riptor"
        } else {
            "Show Riptor"
        });
    }
}

fn sync_alt(app: &AppHandle, on: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let mut last = items.last_alt.lock();
        if *last != Some(on) {
            let _ = items.alt_speed.set_checked(on);
            *last = Some(on);
        }
    }
}

fn rate(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut v = bps as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn update(app: &AppHandle, g: &GlobalStats) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let vpn = match g.network.as_str() {
            "vpn" => "\nVPN: connected",
            "blocked" => "\nVPN: kill switch engaged",
            _ => "",
        };
        let _ = tray.set_tooltip(Some(format!(
            "Riptor\n↓ {}  ↑ {}{vpn}",
            rate(g.dl_rate),
            rate(g.ul_rate)
        )));
    }
    sync_alt(app, g.alt_speed);
}
