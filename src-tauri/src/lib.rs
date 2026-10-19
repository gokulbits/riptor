mod associations;
mod commands;
mod elevate;
mod tray;
mod vpn;

use std::sync::Arc;

use parking_lot::Mutex;
use riptor_core::{Engine, EngineConfig, Event, Source};
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::OnceCell;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Default)]
pub struct AppState {
    pub engine: OnceCell<Arc<Engine>>,
    pub startup_error: Mutex<Option<String>>,
    pub startup_items: Mutex<Vec<Source>>,
    pub search: OnceCell<Arc<riptor_search::SearchManager>>,
    pub vpn: OnceCell<Arc<vpn::VpnManager>>,
}

/// Turn command-line arguments / deep links into add requests. Anything that
/// is not a magnet link or an existing `.torrent` file is ignored.
fn items_from_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Vec<Source> {
    args.into_iter()
        .filter_map(|a| {
            let a = a.trim();
            if a.starts_with("magnet:?") {
                Some(Source::Magnet(a.to_string()))
            } else {
                let p = std::path::PathBuf::from(a);
                let is_torrent = p
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("torrent"));
                (is_torrent && p.is_file()).then_some(Source::File(p))
            }
        })
        .collect()
}

/// External add requests always go to the UI's add dialog; nothing is added
/// without the user seeing it.
fn forward_external(app: &AppHandle, items: Vec<Source>) {
    if items.is_empty() {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    let _ = app.emit("riptor://external-add", items);
}

fn pump_events(app: AppHandle, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    match &ev {
                        Event::Finished { name, .. } => {
                            let _ = app
                                .notification()
                                .builder()
                                .title("Download complete")
                                .body(name)
                                .show();
                        }
                        Event::Tick { global, .. } => tray::update(&app, global),
                        _ => {}
                    }
                    let _ = app.emit("riptor://event", ev);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,librqbit=warn")),
        )
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            forward_external(
                app,
                items_from_args(argv.iter().skip(1).map(String::as_str)),
            );
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let args: Vec<String> = std::env::args().skip(1).collect();
            *app.state::<AppState>().startup_items.lock() =
                items_from_args(args.iter().map(String::as_str));

            let dl = handle.clone();
            app.deep_link().on_open_url(move |event| {
                let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                forward_external(&dl, items_from_args(urls.iter().map(String::as_str)));
            });

            tray::create(&handle)?;

            // Installed builds list Riptor under "Open with" (never changes defaults).
            // Skipped in dev so the registry never points at a target/debug exe.
            if !cfg!(debug_assertions) {
                std::thread::spawn(|| {
                    if let Err(e) = associations::register() {
                        tracing::warn!("file association registration failed: {e:#}");
                    }
                });
            }

            let data_dir = app.path().app_data_dir()?;
            let data_dir_search = data_dir.clone();
            let data_dir_vpn = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                match Engine::start(EngineConfig { data_dir }).await {
                    Ok(engine) => {
                        info!("engine started");
                        match riptor_core::input::http_client().and_then(|http| {
                            riptor_search::SearchManager::new(&data_dir_search, http)
                        }) {
                            Ok(s) => {
                                // Installed builds ship Python next to Riptor.
                                if let Ok(res) = handle.path().resource_dir() {
                                    s.set_shipped_python(res.join("python"));
                                }
                                let _ = handle.state::<AppState>().search.set(s.clone());
                                tauri::async_runtime::spawn(
                                    async move { s.refresh_plugins().await },
                                );
                            }
                            Err(e) => error!("search failed to start: {e:#}"),
                        }
                        match vpn::VpnManager::new(handle.clone(), engine.clone(), &data_dir_vpn) {
                            Ok(v) => {
                                let _ = handle.state::<AppState>().vpn.set(v);
                            }
                            Err(e) => error!("VPN failed to start: {e:#}"),
                        }
                        pump_events(handle.clone(), engine.clone());
                        let _ = handle.state::<AppState>().engine.set(engine);
                        let _ = handle.emit("riptor://ready", ());
                    }
                    Err(e) => {
                        error!("engine failed to start: {e:#}");
                        *handle.state::<AppState>().startup_error.lock() = Some(format!("{e:#}"));
                        let _ = handle.emit("riptor://ready", ());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_version,
            commands::engine_status,
            commands::take_startup_items,
            commands::preview_torrent,
            commands::add_torrent,
            commands::stop_torrents,
            commands::resume_torrents,
            commands::force_start_torrents,
            commands::remove_torrents,
            commands::set_files,
            commands::torrent_files,
            commands::torrent_trackers,
            commands::torrent_peers,
            commands::set_category,
            commands::set_tags,
            commands::set_share_limits,
            commands::queue_move,
            commands::get_categories,
            commands::save_category,
            commands::delete_category,
            commands::get_settings,
            commands::update_settings,
            commands::set_alt_speed,
            commands::reveal_torrent,
            commands::open_web_url,
            commands::statistics,
            commands::create_backup,
            commands::inspect_backup,
            commands::restore_backup,
            commands::association_status,
            commands::make_default_app,
            commands::open_default_apps,
            commands::search_providers,
            commands::search_python_status,
            commands::search_start,
            commands::search_cancel,
            commands::search_set_enabled,
            commands::search_add_torznab,
            commands::search_remove_torznab,
            commands::search_preview_plugin,
            commands::search_install_plugin,
            commands::search_uninstall_plugin,
            commands::search_plugin_download,
            commands::search_catalog,
            commands::search_install_python,
            commands::search_remove_python,
            commands::search_options,
            commands::search_set_scrape,
            commands::vpn_profiles,
            commands::vpn_import,
            commands::vpn_delete,
            commands::vpn_set_credentials,
            commands::vpn_connect,
            commands::vpn_disconnect,
            commands::vpn_status,
            commands::vpn_get_prefs,
            commands::vpn_set_prefs,
            commands::vpn_openvpn_installed,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Riptor");

    app.run(|handle, event| match event {
        // Last window closed (or the OS asked us to exit): shut down properly first.
        RunEvent::ExitRequested { api, .. } => {
            api.prevent_exit();
            quit(handle);
        }
        // Only reached if something bypassed `quit`. The async runtime is
        // already going away here, so don't await anything: just clean up the
        // tray icon and leave.
        RunEvent::Exit => {
            let _ = handle.remove_tray_by_id(tray::TRAY_ID);
            std::process::exit(0);
        }
        _ => {}
    });
}

static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Quit Riptor for real: save statistics, flush every torrent's buffered
/// pieces, remove the tray icon, then end the process.
///
/// This must run while the async runtime is alive. It used to run in
/// `RunEvent::Exit`, where the runtime is already being torn down: timers
/// never fire there, so librqbit's `stop()` (which sleeps for a second) hung
/// forever and the process lingered without a window, which made "Quit" look
/// broken and left the tray icon behind.
pub fn quit(handle: &AppHandle) {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(8);
    if QUITTING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.hide();
    }
    // Last resort if something below blocks a thread forever.
    std::thread::spawn(|| {
        std::thread::sleep(LIMIT + std::time::Duration::from_secs(4));
        error!("shutdown did not finish in time; exiting anyway");
        std::process::exit(1);
    });
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        // Stop OpenVPN first: the tunnel is ours and must not outlive Riptor.
        if let Some(v) = handle.state::<AppState>().vpn.get().cloned() {
            if tokio::time::timeout(std::time::Duration::from_secs(3), v.shutdown())
                .await
                .is_err()
            {
                error!("VPN disconnect timed out");
            }
        }
        if let Some(engine) = handle.state::<AppState>().engine.get().cloned() {
            match tokio::time::timeout(LIMIT, engine.shutdown()).await {
                Ok(Ok(())) => info!("shutdown complete"),
                Ok(Err(e)) => error!("shutdown: {e:#}"),
                Err(_) => error!("shutdown timed out after {LIMIT:?}"),
            }
        }
        // Remove the icon ourselves so Windows doesn't keep a dead one in the tray.
        let _ = handle.remove_tray_by_id(tray::TRAY_ID);
        std::process::exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_args_only_accept_magnets_and_existing_torrent_files() {
        let dir = std::env::temp_dir().join("riptor-args-test");
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("x.torrent");
        std::fs::write(&t, b"d4:infode").unwrap();
        let items = items_from_args([
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            t.to_str().unwrap(),
            "C:\\Windows\\System32\\config\\SAM",
            "--some-flag",
            "javascript:alert(1)",
            dir.join("missing.torrent").to_str().unwrap(),
        ]);
        assert_eq!(items.len(), 2);
    }
}
