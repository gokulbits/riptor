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
pub async fn torrent_magnet(state: State<'_, AppState>, id: String) -> Res<String> {
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
pub async fn torrent_peers(state: State<'_, AppState>, id: String) -> Res<serde_json::Value> {
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
    // Lookups follow the torrents' network.
    if let Some(e) = state.engine.get() {
        use riptor_core::NetworkMode;
        use riptor_search::LookupRoute;
        s.route_lookups(match e.network() {
            NetworkMode::Normal => LookupRoute::Direct,
            NetworkMode::Blocked => LookupRoute::Held,
            NetworkMode::Vpn(b) => {
                let r = riptor_core::dns::TunnelResolver::new(
                    b.local_ip.into(),
                    &riptor_core::dns::SERVERS,
                );
                LookupRoute::Tunnel(riptor_search::scrape::Tunnel {
                    local: b.local_ip.into(),
                    lookup: r.hook(),
                })
            }
        });
    }
    s.start(&query, &category, providers, sink)
        .await
        .map_err(err)
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
    let p = search(&state)?
        .plugin_download(&provider, &url)
        .await
        .map_err(err)?;
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
