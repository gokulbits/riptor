import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AddParams,
  AssociationStatus,
  BackupSummary,
  RestoreOptions,
  RestoreReport,
  StatsReport,
  ProviderInfo,
  PythonStatus,
  PluginPreview,
  PluginCatalog,
  SearchEvent,
  SearchOptions,
  EngineEvent,
  FileEntry,
  Preview,
  QueueMove,
  Settings,
  ShareLimitAction,
  ShareLimits,
  Source,
} from "./types";

export const api = {
  engineStatus: () => invoke<void>("engine_status"),
  takeStartupItems: () => invoke<Source[]>("take_startup_items"),
  preview: (source: Source) => invoke<Preview>("preview_torrent", { source }),
  add: (params: AddParams) => invoke<string>("add_torrent", { params }),
  stop: (ids: string[]) => invoke<void>("stop_torrents", { ids }),
  resume: (ids: string[]) => invoke<void>("resume_torrents", { ids }),
  forceStart: (ids: string[]) => invoke<void>("force_start_torrents", { ids }),
  remove: (ids: string[], deleteFiles: boolean) => invoke<void>("remove_torrents", { ids, deleteFiles }),
  setFiles: (id: string, files: number[]) => invoke<void>("set_files", { id, files }),
  files: (id: string) => invoke<FileEntry[]>("torrent_files", { id }),
  trackers: (id: string) => invoke<string[]>("torrent_trackers", { id }),
  peers: (id: string) => invoke<unknown>("torrent_peers", { id }),
  setCategory: (ids: string[], category: string | null) => invoke<void>("set_category", { ids, category }),
  setTags: (ids: string[], tags: string[]) => invoke<void>("set_tags", { ids, tags }),
  setShareLimits: (ids: string[], limits: ShareLimits, action: ShareLimitAction | null) =>
    invoke<void>("set_share_limits", { ids, limits, action }),
  queueMove: (ids: string[], how: QueueMove) => invoke<void>("queue_move", { ids, how }),
  categories: () => invoke<Record<string, string | null>>("get_categories"),
  saveCategory: (name: string, savePath: string | null) => invoke<void>("save_category", { name, savePath }),
  deleteCategory: (name: string) => invoke<void>("delete_category", { name }),
  settings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<void>("update_settings", { settings }),
  setAltSpeed: (enabled: boolean) => invoke<void>("set_alt_speed", { enabled }),
  reveal: (id: string) => invoke<void>("reveal_torrent", { id }),
  openWebUrl: (url: string) => invoke<void>("open_web_url", { url }),
  version: () => invoke<string>("app_version"),
  statistics: () => invoke<StatsReport>("statistics"),
  createBackup: (path: string, password: string | null) => invoke<BackupSummary>("create_backup", { path, password }),
  inspectBackup: (path: string, password: string | null) => invoke<BackupSummary>("inspect_backup", { path, password }),
  restoreBackup: (path: string, password: string | null, options: RestoreOptions) =>
    invoke<RestoreReport>("restore_backup", { path, password, options }),
  associationStatus: () => invoke<AssociationStatus>("association_status"),
  makeDefaultApp: () => invoke<AssociationStatus>("make_default_app"),
  openDefaultApps: () => invoke<void>("open_default_apps"),
  searchProviders: () => invoke<ProviderInfo[]>("search_providers"),
  pythonStatus: (recheck: boolean) => invoke<PythonStatus>("search_python_status", { recheck }),
  searchStart: (query: string, category: string, providers: string[] | null) =>
    invoke<number>("search_start", { query, category, providers }),
  searchCancel: (id: number) => invoke<void>("search_cancel", { id }),
  searchSetEnabled: (id: string, enabled: boolean) => invoke<void>("search_set_enabled", { id, enabled }),
  addTorznab: (name: string, url: string, apiKey: string) => invoke<void>("search_add_torznab", { name, url, apiKey }),
  removeTorznab: (id: string) => invoke<void>("search_remove_torznab", { id }),
  previewPlugin: (source: string) => invoke<PluginPreview>("search_preview_plugin", { source }),
  installPlugin: (sha256: string) => invoke<void>("search_install_plugin", { sha256 }),
  uninstallPlugin: (id: string) => invoke<void>("search_uninstall_plugin", { id }),
  pluginDownload: (provider: string, url: string) => invoke<string>("search_plugin_download", { provider, url }),
  pluginCatalog: (refresh: boolean) => invoke<PluginCatalog>("search_catalog", { refresh }),
  installPython: () => invoke<PythonStatus>("search_install_python"),
  removePython: () => invoke<PythonStatus>("search_remove_python"),
  searchOptions: () => invoke<SearchOptions>("search_options"),
  searchSetScrape: (enabled: boolean) => invoke<void>("search_set_scrape", { enabled }),
};

export const events = {
  engine: (cb: (e: EngineEvent) => void): Promise<UnlistenFn> => listen<EngineEvent>("riptor://event", (e) => cb(e.payload)),
  ready: (cb: () => void) => listen("riptor://ready", () => cb()),
  externalAdd: (cb: (items: Source[]) => void) => listen<Source[]>("riptor://external-add", (e) => cb(e.payload)),
  toggleAltSpeed: (cb: () => void) => listen("riptor://toggle-alt-speed", () => cb()),
  search: (cb: (e: SearchEvent) => void) => listen<SearchEvent>("riptor://search", (e) => cb(e.payload)),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
