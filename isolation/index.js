// Tauri isolation hook: every IPC message from the UI passes through here,
// in a separate origin, before it reaches Rust. Anything that is not on the
// allowlist is dropped, so a compromised UI dependency cannot reach other
// commands or plugins.
const APP_COMMANDS = new Set([
  "app_version", "engine_status", "take_startup_items", "preview_torrent", "add_torrent",
  "stop_torrents", "resume_torrents", "force_start_torrents", "remove_torrents", "set_files",
  "torrent_files", "torrent_trackers", "torrent_peers", "set_category", "set_tags",
  "set_share_limits", "queue_move", "get_categories", "save_category", "delete_category",
  "get_settings", "update_settings", "set_alt_speed", "reveal_torrent", "open_web_url",
  "statistics", "create_backup", "inspect_backup", "restore_backup",
  "association_status", "make_default_app", "open_default_apps",
  "search_providers", "search_python_status", "search_start", "search_cancel", "search_set_enabled",
  "search_add_torznab", "search_remove_torznab", "search_preview_plugin", "search_install_plugin",
  "search_uninstall_plugin", "search_plugin_download", "search_catalog", "search_install_python",
  "search_remove_python", "search_options", "search_set_scrape",
]);
const PLUGIN_PREFIXES = ["plugin:event|", "plugin:window|", "plugin:webview|", "plugin:app|", "plugin:path|"];
const PLUGIN_COMMANDS = new Set(["plugin:dialog|open", "plugin:dialog|save", "plugin:dialog|message", "plugin:dialog|ask", "plugin:dialog|confirm"]);

window.__TAURI_ISOLATION_HOOK__ = (payload) => {
  const cmd = payload && payload.cmd;
  if (typeof cmd !== "string") throw new Error("blocked IPC message");
  if (APP_COMMANDS.has(cmd) || PLUGIN_COMMANDS.has(cmd) || PLUGIN_PREFIXES.some((p) => cmd.startsWith(p))) {
    return payload;
  }
  console.warn("Riptor isolation: blocked IPC command", cmd);
  throw new Error(`blocked IPC command: ${cmd}`);
};
