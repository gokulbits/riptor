use std::path::PathBuf;

use riptor_seeding::Prefs as SeedingPrefs;
use riptor_storage::StorageConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// uTP is still experimental in librqbit.
    #[default]
    Tcp,
    TcpAndUtp,
}

/// A folder whose `.torrent` files are added automatically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedFolder {
    pub path: PathBuf,
    #[serde(default)]
    pub category: Option<String>,
    /// `None`: the category's folder, else the default.
    #[serde(default)]
    pub save_path: Option<PathBuf>,
}

const MAX_WATCHED: usize = 20;

/// One column of the torrent list, in display order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPref {
    pub key: String,
    pub visible: bool,
    #[serde(default)]
    pub width: Option<u32>,
}

const MAX_COLUMNS: usize = 32;
const MAX_EXTRA_TRACKERS: usize = 100;

/// http(s) or udp, no credentials or odd characters.
pub fn valid_tracker(t: &str) -> bool {
    t.len() <= 512
        && !t.chars().any(|c| c.is_whitespace() || c.is_control())
        && url::Url::parse(t).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https" | "udp")
                && u.host_str().is_some()
                && u.password().is_none()
        })
}

/// When the alternative speed limits switch on by themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AltSchedule {
    pub enabled: bool,
    /// Minutes after midnight, local time. `from == to` means all day.
    pub from: u16,
    pub to: u16,
    /// Bit 0 = Monday ... bit 6 = Sunday.
    pub days: u8,
}

impl Default for AltSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            from: 8 * 60,
            to: 20 * 60,
            days: 0x7f,
        }
    }
}

impl AltSchedule {
    /// Monday = 0. Overnight windows count for the day they start.
    pub fn active(&self, weekday: u32, minute: u16) -> bool {
        let on = |d: u32| self.days & (1 << (d % 7)) != 0;
        let yesterday = (weekday + 6) % 7;
        match self.from.cmp(&self.to) {
            std::cmp::Ordering::Equal => on(weekday),
            std::cmp::Ordering::Less => on(weekday) && (self.from..self.to).contains(&minute),
            std::cmp::Ordering::Greater => {
                (on(weekday) && minute >= self.from) || (on(yesterday) && minute < self.to)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub create_subfolder: bool,
    /// Changing a torrent's category moves its files to that category's folder.
    pub move_on_category_change: bool,
    /// 0 = random.
    pub listen_port: u16,
    /// `None` = all interfaces.
    pub listen_address: Option<std::net::IpAddr>,
    pub transport: Transport,
    pub upnp: bool,
    pub dht: bool,
    pub lsd: bool,
    pub peer_limit: Option<usize>,
    /// KiB/s; `None` = unlimited.
    pub dl_limit_kib: Option<u32>,
    pub ul_limit_kib: Option<u32>,
    pub alt_dl_limit_kib: Option<u32>,
    pub alt_ul_limit_kib: Option<u32>,
    pub alt_speed_enabled: bool,
    pub alt_schedule: AltSchedule,
    /// Older single folder; `migrate` moves it into `watched_folders`.
    pub watched_folder: Option<PathBuf>,
    pub watched_folders: Vec<WatchedFolder>,
    /// Downloads live here until complete, then move to their save folder.
    pub incomplete_dir: Option<PathBuf>,
    /// Added to every new torrent that isn't private.
    pub extra_trackers: Vec<String>,
    /// New torrents start stopped (the add dialog's default, the watched folder, RSS).
    pub add_stopped: bool,
    /// Look for a new version shortly after start (Windows).
    pub check_updates: bool,
    /// Pause downloads when their drive has less free; `None` = never.
    pub min_free_mib: Option<u64>,
    /// Used instead of `blocklist_url` when set.
    pub blocklist_file: Option<PathBuf>,
    /// Peers banned by hand, for every torrent.
    pub banned_ips: Vec<std::net::IpAddr>,
    /// `socks5://[user:pass@]host:port` for peer connections.
    pub proxy_url: Option<String>,
    pub blocklist_url: Option<String>,
    /// No DHT/LSD/UPnP, generic client name.
    pub anonymous_mode: bool,
    /// Download from the HTTP servers a torrent lists (BEP 19).
    pub web_seeds: bool,
    pub mark_of_the_web: bool,
    /// Kill switch.
    pub vpn_required: bool,
    /// Closing the window hides it; Quit is in the tray menu.
    pub close_to_tray: bool,
    /// Empty means the default layout.
    pub columns: Vec<ColumnPref>,
    pub storage: StorageConfig,
    pub seeding: SeedingPrefs,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            create_subfolder: true,
            move_on_category_change: false,
            listen_port: 0,
            listen_address: None,
            transport: Transport::Tcp,
            upnp: true,
            dht: true,
            lsd: true,
            peer_limit: Some(128),
            dl_limit_kib: None,
            ul_limit_kib: None,
            alt_dl_limit_kib: Some(1024),
            alt_ul_limit_kib: Some(256),
            alt_speed_enabled: false,
            alt_schedule: AltSchedule::default(),
            watched_folder: None,
            watched_folders: Vec::new(),
            incomplete_dir: None,
            extra_trackers: Vec::new(),
            add_stopped: false,
            check_updates: true,
            min_free_mib: Some(1024),
            blocklist_file: None,
            banned_ips: Vec::new(),
            proxy_url: None,
            blocklist_url: None,
            anonymous_mode: false,
            web_seeds: true,
            mark_of_the_web: true,
            vpn_required: false,
            close_to_tray: true,
            columns: Vec::new(),
            storage: StorageConfig::default(),
            seeding: SeedingPrefs::default(),
        }
    }
}

impl Settings {
    /// Brings settings from older versions up to date.
    pub fn migrate(&mut self) {
        if let Some(p) = self.watched_folder.take() {
            if !self.watched_folders.iter().any(|w| w.path == p) {
                self.watched_folders.insert(
                    0,
                    WatchedFolder {
                        path: p,
                        category: None,
                        save_path: None,
                    },
                );
            }
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.download_dir.is_absolute(),
            "download folder must be an absolute path"
        );
        if let Some(p) = &self.proxy_url {
            let u = url::Url::parse(p)?;
            anyhow::ensure!(
                matches!(u.scheme(), "socks5"),
                "only socks5:// proxies are supported"
            );
        }
        if let Some(b) = &self.blocklist_url {
            let u = url::Url::parse(b)?;
            anyhow::ensure!(
                u.scheme() == "https",
                "blocklists must be fetched over https"
            );
        }
        anyhow::ensure!(
            self.storage.read_cache_bytes <= 4 << 30 && self.storage.write_buffer_bytes <= 4 << 30,
            "cache sizes above 4 GiB are not allowed"
        );
        anyhow::ensure!(
            (16..=65536).contains(&self.storage.max_open_files),
            "open file limit out of range"
        );
        let a = &self.alt_schedule;
        anyhow::ensure!(
            a.from < 24 * 60 && a.to < 24 * 60 && a.days <= 0x7f,
            "invalid speed schedule"
        );
        if let Some(w) = &self.watched_folder {
            anyhow::ensure!(w.is_absolute(), "watched folder must be an absolute path");
        }
        anyhow::ensure!(
            self.watched_folders.len() <= MAX_WATCHED,
            "too many watched folders"
        );
        let mut seen = std::collections::HashSet::new();
        for w in &self.watched_folders {
            anyhow::ensure!(
                w.path.is_absolute(),
                "watched folders must be absolute paths"
            );
            anyhow::ensure!(seen.insert(&w.path), "{} is listed twice", w.path.display());
            if let Some(p) = &w.save_path {
                anyhow::ensure!(p.is_absolute(), "save folders must be absolute paths");
            }
            if let Some(c) = &w.category {
                anyhow::ensure!(
                    !c.trim().is_empty()
                        && c.len() <= 64
                        && !c
                            .chars()
                            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\')),
                    "invalid category for a watched folder"
                );
            }
        }
        if let Some(d) = &self.incomplete_dir {
            anyhow::ensure!(
                d.is_absolute(),
                "incomplete downloads folder must be an absolute path"
            );
        }
        anyhow::ensure!(
            self.extra_trackers.len() <= MAX_EXTRA_TRACKERS,
            "too many trackers"
        );
        anyhow::ensure!(
            self.min_free_mib.is_none_or(|m| m <= 1024 * 1024),
            "free space limit above 1 TiB"
        );
        if let Some(f) = &self.blocklist_file {
            anyhow::ensure!(
                f.is_absolute(),
                "the blocklist file must be an absolute path"
            );
        }
        anyhow::ensure!(self.banned_ips.len() <= 10_000, "too many banned peers");
        for t in &self.extra_trackers {
            anyhow::ensure!(valid_tracker(t), "not a tracker URL: {t}");
        }
        anyhow::ensure!(self.columns.len() <= MAX_COLUMNS, "too many columns");
        let mut seen = std::collections::HashSet::new();
        for c in &self.columns {
            anyhow::ensure!(
                (1..=32).contains(&c.key.len())
                    && c.key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "invalid column"
            );
            anyhow::ensure!(seen.insert(c.key.as_str()), "duplicate column");
            anyhow::ensure!(
                c.width.is_none_or(|w| (40..=2000).contains(&w)),
                "column width out of range"
            );
        }
        Ok(())
    }

    pub fn effective_limits(&self) -> (Option<u32>, Option<u32>) {
        if self.alt_speed_enabled {
            (self.alt_dl_limit_kib, self.alt_ul_limit_kib)
        } else {
            (self.dl_limit_kib, self.ul_limit_kib)
        }
    }

    pub fn needs_restart(&self, other: &Settings) -> bool {
        self.listen_port != other.listen_port
            || self.listen_address != other.listen_address
            || self.transport != other.transport
            || self.upnp != other.upnp
            || self.dht != other.dht
            || self.lsd != other.lsd
            || self.peer_limit != other.peer_limit
            || self.proxy_url != other.proxy_url
            || self.blocklist_url != other.blocklist_url
            || self.blocklist_file != other.blocklist_file
            || self.anonymous_mode != other.anonymous_mode
    }
}

fn default_download_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Downloads")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        let mut s = Settings {
            download_dir: std::env::temp_dir(),
            ..Settings::default()
        };
        s.validate().unwrap();
        s.proxy_url = Some("http://evil".into());
        assert!(s.validate().is_err());
        s.proxy_url = Some("socks5://127.0.0.1:1080".into());
        s.validate().unwrap();
        s.blocklist_url = Some("http://plain.example/list".into());
        assert!(s.validate().is_err());
        s.blocklist_url = None;
        s.extra_trackers = vec!["udp://tracker.example:1337/announce".into()];
        s.validate().unwrap();
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "udp://a b/announce",
            "http://user:pass@tracker.example/announce",
            "udp://",
        ] {
            s.extra_trackers = vec![bad.into()];
            assert!(s.validate().is_err(), "{bad}");
        }
        s.extra_trackers = vec!["udp://t.example:1/a".into(); 101];
        assert!(s.validate().is_err());
        s.extra_trackers.clear();
        s.incomplete_dir = Some(PathBuf::from("relative"));
        assert!(s.validate().is_err());
        s.incomplete_dir = None;
        s.download_dir = PathBuf::from("relative");
        assert!(s.validate().is_err());
    }

    #[test]
    fn columns_are_validated() {
        let col = |key: &str, width| ColumnPref {
            key: key.into(),
            visible: true,
            width,
        };
        let mut s = Settings {
            download_dir: std::env::temp_dir(),
            columns: vec![col("name", Some(380)), col("size", None)],
            ..Settings::default()
        };
        s.validate().unwrap();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.columns, s.columns);

        for bad in [
            vec![col("name", None), col("name", None)],
            vec![col("<img onerror=x>", None)],
            vec![col("", None)],
            vec![col("name", Some(5))],
            vec![col("name", Some(1_000_000))],
            (0..40)
                .map(|i| col(&"a".repeat(i % 30 + 1), None))
                .collect(),
        ] {
            s.columns = bad;
            assert!(s.validate().is_err(), "{:?}", s.columns);
        }
    }

    #[test]
    fn partial_json_uses_defaults() {
        let s: Settings = serde_json::from_str(r#"{"listen_port": 6881}"#).unwrap();
        assert_eq!(s.listen_port, 6881);
        assert!(s.dht);
        assert!(s.columns.is_empty());
    }

    #[test]
    fn speed_schedule_windows() {
        let day = |from, to, days| AltSchedule {
            enabled: true,
            from,
            to,
            days,
        };
        // Weekdays 08:00-20:00.
        let w = day(8 * 60, 20 * 60, 0b0011111);
        assert!(w.active(0, 8 * 60));
        assert!(!w.active(0, 20 * 60));
        assert!(!w.active(5, 12 * 60));
        // Friday night 22:00 into Saturday 06:00 only.
        let n = day(22 * 60, 6 * 60, 1 << 4);
        assert!(n.active(4, 23 * 60));
        assert!(n.active(5, 60));
        assert!(!n.active(5, 23 * 60));
        assert!(!n.active(4, 60));
        // from == to: all of every chosen day.
        let all = day(0, 0, 1 << 6);
        assert!(all.active(6, 0) && all.active(6, 1439) && !all.active(0, 600));

        let mut s = Settings::default();
        s.alt_schedule.to = 24 * 60;
        assert!(s.validate().is_err());
        let mut s = Settings::default();
        s.alt_schedule.days = 0x80;
        assert!(s.validate().is_err());
        let s = Settings {
            watched_folder: Some("relative".into()),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn the_old_watched_folder_moves_into_the_list() {
        let dir = std::env::temp_dir();
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "download_dir": dir,
            "watched_folder": dir.join("in"),
        }))
        .unwrap();
        s.migrate();
        assert_eq!(s.watched_folder, None);
        assert_eq!(s.watched_folders.len(), 1);
        assert_eq!(s.watched_folders[0].path, dir.join("in"));
        s.validate().unwrap();
        let w = |p: &str| WatchedFolder {
            path: p.into(),
            category: None,
            save_path: None,
        };
        s.watched_folders.push(s.watched_folders[0].clone());
        assert!(s.validate().is_err(), "duplicates");
        s.watched_folders = vec![w("relative")];
        assert!(s.validate().is_err());
        s.watched_folders = vec![WatchedFolder {
            category: Some("a/b".into()),
            ..s.watched_folders[0].clone()
        }];
        assert!(s.validate().is_err());
    }
}
