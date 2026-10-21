use std::collections::BTreeMap;
use std::path::PathBuf;

use riptor_seeding::{Intent, ShareLimitAction, ShareLimits};
use serde::{Deserialize, Serialize};

pub use crate::suspicion::{Finding, Level};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub info_hash: String,
    pub name: String,
    pub root: PathBuf,
    /// We created `root`, so we may remove it.
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
    /// KiB/s; `None` = no limit of its own.
    #[serde(default)]
    pub dl_limit_kib: Option<u32>,
    #[serde(default)]
    pub ul_limit_kib: Option<u32>,
    /// Where the files go once complete (downloads start in the incomplete folder).
    #[serde(default)]
    pub move_on_complete: Option<PathBuf>,
    /// Files not listed are normal priority.
    #[serde(default)]
    pub file_priority: BTreeMap<usize, FilePriority>,
    /// Files the user renamed: index -> relative path.
    #[serde(default)]
    pub renames: BTreeMap<usize, String>,
    /// The last check against checksum files in the torrent.
    #[serde(default)]
    pub checksums: Option<crate::checksums::ChecksumReport>,
    /// Unix time the data on disk was last re-checked in the background.
    #[serde(default)]
    pub last_scrub: Option<u64>,
    /// Disguised names in the file list; `None` until checked.
    #[serde(default)]
    pub findings: Option<Vec<Finding>>,
    /// Finished files that aren't what their names say; `None` until checked.
    #[serde(default)]
    pub content_findings: Option<Vec<Finding>>,
}

impl Record {
    pub fn suspicion(&self) -> Option<Level> {
        self.findings
            .iter()
            .chain(&self.content_findings)
            .filter_map(|f| crate::suspicion::worst(f))
            .max()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePriority {
    Low,
    #[default]
    Normal,
    High,
}

impl FilePriority {
    pub fn level(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub torrents: Vec<Record>,
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
    /// Held until its drive has room again.
    NoSpace,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentSummary {
    pub id: String,
    pub name: String,
    pub state: TorrentState,
    pub forced: bool,
    pub progress: f64,
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
    /// Host of the first tracker, alphabetically.
    pub tracker: Option<String>,
    pub added_on: u64,
    pub completed_on: Option<u64>,
    pub seeding_time: u64,
    pub queue_pos: Option<u32>,
    pub limits: ShareLimits,
    pub limit_action: Option<ShareLimitAction>,
    pub has_executables: bool,
    pub dl_limit_kib: Option<u32>,
    pub ul_limit_kib: Option<u32>,
    pub error: Option<String>,
    /// Checksum files: every listed file matches (true), some don't (false), or nothing checked.
    pub checksum: Option<bool>,
    /// Looks like a fake release.
    pub suspicion: Option<Level>,
    /// The whole swarm, as trackers report it (we count as a seeder too).
    pub swarm_seeds: Option<u32>,
    pub swarm_leechers: Option<u32>,
    /// Background data check in progress (0-1).
    pub scrub: Option<f32>,
    pub last_scrub: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalStats {
    /// "normal", "vpn" or "blocked".
    pub network: String,
    pub dl_rate: u64,
    pub ul_rate: u64,
    pub dht_nodes: Option<usize>,
    pub listen_port: Option<u16>,
    pub alt_speed: bool,
    pub read_cache: riptor_storage::cache::CacheStats,
    pub write_buffer_used: usize,
    /// Why torrents are waiting (metered connection, data cap), if they are.
    pub hold: Option<String>,
    pub data_cap: Option<CapUsage>,
    /// The kill switch is on only because this network isn't trusted.
    pub untrusted_network: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CapUsage {
    pub used: u64,
    pub limit: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub index: usize,
    /// Relative to the torrent root, sanitized.
    pub path: String,
    pub size: u64,
    pub done: u64,
    pub selected: bool,
    pub executable: bool,
    #[serde(default)]
    pub priority: FilePriority,
    /// Bytes on disk, unbroken, from the start of the file.
    #[serde(default)]
    pub ready: u64,
    /// Audio or video: can be opened before it's complete.
    #[serde(default)]
    pub media: bool,
    /// An image or text file Riptor can show itself.
    #[serde(default)]
    pub previewable: bool,
}

/// The VPN leak check's findings.
#[derive(Clone, Debug, Serialize)]
pub struct LeakCheck {
    /// What trackers and peers see: Riptor's sockets are pinned to the tunnel.
    pub vpn_ip: Option<String>,
    /// The rest of the computer's address.
    pub normal_ip: Option<String>,
    pub tunnel_dns: bool,
    pub adapter: String,
    pub kill_switch: bool,
    pub passed: bool,
    pub notes: Vec<String>,
}

/// Which parts are on disk, squashed into `buckets` (0-255 each).
#[derive(Clone, Debug, Serialize)]
pub struct PieceMap {
    pub total: usize,
    pub have: usize,
    pub buckets: Vec<u8>,
}

impl PieceMap {
    pub fn new(have: &[bool], buckets: usize) -> Self {
        let total = have.len();
        let n = buckets.clamp(1, 1000).min(total.max(1));
        let buckets = (0..n)
            .map(|b| {
                let (lo, hi) = (
                    b * total / n,
                    ((b + 1) * total / n).max(b * total / n + 1).min(total),
                );
                let span = &have[lo.min(total)..hi];
                if span.is_empty() {
                    return 0;
                }
                (span.iter().filter(|x| **x).count() * 255 / span.len()) as u8
            })
            .collect();
        Self {
            total,
            have: have.iter().filter(|x| **x).count(),
            buckets,
        }
    }
}

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
    /// Signs of a fake release; a dangerous one is added stopped unless allowed.
    pub findings: Vec<Finding>,
    /// Files other torrents already downloaded, proven by this torrent's piece hashes.
    pub duplicates: Vec<Duplicate>,
    pub duplicate_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Duplicate {
    pub index: usize,
    /// Where it already is.
    pub existing: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AddParams {
    pub info_hash: String,
    /// `None`: category folder, else the default.
    pub save_path: Option<PathBuf>,
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub start_paused: bool,
    /// `None` = all.
    pub only_files: Option<Vec<usize>>,
    /// Overrides the global "create subfolder" setting.
    pub create_subfolder: Option<bool>,
    /// The user saw the warnings; otherwise a dangerous-looking torrent is added stopped.
    #[serde(default)]
    pub allow_suspicious: bool,
    /// Link or copy files other torrents already have instead of downloading them.
    #[serde(default)]
    pub reuse_existing: bool,
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

/// What Riptor knows about whether a torrent's contents can be trusted.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Safety {
    /// It has checksum files (SHA256SUMS, .sfv…).
    pub has_manifest: bool,
    pub checksums: Option<crate::checksums::ChecksumReport>,
    /// Trackers whose URL carries a personal passkey.
    pub secret_trackers: usize,
    pub private: bool,
    /// Signs of a fake release, in names and (once finished) in contents.
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MagnetLink {
    pub link: String,
    /// Trackers left out because they carry a passkey.
    pub withheld: usize,
    /// Only works for members of its tracker.
    pub private: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicIp {
    pub ip: String,
    pub via_vpn: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_map_squashes_pieces_into_buckets() {
        let have = [true, true, false, false, true, false, true, true];
        let m = PieceMap::new(&have, 4);
        assert_eq!((m.total, m.have), (8, 5));
        assert_eq!(m.buckets, vec![255, 0, 127, 255]);
        // Never more buckets than pieces, and an empty torrent is fine.
        assert_eq!(PieceMap::new(&have, 100).buckets.len(), 8);
        assert_eq!(PieceMap::new(&[], 10).buckets, vec![0]);
    }
}
