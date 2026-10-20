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

/// One column of the torrent list, in display order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPref {
    pub key: String,
    pub visible: bool,
    #[serde(default)]
    pub width: Option<u32>,
}

const MAX_COLUMNS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub create_subfolder: bool,
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
    /// `socks5://[user:pass@]host:port` for peer connections.
    pub proxy_url: Option<String>,
    pub blocklist_url: Option<String>,
    /// No DHT/LSD/UPnP, generic client name.
    pub anonymous_mode: bool,
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
            proxy_url: None,
            blocklist_url: None,
            anonymous_mode: false,
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
}
