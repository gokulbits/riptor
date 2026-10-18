//! IPC surface. Every argument is validated again in `riptor-core`; nothing
//! here trusts the webview. Errors are flattened to strings for the UI.

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

/// Magnet links / .torrent paths passed on the command line at launch.
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
pub async fn torrent_peers(state: State<'_, AppState>, id: String) -> Res<serde_json::Value> {
    engine(&state).await?.peers(&id).map_err(err)
}

#[tauri::command]
pub async fn set_category(
    state: State<'_, AppState>,
    ids: Vec<String>,
    category: Option<String>,
) -> Res<()> {
    engine(&state)
        .await?
        .set_category(&ids, category)
        .map_err(err)
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

/// Reveal a torrent's folder. The path comes from the engine, never from the UI.
#[tauri::command]
pub async fn reveal_torrent(app: AppHandle, state: State<'_, AppState>, id: String) -> Res<()> {
    let path = engine(&state).await?.content_path(&id).map_err(err)?;
    if path.exists() {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string())
    } else {
        Err("the download folder does not exist yet".into())
    }
}

/// Open a web page (tracker or search result description) in the browser.
/// Only plain http(s) URLs are allowed.
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
    // Argon2 key derivation is deliberately slow; keep it off the async runtime.
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
    search(&state)?
        .start(&query, &category, providers, sink)
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

/// Fetch a plugin result's .torrent through the plugin; returns a temp file
/// path for the add dialog.
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
