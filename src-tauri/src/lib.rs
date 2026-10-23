mod associations;
mod autostart;
mod commands;
mod elevate;
mod logs;
mod rss;
mod tray;
mod updater;
mod vpn;
mod when_done;

use std::sync::Arc;

use parking_lot::Mutex;
use riptor_core::{Engine, EngineConfig, Event, Source};
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::OnceCell;
use tracing::{error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[derive(Default)]
pub struct AppState {
    pub engine: OnceCell<Arc<Engine>>,
    pub startup_error: Mutex<Option<String>>,
    pub startup_items: Mutex<Vec<Source>>,
    pub search: OnceCell<Arc<riptor_search::SearchManager>>,
    pub vpn: OnceCell<Arc<vpn::VpnManager>>,
    pub rss: OnceCell<Arc<riptor_rss::Rss>>,
    /// The remote control server (qBittorrent Web API), when on.
    pub webui: Arc<parking_lot::Mutex<riptor_webapi::Status>>,
}

/// Magnet links and existing .torrent files only.
fn items_from_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Vec<Source> {
    args.into_iter()
        .filter_map(|a| {
            let a = a.trim();
            if a.starts_with("magnet:?") {
                Some(Source::Magnet(a.to_string()))
            } else {
                // Linux file managers pass file:// URLs to `Exec=... %U`.
                let p = if a.starts_with("file://") {
                    url::Url::parse(a).ok()?.to_file_path().ok()?
                } else {
                    std::path::PathBuf::from(a)
                };
                let is_torrent = p
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("torrent"));
                (is_torrent && p.is_file()).then_some(Source::File(p))
            }
        })
        .collect()
}

/// Always goes through the add dialog, never added silently.
fn forward_external(app: &AppHandle, items: Vec<Source>) {
    if items.is_empty() {
        return;
    }
    tray::show_main(app);
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
                        Event::Tick { global, torrents } => {
                            tray::update(&app, global);
                            app.state::<when_done::WhenDone>().on_tick(&app, torrents);
                        }
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
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,librqbit=warn")),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(logs::Capture)
        .init();

    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let app = builder
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Launching Riptor again brings back a window hidden in the tray.
            tray::show_main(app);
            forward_external(
                app,
                items_from_args(argv.iter().skip(1).map(String::as_str)),
            );
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let to_tray = window
                    .state::<AppState>()
                    .engine
                    .get()
                    .is_some_and(|e| e.settings().close_to_tray);
                if to_tray && window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                    tray::set_toggle_text(window.app_handle(), false);
                }
            }
        })
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
            updater::init(&handle);
            handle.manage(when_done::WhenDone::default());
            if autostart::started_minimized() {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.hide();
                    tray::set_toggle_text(&handle, false);
                }
            }

            // Not in dev builds, or the registry would point at target/debug.
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
            let data_dir_rss = data_dir.clone();
            let data_dir_webui = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                // The OS credential store can be slow; keep it off the UI thread.
                let _ =
                    tauri::async_runtime::spawn_blocking(riptor_core::history::use_os_key).await;
                match Engine::start(EngineConfig { data_dir }).await {
                    Ok(engine) => {
                        info!("engine started");
                        match riptor_core::input::http_client().and_then(|http| {
                            riptor_search::SearchManager::new(&data_dir_search, http)
                        }) {
                            Ok(s) => {
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
                                let _ = handle.state::<AppState>().vpn.set(v.clone());
                                v.auto_connect();
                            }
                            Err(e) => error!("VPN failed to start: {e:#}"),
                        }
                        let feeds = Arc::new(riptor_rss::Rss::open(&data_dir_rss));
                        let _ = handle.state::<AppState>().rss.set(feeds.clone());
                        rss::spawn(handle.clone(), engine.clone(), feeds);
                        tauri::async_runtime::spawn(riptor_webapi::supervise(
                            engine.clone(),
                            data_dir_webui,
                            handle.state::<AppState>().webui.clone(),
                        ));
                        if updater::SUPPORTED && engine.settings().check_updates {
                            let h = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                                match updater::check(&h).await {
                                    Ok(Some(u)) => {
                                        info!("update available: {}", u.version);
                                        let _ = h.emit("riptor://update-available", u);
                                    }
                                    Ok(None) => {}
                                    Err(e) => tracing::warn!("update check failed: {e:#}"),
                                }
                            });
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
            commands::about_info,
            commands::country_status,
            commands::recent_logs,
            commands::create_torrent,
            commands::create_torrent_progress,
            commands::create_torrent_cancel,
            commands::engine_status,
            commands::take_startup_items,
            commands::preview_torrent,
            commands::add_torrent,
            commands::stop_torrents,
            commands::resume_torrents,
            commands::force_start_torrents,
            commands::remove_torrents,
            commands::set_files,
            commands::set_file_priority,
            commands::autostart_get,
            commands::when_done_get,
            commands::when_done_set,
            commands::when_done_cancel,
            commands::update_supported,
            commands::update_check,
            commands::update_install,
            commands::update_download,
            commands::webui_state,
            commands::webui_set_password,
            commands::webui_new_api_key,
            commands::webui_remove_api_key,
            commands::webui_set_ddns_password,
            commands::autostart_set,
            commands::reveal_torrent_file,
            commands::torrent_pieces,
            commands::rename_files,
            commands::set_trackers,
            commands::import_torrents,
            commands::import_scan,
            commands::vpn_leak_check,
            commands::verify_checksums,
            commands::torrent_safety,
            commands::connections,
            commands::connections_clear,
            commands::swarm_view,
            commands::release_leak_hold,
            commands::network_info,
            commands::check_data,
            commands::preview_file,
            commands::reannounce,
            commands::add_peer,
            commands::ban_peer,
            commands::open_torrent_file,
            commands::export_torrent,
            commands::rss_view,
            commands::rss_articles,
            commands::rss_add_feed,
            commands::rss_remove_feed,
            commands::rss_rename_feed,
            commands::rss_mark_read,
            commands::rss_refresh,
            commands::rss_set_options,
            commands::rss_save_rule,
            commands::rss_delete_rule,
            commands::rss_preview_rule,
            commands::torrent_files,
            commands::torrent_trackers,
            commands::torrent_peers,
            commands::torrent_magnet,
            commands::free_space,
            commands::public_ip,
            commands::set_category,
            commands::move_torrents,
            commands::recheck_torrents,
            commands::set_tags,
            commands::set_share_limits,
            commands::set_speed_limits,
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
        RunEvent::ExitRequested { api, .. } => {
            api.prevent_exit();
            quit(handle);
        }
        // Only if something bypassed quit(). The runtime is already gone here.
        RunEvent::Exit => {
            let _ = handle.remove_tray_by_id(tray::TRAY_ID);
            std::process::exit(0);
        }
        _ => {}
    });
}

/// Not VISIBLE: a window hidden to the tray should still open next launch.
fn window_state_flags() -> tauri_plugin_window_state::StateFlags {
    use tauri_plugin_window_state::StateFlags;
    StateFlags::all() - StateFlags::VISIBLE
}

static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Needs a live runtime: in RunEvent::Exit librqbit's stop() hangs.
pub fn quit(handle: &AppHandle) {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(8);
    if QUITTING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // process::exit below skips the plugin's own save.
    {
        use tauri_plugin_window_state::AppHandleExt;
        let _ = handle.save_window_state(window_state_flags());
    }
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.hide();
    }
    std::thread::spawn(|| {
        std::thread::sleep(LIMIT + std::time::Duration::from_secs(4));
        error!("shutdown did not finish in time; exiting anyway");
        std::process::exit(1);
    });
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        shut_down(&handle).await;
        std::process::exit(0);
    });
}

/// Stops the VPN and the engine; also used before an update installs.
pub async fn shut_down(handle: &AppHandle) {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(8);
    {
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
        // Otherwise Windows leaves a dead icon in the tray.
        let _ = handle.remove_tray_by_id(tray::TRAY_ID);
    }
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

    #[test]
    fn file_urls_from_file_managers_are_read_as_paths() {
        let dir = std::env::temp_dir().join("riptor-args-url-test");
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("with space.torrent");
        std::fs::write(&t, b"d4:infode").unwrap();
        let file_url = url::Url::from_file_path(&t).unwrap().to_string();
        assert!(file_url.contains("%20"));
        let items = items_from_args([
            file_url.as_str(),
            "file://remote-host/x.torrent",
            "file:///nowhere/missing.torrent",
            "file://",
        ]);
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], Source::File(p) if *p == t));
    }
}
