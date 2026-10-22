//! IPC commands. Nothing here trusts the webview; riptor-core validates again.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use riptor_core::{
    AddParams, Engine, FileEntry, Preview, QueueMove, Settings, ShareLimitAction, ShareLimits,
    Source,
};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

async fn engine(state: &State<'_, AppState>) -> Res<Arc<Engine>> {
    if let Some(e) = state.engine.get() {
        return Ok(e.clone());
    }
    match state.startup_error.lock().clone() {
        Some(e) => Err(format!("the torrent engine failed to start: {e}")),
        None => Err("the torrent engine is still starting".into()),
    }
}

#[tauri::command]
pub async fn engine_status(state: State<'_, AppState>) -> Res<()> {
    engine(&state).await.map(|_| ())
}

#[tauri::command]
pub fn take_startup_items(state: State<'_, AppState>) -> Vec<Source> {
    std::mem::take(&mut *state.startup_items.lock())
}

#[tauri::command]
pub async fn preview_torrent(state: State<'_, AppState>, source: Source) -> Res<Preview> {
    engine(&state).await?.preview(source).await.map_err(err)
}

#[tauri::command]
pub async fn add_torrent(state: State<'_, AppState>, params: AddParams) -> Res<String> {
    engine(&state).await?.add(params).await.map_err(err)
}

#[tauri::command]
pub async fn stop_torrents(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.stop(&ids).await.map_err(err)
}

#[tauri::command]
pub async fn resume_torrents(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.resume(&ids).map_err(err)
}

#[tauri::command]
pub async fn force_start_torrents(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.force_start(&ids).map_err(err)
}

#[tauri::command]
pub async fn remove_torrents(
    state: State<'_, AppState>,
    ids: Vec<String>,
    delete_files: bool,
) -> Res<()> {
    engine(&state)
        .await?
        .remove(&ids, delete_files)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_files(state: State<'_, AppState>, id: String, files: Vec<usize>) -> Res<()> {
    engine(&state)
        .await?
        .set_files(&id, files)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn torrent_files(state: State<'_, AppState>, id: String) -> Res<Vec<FileEntry>> {
    engine(&state).await?.files(&id).map_err(err)
}

#[tauri::command]
pub async fn torrent_trackers(state: State<'_, AppState>, id: String) -> Res<Vec<String>> {
    engine(&state).await?.trackers(&id).map_err(err)
}

#[tauri::command]
pub async fn torrent_magnet(
    state: State<'_, AppState>,
    id: String,
) -> Res<riptor_core::MagnetLink> {
    engine(&state).await?.magnet_link(&id).map_err(err)
}

#[tauri::command]
pub async fn public_ip(state: State<'_, AppState>) -> Res<riptor_core::PublicIp> {
    engine(&state).await?.public_ip().await.map_err(err)
}

/// Free space only; nothing else about the path is read.
#[tauri::command]
pub async fn free_space(state: State<'_, AppState>, path: PathBuf) -> Res<u64> {
    let e = engine(&state).await?;
    tauri::async_runtime::spawn_blocking(move || e.free_space(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[tauri::command]
pub async fn torrent_peers(
    state: State<'_, AppState>,
    id: String,
) -> Res<Vec<riptor_core::PeerInfo>> {
    engine(&state).await?.peers(&id).map_err(err)
}

#[tauri::command]
pub async fn set_category(
    state: State<'_, AppState>,
    ids: Vec<String>,
    category: Option<String>,
) -> Res<()> {
    let e = engine(&state).await?;
    e.set_category(&ids, category).map_err(err)?;
    if e.settings().move_on_category_change {
        e.move_to_category_folders(&ids).await.map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn move_torrents(state: State<'_, AppState>, ids: Vec<String>, path: PathBuf) -> Res<()> {
    engine(&state)
        .await?
        .move_torrents(&ids, &path)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn recheck_torrents(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.recheck(&ids).await.map_err(err)
}

#[tauri::command]
pub async fn set_tags(state: State<'_, AppState>, ids: Vec<String>, tags: Vec<String>) -> Res<()> {
    engine(&state).await?.set_tags(&ids, tags).map_err(err)
}

#[tauri::command]
pub async fn set_share_limits(
    state: State<'_, AppState>,
    ids: Vec<String>,
    limits: ShareLimits,
    action: Option<ShareLimitAction>,
) -> Res<()> {
    engine(&state)
        .await?
        .set_share_limits(&ids, limits, action)
        .map_err(err)
}

#[tauri::command]
pub async fn set_speed_limits(
    state: State<'_, AppState>,
    ids: Vec<String>,
    dl_kib: Option<u32>,
    ul_kib: Option<u32>,
) -> Res<()> {
    engine(&state)
        .await?
        .set_speed_limits(&ids, dl_kib, ul_kib)
        .map_err(err)
}

#[tauri::command]
pub async fn queue_move(state: State<'_, AppState>, ids: Vec<String>, how: QueueMove) -> Res<()> {
    engine(&state).await?.queue_move(&ids, how).map_err(err)
}

#[tauri::command]
pub async fn get_categories(state: State<'_, AppState>) -> Res<BTreeMap<String, Option<PathBuf>>> {
    Ok(engine(&state).await?.categories())
}

#[tauri::command]
pub async fn save_category(
    state: State<'_, AppState>,
    name: String,
    save_path: Option<PathBuf>,
) -> Res<()> {
    engine(&state)
        .await?
        .set_category_def(&name, save_path)
        .map_err(err)
}

#[tauri::command]
pub async fn delete_category(state: State<'_, AppState>, name: String) -> Res<()> {
    engine(&state)
        .await?
        .remove_category_def(&name)
        .map_err(err)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Res<Settings> {
    Ok(engine(&state).await?.settings())
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: Settings) -> Res<()> {
    engine(&state)
        .await?
        .update_settings(settings)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_alt_speed(state: State<'_, AppState>, enabled: bool) -> Res<()> {
    engine(&state).await?.set_alt_speed(enabled).map_err(err)
}

#[tauri::command]
pub async fn reveal_torrent(app: AppHandle, state: State<'_, AppState>, id: String) -> Res<()> {
    // Revealing a folder opens its parent, so folders are opened directly.
    match engine(&state).await?.reveal_target(&id).map_err(err)? {
        riptor_core::RevealTarget::Folder(dir) => app
            .opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string()),
        riptor_core::RevealTarget::File(file) => app
            .opener()
            .reveal_item_in_dir(&file)
            .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn reveal_torrent_file(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    index: usize,
) -> Res<()> {
    let (path, _, _) = engine(&state).await?.file_path(&id, index).map_err(err)?;
    if !path.is_file() {
        return Err("this file hasn't been created yet".into());
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}

/// Never anything that runs code; those are only shown in their folder.
#[tauri::command]
pub async fn open_torrent_file(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    index: usize,
) -> Res<()> {
    let e = engine(&state).await?;
    let (path, len, ready) = e.file_path(&id, index).map_err(err)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !riptor_core::input::safe_to_open(&name) {
        return Err(
            "Riptor doesn't open programs or files that can run code; use Show in folder".into(),
        );
    }
    if e.flagged_dangerous(&id, index) {
        return Err("This file isn't what its name says, so Riptor won't open it".into());
    }
    // Audio and video can start playing before the rest arrives.
    let enough = ready >= len
        || (riptor_core::input::is_media(&name)
            && ready >= riptor_core::input::stream_threshold(len));
    if !enough || !path.is_file() {
        return Err("this file isn't downloaded far enough to open yet".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// http(s) only.
#[tauri::command]
pub fn open_web_url(app: AppHandle, url: String) -> Res<()> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("only http(s) links can be opened".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(serde::Serialize)]
pub struct AboutInfo {
    version: &'static str,
    tauri: &'static str,
    webview: Option<String>,
    librqbit: &'static str,
    os: &'static str,
    arch: &'static str,
    python: &'static str,
    openvpn_installed: bool,
}

/// Versions for Settings › About.
#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        version: env!("CARGO_PKG_VERSION"),
        tauri: tauri::VERSION,
        webview: tauri::webview_version().ok(),
        // Vendored; see vendor/PATCHES.md.
        librqbit: "9.0.1",
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        python: riptor_search::pyembed::VERSION,
        openvpn_installed: crate::vpn::openvpn_installed(),
    }
}

#[tauri::command]
pub async fn country_status(state: State<'_, AppState>) -> Res<riptor_core::CountryStatus> {
    Ok(engine(&state).await?.country_status())
}

#[tauri::command]
pub fn recent_logs() -> Vec<crate::logs::LogLine> {
    crate::logs::recent()
}

/// Returns the new torrent's id when it was added to seed.
#[tauri::command]
pub async fn create_torrent(
    state: State<'_, AppState>,
    source: PathBuf,
    output: PathBuf,
    options: riptor_core::create::CreateOptions,
    seed: bool,
) -> Res<Option<String>> {
    engine(&state)
        .await?
        .create_torrent(source, output, options, seed)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn create_torrent_progress(state: State<'_, AppState>) -> Res<Option<(u64, u64)>> {
    Ok(engine(&state).await?.create_progress())
}

#[tauri::command]
pub async fn create_torrent_cancel(state: State<'_, AppState>) -> Res<()> {
    engine(&state).await?.cancel_create();
    Ok(())
}

#[tauri::command]
pub async fn statistics(state: State<'_, AppState>) -> Res<riptor_core::StatsReport> {
    Ok(engine(&state).await?.statistics())
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, AppState>,
    path: PathBuf,
    password: Option<String>,
) -> Res<riptor_core::BackupSummary> {
    let e = engine(&state).await?;
    tauri::async_runtime::spawn_blocking(move || e.create_backup(&path, password))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[tauri::command]
pub async fn inspect_backup(
    state: State<'_, AppState>,
    path: PathBuf,
    password: Option<String>,
) -> Res<riptor_core::BackupSummary> {
    let e = engine(&state).await?;
    // Argon2 is slow on purpose.
    tauri::async_runtime::spawn_blocking(move || e.inspect_backup(&path, password))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, AppState>,
    path: PathBuf,
    password: Option<String>,
    options: riptor_core::RestoreOptions,
) -> Res<riptor_core::RestoreReport> {
    engine(&state)
        .await?
        .restore_backup(&path, password, options)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn association_status() -> crate::associations::AssociationStatus {
    crate::associations::status()
}

#[tauri::command]
pub fn make_default_app() -> Res<crate::associations::AssociationStatus> {
    crate::associations::make_default().map_err(err)?;
    Ok(crate::associations::status())
}

#[tauri::command]
pub fn open_default_apps() {
    crate::associations::open_default_apps();
}

// ---------------------------------------------------------------- search

fn search(state: &State<'_, AppState>) -> Res<Arc<riptor_search::SearchManager>> {
    state
        .search
        .get()
        .cloned()
        .ok_or_else(|| "search is still starting".to_string())
}

#[tauri::command]
pub fn search_providers(state: State<'_, AppState>) -> Res<Vec<riptor_search::ProviderInfo>> {
    Ok(search(&state)?.providers())
}

#[tauri::command]
pub async fn search_python_status(
    state: State<'_, AppState>,
    recheck: bool,
) -> Res<riptor_search::manager::PythonStatus> {
    let s = search(&state)?;
    let status = s.python_status(recheck).await;
    if recheck {
        s.refresh_plugins().await;
    }
    Ok(status)
}

#[tauri::command]
pub async fn search_start(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    category: String,
    providers: Option<Vec<String>>,
) -> Res<u64> {
    use tauri::Emitter;
    let sink: Arc<dyn Fn(riptor_search::SearchEvent) + Send + Sync> = Arc::new(move |ev| {
        let _ = app.emit("riptor://search", ev);
    });
    let s = search(&state)?;
    route_search(&state, &s);
    s.start(&query, &category, providers, sink)
        .await
        .map_err(err)
}

/// Search traffic follows the torrents' network.
fn route_search(state: &State<'_, AppState>, s: &riptor_search::SearchManager) {
    use riptor_core::NetworkMode;
    use riptor_search::Route;
    let Some(e) = state.engine.get() else {
        return;
    };
    s.set_route(match e.network() {
        NetworkMode::Normal => Route::Direct,
        NetworkMode::Blocked => Route::Held,
        // Never the system resolver while on the VPN.
        NetworkMode::Vpn(b) => match riptor_core::dns::TunnelResolver::new(
            b.local_ip.into(),
            &riptor_core::dns::SERVERS,
        ) {
            Ok(r) => Route::Tunnel(riptor_search::scrape::Tunnel {
                local: b.local_ip.into(),
                lookup: r.hook(),
            }),
            Err(_) => Route::Held,
        },
    });
}

#[tauri::command]
pub fn search_cancel(state: State<'_, AppState>, id: u64) -> Res<()> {
    search(&state)?.cancel(id);
    Ok(())
}

#[tauri::command]
pub fn search_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Res<()> {
    search(&state)?.set_enabled(&id, enabled).map_err(err)
}

#[tauri::command]
pub fn search_add_torznab(
    state: State<'_, AppState>,
    name: String,
    url: String,
    api_key: String,
) -> Res<()> {
    search(&state)?
        .add_torznab(&name, &url, &api_key)
        .map_err(err)
}

#[tauri::command]
pub fn search_remove_torznab(state: State<'_, AppState>, id: String) -> Res<()> {
    search(&state)?.remove_torznab(&id).map_err(err)
}

#[tauri::command]
pub async fn search_preview_plugin(
    state: State<'_, AppState>,
    source: String,
) -> Res<riptor_search::nova::PluginPreview> {
    search(&state)?.preview_plugin(&source).await.map_err(err)
}

#[tauri::command]
pub async fn search_install_plugin(state: State<'_, AppState>, sha256: String) -> Res<()> {
    search(&state)?.install_plugin(&sha256).await.map_err(err)
}

#[tauri::command]
pub async fn search_uninstall_plugin(state: State<'_, AppState>, id: String) -> Res<()> {
    search(&state)?.uninstall_plugin(&id).await.map_err(err)
}

#[tauri::command]
pub async fn search_catalog(
    state: State<'_, AppState>,
    refresh: bool,
) -> Res<riptor_search::catalog::Catalog> {
    Ok(search(&state)?.catalog(refresh).await)
}

#[tauri::command]
pub async fn search_install_python(
    state: State<'_, AppState>,
) -> Res<riptor_search::manager::PythonStatus> {
    search(&state)?.install_python().await.map_err(err)
}

#[tauri::command]
pub async fn search_remove_python(
    state: State<'_, AppState>,
) -> Res<riptor_search::manager::PythonStatus> {
    search(&state)?.remove_python().await.map_err(err)
}

#[tauri::command]
pub fn search_options(state: State<'_, AppState>) -> Res<riptor_search::manager::SearchOptions> {
    Ok(search(&state)?.options())
}

#[tauri::command]
pub fn search_set_scrape(state: State<'_, AppState>, enabled: bool) -> Res<()> {
    search(&state)?.set_scrape(enabled).map_err(err)
}

/// Returns a temp .torrent path for the add dialog.
#[tauri::command]
pub async fn search_plugin_download(
    state: State<'_, AppState>,
    provider: String,
    url: String,
) -> Res<String> {
    let s = search(&state)?;
    route_search(&state, &s);
    let p = s.plugin_download(&provider, &url).await.map_err(err)?;
    Ok(p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- VPN

fn vpn(state: &State<'_, AppState>) -> Res<Arc<crate::vpn::VpnManager>> {
    state
        .vpn
        .get()
        .cloned()
        .ok_or_else(|| "the VPN is still starting".to_string())
}

/// No config text, no secrets.
#[derive(serde::Serialize)]
pub struct VpnProfileView {
    id: String,
    name: String,
    kind: riptor_vpn::profiles::Kind,
    servers: Vec<String>,
    removed: Vec<String>,
    needs_credentials: bool,
    has_credentials: bool,
    app_only_supported: bool,
    created: u64,
}

fn profile_view(
    store: &riptor_vpn::profiles::ProfileStore,
    p: riptor_vpn::profiles::Profile,
) -> VpnProfileView {
    VpnProfileView {
        has_credentials: p.needs_credentials && store.has_credentials(&p.id),
        app_only_supported: p.app_only_supported(),
        id: p.id,
        name: p.name,
        kind: p.kind,
        servers: p.servers,
        removed: p.removed,
        needs_credentials: p.needs_credentials,
        created: p.created,
    }
}

#[tauri::command]
pub fn vpn_profiles(state: State<'_, AppState>) -> Res<Vec<VpnProfileView>> {
    let v = vpn(&state)?;
    let store = v.store();
    Ok(store
        .list()
        .into_iter()
        .map(|p| profile_view(store, p))
        .collect())
}

/// The file picker is opened here so the webview never names a path.
#[tauri::command]
pub async fn vpn_import(app: AppHandle, state: State<'_, AppState>) -> Res<Option<VpnProfileView>> {
    use std::io::Read;
    use tauri_plugin_dialog::DialogExt;
    let v = vpn(&state)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Import a VPN profile")
            .add_filter("OpenVPN or WireGuard profile", &["ovpn", "conf"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "VPN".into());
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("not a file".into());
    }
    // One byte over the limit so the parser rejects it.
    let mut bytes = Vec::new();
    file.take(riptor_vpn::ovpn::MAX_PROFILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let store = v.store();
    let p = store.import(&name, &bytes).map_err(err)?;
    Ok(Some(profile_view(store, p)))
}

#[tauri::command]
pub async fn vpn_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    let v = vpn(&state)?;
    if v.status().profile_id.as_deref() == Some(id.as_str()) {
        v.disconnect().await;
    }
    v.store().delete(&id).map_err(err)
}

#[tauri::command]
pub fn vpn_set_credentials(
    state: State<'_, AppState>,
    id: String,
    username: String,
    password: String,
) -> Res<()> {
    vpn(&state)?
        .store()
        .set_credentials(&id, &username, &password)
        .map_err(err)
}

#[tauri::command]
pub async fn vpn_connect(state: State<'_, AppState>, id: String) -> Res<()> {
    vpn(&state)?.connect(&id).await.map_err(err)
}

#[tauri::command]
pub async fn vpn_disconnect(state: State<'_, AppState>) -> Res<()> {
    vpn(&state)?.disconnect().await;
    Ok(())
}

#[tauri::command]
pub fn vpn_status(state: State<'_, AppState>) -> Res<crate::vpn::VpnStatus> {
    Ok(vpn(&state)?.status())
}

#[tauri::command]
pub fn vpn_get_prefs(state: State<'_, AppState>) -> Res<crate::vpn::VpnPrefs> {
    Ok(vpn(&state)?.prefs())
}

#[tauri::command]
pub fn vpn_set_prefs(state: State<'_, AppState>, prefs: crate::vpn::VpnPrefs) -> Res<()> {
    vpn(&state)?.set_prefs(prefs).map_err(err)
}

#[tauri::command]
pub fn vpn_openvpn_installed() -> bool {
    crate::vpn::openvpn_installed()
}

#[tauri::command]
pub async fn set_file_priority(
    state: State<'_, AppState>,
    id: String,
    files: Vec<usize>,
    priority: riptor_core::FilePriority,
) -> Res<()> {
    engine(&state)
        .await?
        .set_file_priority(&id, &files, priority)
        .map_err(err)
}

/// `path` comes from the save dialog.
#[tauri::command]
pub async fn export_torrent(
    state: State<'_, AppState>,
    id: String,
    path: PathBuf,
    strip_secrets: bool,
) -> Res<()> {
    engine(&state)
        .await?
        .export_torrent(&id, &path, strip_secrets)
        .map_err(err)
}

// ---------------------------------------------------------------- RSS

fn rss(state: &State<'_, AppState>) -> Res<Arc<riptor_rss::Rss>> {
    state
        .rss
        .get()
        .cloned()
        .ok_or_else(|| "RSS is still starting".to_string())
}

#[tauri::command]
pub fn rss_view(state: State<'_, AppState>) -> Res<riptor_rss::View> {
    Ok(rss(&state)?.view())
}

#[tauri::command]
pub fn rss_articles(state: State<'_, AppState>, feed: u32) -> Res<Vec<riptor_rss::Article>> {
    rss(&state)?.articles(feed).map_err(err)
}

#[tauri::command]
pub async fn rss_add_feed(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    name: Option<String>,
) -> Res<u32> {
    let r = rss(&state)?;
    let id = r.add_feed(&url, name.as_deref()).map_err(err)?;
    let e = engine(&state).await?;
    crate::rss::refresh(&app, &e, &r, true, Some(vec![id])).await;
    Ok(id)
}

#[tauri::command]
pub fn rss_remove_feed(state: State<'_, AppState>, feed: u32) -> Res<()> {
    rss(&state)?.remove_feed(feed).map_err(err)
}

#[tauri::command]
pub fn rss_rename_feed(state: State<'_, AppState>, feed: u32, name: String) -> Res<()> {
    rss(&state)?.rename_feed(feed, &name).map_err(err)
}

#[tauri::command]
pub fn rss_mark_read(state: State<'_, AppState>, feed: u32, article: Option<String>) -> Res<()> {
    rss(&state)?
        .mark_read(feed, article.as_deref())
        .map_err(err)
}

#[tauri::command]
pub async fn rss_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    feeds: Option<Vec<u32>>,
) -> Res<()> {
    let r = rss(&state)?;
    let e = engine(&state).await?;
    crate::rss::refresh(&app, &e, &r, true, feeds).await;
    Ok(())
}

#[tauri::command]
pub fn rss_set_options(state: State<'_, AppState>, options: riptor_rss::Options) -> Res<()> {
    rss(&state)?.set_options(options).map_err(err)
}

#[tauri::command]
pub async fn rss_save_rule(state: State<'_, AppState>, rule: riptor_rss::Rule) -> Res<u32> {
    if let Some(p) = &rule.save_path {
        engine(&state).await?.check_root(p).map_err(err)?;
    }
    rss(&state)?.save_rule(rule).map_err(err)
}

#[tauri::command]
pub fn rss_delete_rule(state: State<'_, AppState>, id: u32) -> Res<()> {
    rss(&state)?.delete_rule(id).map_err(err)
}

#[tauri::command]
pub fn rss_preview_rule(
    state: State<'_, AppState>,
    rule: riptor_rss::Rule,
) -> Res<Vec<riptor_rss::RuleMatch>> {
    rss(&state)?.preview_rule(&rule).map_err(err)
}

#[tauri::command]
pub fn autostart_get() -> bool {
    crate::autostart::enabled()
}

#[tauri::command]
pub fn autostart_set(enabled: bool) -> Res<()> {
    crate::autostart::set(enabled).map_err(err)
}

#[tauri::command]
pub fn update_supported() -> bool {
    crate::updater::SUPPORTED
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Res<Option<crate::updater::UpdateInfo>> {
    crate::updater::check(&app).await.map_err(err)
}

/// Riptor exits once the installer starts.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Res<()> {
    crate::updater::install(&app).await.map_err(err)
}

/// Downloads the update found by the last check; `riptor://update-ready` follows.
#[tauri::command]
pub async fn update_download(app: AppHandle) -> Res<()> {
    crate::updater::download(&app).await.map_err(err)
}

#[tauri::command]
pub fn when_done_get(state: State<'_, crate::when_done::WhenDone>) -> crate::when_done::Action {
    state.get()
}

#[tauri::command]
pub fn when_done_set(
    state: State<'_, crate::when_done::WhenDone>,
    action: crate::when_done::Action,
) {
    state.set(action);
}

/// Stops a running countdown and turns the action off.
#[tauri::command]
pub fn when_done_cancel(state: State<'_, crate::when_done::WhenDone>) {
    state.set(crate::when_done::Action::Nothing);
}

#[tauri::command]
pub async fn torrent_pieces(
    state: State<'_, AppState>,
    id: String,
    buckets: usize,
) -> Res<riptor_core::PieceMap> {
    engine(&state).await?.piece_map(&id, buckets).map_err(err)
}

/// `changes`: (file index, new path relative to the torrent's folder, `/` between folders).
#[tauri::command]
pub async fn rename_files(
    state: State<'_, AppState>,
    id: String,
    changes: Vec<(usize, String)>,
) -> Res<()> {
    engine(&state)
        .await?
        .rename_files(&id, &changes)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_trackers(
    state: State<'_, AppState>,
    id: String,
    trackers: Vec<String>,
) -> Res<()> {
    engine(&state)
        .await?
        .set_trackers(&id, &trackers)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn reannounce(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.reannounce(&ids).await.map_err(err)
}

#[tauri::command]
pub async fn add_peer(state: State<'_, AppState>, id: String, addr: String) -> Res<()> {
    engine(&state).await?.add_peer(&id, &addr).map_err(err)
}

#[tauri::command]
pub async fn ban_peer(state: State<'_, AppState>, ip: String) -> Res<()> {
    let ip: std::net::IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| "not an IP address".to_string())?;
    engine(&state).await?.ban_peer(ip).await.map_err(err)
}

/// `path` comes from the folder picker.
#[tauri::command]
pub async fn import_torrents(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Res<riptor_core::import::ImportReport> {
    engine(&state).await?.import_from(&path).await.map_err(err)
}

/// `None` looks in qBittorrent's usual folders; nothing is added.
#[tauri::command]
pub async fn import_scan(
    state: State<'_, AppState>,
    path: Option<PathBuf>,
) -> Res<riptor_core::import::ImportScan> {
    engine(&state).await?.import_scan(path).await.map_err(err)
}

#[derive(serde::Serialize)]
pub struct Connection {
    purpose: &'static str,
    host: String,
    route: &'static str,
    count: u64,
    failures: u64,
    first: u64,
    last: u64,
}

/// Hosts contacted this session; kept in memory only.
#[tauri::command]
pub fn connections() -> Vec<Connection> {
    riptor_net::ledger::snapshot()
        .into_iter()
        .map(|e| Connection {
            purpose: e.purpose.label(),
            host: e.host,
            route: e.route.label(),
            count: e.count,
            failures: e.failures,
            first: e.first,
            last: e.last,
        })
        .collect()
}

#[tauri::command]
pub fn connections_clear() {
    riptor_net::ledger::clear();
}

/// `None` until the OS has been asked (a few seconds after start).
#[tauri::command]
pub async fn network_info(
    state: State<'_, AppState>,
) -> Res<Option<riptor_core::netinfo::NetInfo>> {
    Ok(engine(&state).await?.net_info())
}

#[tauri::command]
pub async fn swarm_view(state: State<'_, AppState>) -> Res<riptor_core::SwarmView> {
    Ok(engine(&state).await?.swarm_view())
}

#[tauri::command]
pub async fn release_leak_hold(state: State<'_, AppState>) -> Res<()> {
    engine(&state).await?.release_leak_hold();
    Ok(())
}

#[tauri::command]
pub async fn preview_file(
    state: State<'_, AppState>,
    id: String,
    index: usize,
) -> Res<riptor_core::preview::FilePreview> {
    engine(&state)
        .await?
        .preview_file(&id, index)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn check_data(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.check_data(&ids).map_err(err)
}

#[tauri::command]
pub async fn verify_checksums(state: State<'_, AppState>, ids: Vec<String>) -> Res<()> {
    engine(&state).await?.verify_checksums(&ids).map_err(err)
}

#[tauri::command]
pub async fn torrent_safety(state: State<'_, AppState>, id: String) -> Res<riptor_core::Safety> {
    engine(&state).await?.safety(&id).map_err(err)
}

#[tauri::command]
pub async fn vpn_leak_check(state: State<'_, AppState>) -> Res<riptor_core::LeakCheck> {
    engine(&state).await?.leak_check().await.map_err(err)
}
