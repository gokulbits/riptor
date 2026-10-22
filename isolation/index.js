// Isolation hook: IPC calls not on this list never reach Rust.
const APP_COMMANDS = new Set([
  "app_version", "recent_logs", "create_torrent", "create_torrent_progress", "create_torrent_cancel",
  "engine_status", "take_startup_items", "preview_torrent", "add_torrent",
  "stop_torrents", "resume_torrents", "force_start_torrents", "remove_torrents", "set_files",
  "torrent_files", "torrent_trackers", "torrent_peers", "torrent_magnet", "free_space", "public_ip", "set_category", "move_torrents", "recheck_torrents", "set_tags",
  "set_share_limits", "set_speed_limits", "queue_move", "get_categories", "save_category", "delete_category",
  "get_settings", "update_settings", "set_alt_speed", "reveal_torrent", "open_web_url",
  "statistics", "create_backup", "inspect_backup", "restore_backup",
  "association_status", "make_default_app", "open_default_apps",
  "search_providers", "search_python_status", "search_start", "search_cancel", "search_set_enabled",
  "search_add_torznab", "search_remove_torznab", "search_preview_plugin", "search_install_plugin",
  "search_uninstall_plugin", "search_plugin_download", "search_catalog", "search_install_python",
  "search_remove_python", "search_options", "search_set_scrape",
  "vpn_profiles", "vpn_import", "vpn_delete", "vpn_set_credentials", "vpn_connect", "vpn_disconnect",
  "vpn_status", "vpn_get_prefs", "vpn_set_prefs", "vpn_openvpn_installed",
  "autostart_get", "autostart_set", "when_done_get", "when_done_set", "when_done_cancel", "update_supported", "update_check", "update_install", "update_download",
  "webui_state", "webui_set_password", "webui_new_api_key", "webui_remove_api_key", "webui_set_ddns_password",
  "set_file_priority", "export_torrent", "reveal_torrent_file", "open_torrent_file", "torrent_pieces", "rename_files", "set_trackers", "import_torrents", "import_scan", "vpn_leak_check", "reannounce", "add_peer", "ban_peer",
  "rss_view", "rss_articles", "rss_add_feed", "rss_remove_feed", "rss_rename_feed", "rss_mark_read",
  "rss_refresh", "rss_set_options", "rss_save_rule", "rss_delete_rule", "rss_preview_rule",
  "verify_checksums", "torrent_safety", "connections", "connections_clear", "swarm_view", "release_leak_hold", "network_info", "check_data", "preview_file", "about_info", "country_status",
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
