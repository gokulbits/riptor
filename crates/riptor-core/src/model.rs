//! Types exchanged with the UI and persisted between runs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use riptor_seeding::{Intent, ShareLimitAction, ShareLimits};
use serde::{Deserialize, Serialize};

/// Per-torrent state Riptor keeps on top of librqbit's own persistence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub info_hash: String,
    pub name: String,
    /// Folder the torrent's files live under (see `owned_root`).
    pub root: PathBuf,
    /// `root` was created by Riptor for this torrent (multi-file subfolder).
    pub owned_root: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub added_on: u64,
    #[serde(default)]
    pub completed_on: Option<u64>,
    #[serde(default)]
    pub all_time_upload: u64,
    #[serde(default)]
    pub all_time_download: u64,
    /// Seconds spent running while complete.
    #[serde(default)]
    pub seeding_time: u64,
    /// Seconds spent running.
    #[serde(default)]
    pub active_time: u64,
    /// Unix time of the last payload transfer.
    #[serde(default)]
    pub last_activity: u64,
    #[serde(default)]
    pub limits: ShareLimits,
    #[serde(default)]
    pub limit_action: Option<ShareLimitAction>,
    #[serde(default)]
    pub queue_pos: Option<u32>,
    pub intent: Intent,
    #[serde(default)]
    pub has_executables: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub torrents: Vec<Record>,
    /// Category name -> optional default save folder.
    #[serde(default)]
    pub categories: BTreeMap<String, Option<PathBuf>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentState {
    Metadata,
    Checking,
    Downloading,
    StalledDl,
    Seeding,
    StalledUp,
    Queued,
    Stopped,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentSummary {
    pub id: String,
    pub name: String,
    pub state: TorrentState,
    pub forced: bool,
    /// 0.0 ..= 1.0 of the selected files.
    pub progress: f64,
    /// Bytes of the selected files.
    pub size: u64,
    pub done: u64,
    pub total_size: u64,
    pub dl_rate: u64,
    pub ul_rate: u64,
    pub eta: Option<u64>,
    pub ratio: f64,
    pub uploaded: u64,
    pub downloaded: u64,
    pub peers: u32,
    pub peers_seen: u32,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub save_path: String,
    pub added_on: u64,
    pub completed_on: Option<u64>,
    pub seeding_time: u64,
    pub queue_pos: Option<u32>,
    pub limits: ShareLimits,
    pub limit_action: Option<ShareLimitAction>,
    pub has_executables: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalStats {
    /// "normal", "vpn" or "blocked" (kill switch engaged).
    pub network: String,
    pub dl_rate: u64,
    pub ul_rate: u64,
    pub dht_nodes: Option<usize>,
    pub listen_port: Option<u16>,
    pub alt_speed: bool,
    pub read_cache: riptor_storage::cache::CacheStats,
    pub write_buffer_used: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub index: usize,
    /// Path relative to the torrent root, as it will appear on disk (sanitized).
    pub path: String,
    pub size: u64,
    pub done: u64,
    pub selected: bool,
    pub executable: bool,
}

/// What the add dialog shows before the user commits.
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub piece_length: u32,
    pub multi_file: bool,
    pub private: bool,
    pub files: Vec<FileEntry>,
    pub has_executables: bool,
    pub already_added: bool,
    pub trackers: Vec<String>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AddParams {
    pub info_hash: String,
    /// `None` = category folder or the default download folder.
    pub save_path: Option<PathBuf>,
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub start_paused: bool,
    /// File indices to download; `None` = all.
    pub only_files: Option<Vec<usize>>,
    /// Overrides the global "create subfolder" setting.
    pub create_subfolder: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Tick {
        torrents: Vec<TorrentSummary>,
        global: GlobalStats,
    },
    Finished {
        id: String,
        name: String,
    },
    Removed {
        id: String,
    },
    Notice {
        level: NoticeLevel,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}
