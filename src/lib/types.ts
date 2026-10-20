// Must match the serde shapes in Rust.

export type Limit = { mode: "use_global" } | { mode: "unlimited" } | { mode: "value"; value: number };

export interface ShareLimits {
  ratio: Limit;
  seeding_time: Limit;
  inactive_seeding_time: Limit;
}

export type ShareLimitAction = "stop" | "remove" | "remove_with_files";

export type TorrentState =
  | "metadata"
  | "checking"
  | "downloading"
  | "stalled_dl"
  | "seeding"
  | "stalled_up"
  | "queued"
  | "stopped"
  | "error";

export interface TorrentSummary {
  id: string;
  name: string;
  state: TorrentState;
  forced: boolean;
  progress: number;
  size: number;
  done: number;
  total_size: number;
  dl_rate: number;
  ul_rate: number;
  eta: number | null;
  ratio: number;
  uploaded: number;
  downloaded: number;
  peers: number;
  peers_seen: number;
  category: string | null;
  tags: string[];
  save_path: string;
  tracker: string | null;
  added_on: number;
  completed_on: number | null;
  seeding_time: number;
  queue_pos: number | null;
  limits: ShareLimits;
  limit_action: ShareLimitAction | null;
  has_executables: boolean;
  dl_limit_kib: number | null;
  ul_limit_kib: number | null;
  error: string | null;
}

export interface CacheStats {
  bytes: number;
  capacity: number;
  hits: number;
  misses: number;
}

export interface GlobalStats {
  network: "normal" | "vpn" | "blocked";
  dl_rate: number;
  ul_rate: number;
  dht_nodes: number | null;
  listen_port: number | null;
  alt_speed: boolean;
  read_cache: CacheStats;
  write_buffer_used: number;
}

export interface FileEntry {
  index: number;
  path: string;
  size: number;
  done: number;
  selected: boolean;
  executable: boolean;
  priority: FilePriority;
}

export type FilePriority = "low" | "normal" | "high";

export interface Preview {
  info_hash: string;
  name: string;
  total_size: number;
  piece_length: number;
  multi_file: boolean;
  private: boolean;
  files: FileEntry[];
  has_executables: boolean;
  already_added: boolean;
  trackers: string[];
  comment: string | null;
}

export interface AddParams {
  info_hash: string;
  save_path?: string | null;
  category?: string | null;
  tags?: string[];
  start_paused?: boolean;
  only_files?: number[] | null;
  create_subfolder?: boolean | null;
}

export type Source = { kind: "magnet"; value: string } | { kind: "url"; value: string } | { kind: "file"; value: string };

export type NoticeLevel = "info" | "warning" | "error";

export type EngineEvent =
  | { type: "tick"; torrents: TorrentSummary[]; global: GlobalStats }
  | { type: "finished"; id: string; name: string }
  | { type: "removed"; id: string }
  | { type: "notice"; level: NoticeLevel; message: string };

export interface QueuePrefs {
  enabled: boolean;
  max_active_downloads: number | null;
  max_active_uploads: number | null;
  max_active_torrents: number | null;
  dont_count_slow: boolean;
  dl_rate_threshold: number;
  ul_rate_threshold: number;
  inactivity_timer_secs: number;
}

export interface SeedingPrefs {
  max_ratio: number | null;
  max_seeding_minutes: number | null;
  max_inactive_seeding_minutes: number | null;
  limit_action: ShareLimitAction;
  queue: QueuePrefs;
}

export interface StorageConfig {
  read_cache_bytes: number;
  write_buffer_bytes: number;
  max_open_files: number;
  preallocation: "sparse" | "full";
}

export interface ColumnPref {
  key: string;
  visible: boolean;
  width: number | null;
}

export interface LogLine {
  /** Unix milliseconds. */
  time: number;
  level: "error" | "warn" | "info" | "debug" | "trace";
  target: string;
  message: string;
}

export interface CreateOptions {
  trackers: string[];
  web_seeds: string[];
  private: boolean;
  comment: string | null;
  /** null picks one from the total size. */
  piece_length: number | null;
}

export interface AltSchedule {
  enabled: boolean;
  /** Minutes after midnight, local time. Equal values mean all day. */
  from: number;
  to: number;
  /** Bit 0 = Monday … bit 6 = Sunday. */
  days: number;
}

export interface Settings {
  download_dir: string;
  create_subfolder: boolean;
  move_on_category_change: boolean;
  listen_port: number;
  listen_address: string | null;
  transport: "tcp" | "tcp_and_utp";
  upnp: boolean;
  dht: boolean;
  lsd: boolean;
  peer_limit: number | null;
  dl_limit_kib: number | null;
  ul_limit_kib: number | null;
  alt_dl_limit_kib: number | null;
  alt_ul_limit_kib: number | null;
  alt_speed_enabled: boolean;
  alt_schedule: AltSchedule;
  watched_folder: string | null;
  incomplete_dir: string | null;
  extra_trackers: string[];
  add_stopped: boolean;
  proxy_url: string | null;
  blocklist_url: string | null;
  anonymous_mode: boolean;
  web_seeds: boolean;
  mark_of_the_web: boolean;
  /** Kill switch. */
  vpn_required: boolean;
  close_to_tray: boolean;
  columns: ColumnPref[];
  storage: StorageConfig;
  seeding: SeedingPrefs;
}

export type QueueMove = "up" | "down" | "top" | "bottom";

export interface DayTotals {
  uploaded: number;
  downloaded: number;
}

export interface Statistics {
  all_time_upload: number;
  all_time_download: number;
  all_time_wasted: number;
  torrents_added: number;
  torrents_completed: number;
  torrents_removed: number;
  total_uptime_secs: number;
  sessions: number;
  first_started: number;
  peak_download_rate: number;
  peak_upload_rate: number;
  daily: Record<string, DayTotals>;
}

export interface DiskMetrics {
  pieces_flushed: number;
  spills: number;
  bytes_written: number;
  bytes_read: number;
  disk_writes: number;
  disk_reads: number;
  avg_flush_us: number;
}

export interface StatsReport {
  all_time: Statistics;
  all_time_ratio: number;
  session_upload: number;
  session_download: number;
  session_wasted: number;
  session_ratio: number;
  session_uptime_secs: number;
  today: DayTotals;
  last_7_days: DayTotals;
  last_30_days: DayTotals;
  dl_rate: number;
  ul_rate: number;
  torrents_total: number;
  torrents_downloading: number;
  torrents_seeding: number;
  torrents_completed: number;
  torrents_stopped: number;
  torrents_queued: number;
  torrents_errored: number;
  total_size: number;
  total_seeding_secs: number;
  peers_connected: number;
  peers_known: number;
  blocked_incoming: number;
  blocked_outgoing: number;
  listen_port: number | null;
  read_cache: CacheStats;
  write_buffer_used: number;
  write_buffer_capacity: number;
  disk: DiskMetrics;
}

export interface BackupSummary {
  created: number;
  app_version: string;
  password_protected: boolean;
  torrents: number;
  torrent_names: string[];
  save_folders: string[];
  categories: number;
  all_time_upload: number;
  all_time_download: number;
}

export interface RestoreOptions {
  settings: boolean;
  torrents: boolean;
  statistics: boolean;
  resume_torrents: boolean;
}

export interface RestoreReport {
  added: number;
  already_present: number;
  settings_restored: boolean;
  statistics_restored: boolean;
  skipped: string[];
}

export interface AssociationStatus {
  registered: boolean;
  torrent_default: boolean;
  magnet_default: boolean;
}

export interface SearchResult {
  name: string;
  link: string;
  size: number | null;
  seeders: number | null;
  leechers: number | null;
  desc_link: string | null;
  pub_date: number | null;
  engine: string;
  via_plugin: boolean;
  /** UI only. */
  scraped?: boolean;
}

export interface ProviderInfo {
  id: string;
  name: string;
  kind: "builtin" | "torznab" | "plugin";
  url: string;
  enabled: boolean;
  categories: string[];
  error: string | null;
}

export type SearchEvent =
  | { type: "results"; search_id: number; provider: string; results: SearchResult[] }
  | { type: "provider_done"; search_id: number; provider: string; count: number; error: string | null }
  | { type: "finished"; search_id: number }
  | { type: "counts"; search_id: number; counts: ResultCounts[] }
  | { type: "lookups_done"; search_id: number };

export interface ResultCounts {
  link: string;
  seeders: number;
  leechers: number;
}

export interface SearchOptions {
  scrape: boolean;
}

export interface CatalogEntry {
  id: string;
  name: string;
  site: string | null;
  download_url: string;
  origin: "official" | "community";
  private: boolean;
  version: string | null;
  author: string | null;
  health: "working" | "issues" | "broken" | "unknown";
  note: string | null;
  problem: string | null;
  installed: boolean;
}

export interface PluginCatalog {
  entries: CatalogEntry[];
  errors: string[];
}

export interface PythonStatus {
  available: boolean;
  version: string | null;
  sandboxed: boolean;
  bundled: boolean;
  bundled_installed: boolean;
  can_bundle: boolean;
  bundle_version: string;
  bundle_size: number | null;
}

export interface PluginPreview {
  id: string;
  sha256: string;
  size: number;
  source: string;
  declared_name: string | null;
  declared_url: string | null;
  replaces_existing: boolean;
}

export type VpnPhase = "disconnected" | "connecting" | "connected" | "reconnecting" | "error";

export interface VpnStatus {
  phase: VpnPhase;
  profile_id: string | null;
  profile_name: string | null;
  local_ip: string | null;
  forwarded_port: number | null;
  message: string | null;
}

export interface VpnProfile {
  id: string;
  name: string;
  kind: "open_vpn" | "wire_guard";
  servers: string[];
  removed: string[];
  needs_credentials: boolean;
  has_credentials: boolean;
  app_only_supported: boolean;
  created: number;
}

export interface PublicIp {
  ip: string;
  via_vpn: boolean;
}

export interface VpnPrefs {
  port_forwarding: boolean;
  auto_connect: boolean;
  last_profile: string | null;
}

export interface RssOptions {
  auto_download: boolean;
  refresh_minutes: number;
}

export interface RssFeed {
  id: number;
  url: string;
  name: string;
  last_refresh: number | null;
  error: string | null;
  unread: number;
  total: number;
}

export interface RssRule {
  id: number;
  name: string;
  enabled: boolean;
  must_contain: string;
  must_not_contain: string;
  use_regex: boolean;
  feeds: number[];
  category: string | null;
  save_path: string | null;
  start_paused: boolean;
  last_match: number | null;
}

export interface RssView {
  options: RssOptions;
  feeds: RssFeed[];
  rules: RssRule[];
}

export interface RssArticle {
  id: string;
  title: string;
  torrent: string;
  page: string | null;
  date: number | null;
  size: number | null;
  read: boolean;
  downloaded: boolean;
}

export interface RssRuleMatch {
  feed: string;
  title: string;
}
