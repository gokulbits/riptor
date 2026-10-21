//! Peer countries: the list is fetched only while the setting is on, the way torrents connect.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{info, warn};

use super::{unix_now, Engine, Job};
use crate::geoip::{self, GeoDb};

const RETRY: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct GeoState {
    pub db: Option<Arc<GeoDb>>,
    pub error: Option<String>,
    pub tried: Option<Instant>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CountryStatus {
    pub enabled: bool,
    pub ready: bool,
    /// Unix time the list was downloaded.
    pub updated: Option<u64>,
    pub error: Option<String>,
    pub attribution: &'static str,
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Engine {
    /// From the ticker: loads or refreshes the list when it's wanted.
    pub(super) fn schedule_countries(&self) {
        if !self.settings.read().peer_countries {
            return;
        }
        let stale = modified(&geoip::cache_file(&self.data_dir))
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|age| age > geoip::REFRESH);
        let g = self.geo.lock();
        let due = (g.db.is_none() || stale) && g.tried.is_none_or(|t| t.elapsed() > RETRY);
        drop(g);
        if due {
            self.queue_job(Job::Countries);
        }
    }

    pub(super) async fn refresh_countries(&self) {
        if !self.settings.read().peer_countries {
            return;
        }
        self.geo.lock().tried = Some(Instant::now());
        let file = geoip::cache_file(&self.data_dir);
        let fresh = modified(&file)
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age <= geoip::REFRESH);
        if self.geo.lock().db.is_none() && file.is_file() {
            let f = file.clone();
            if let Ok(Ok(db)) = tokio::task::spawn_blocking(move || GeoDb::open(&f)).await {
                self.geo.lock().db = Some(Arc::new(db));
                if fresh {
                    return;
                }
            }
        }
        let first = geoip::urls(unix_now())[0].clone();
        let result = match self.web_client(&first) {
            Ok(http) => geoip::download(&http, &self.data_dir, unix_now()).await,
            Err(e) => Err(e),
        };
        // Turned off while downloading: keep nothing.
        if !self.settings.read().peer_countries {
            self.forget_countries();
            return;
        }
        let mut g = self.geo.lock();
        match result {
            Ok(db) => {
                info!("peer country list ready ({} ranges)", db.ranges());
                g.db = Some(Arc::new(db));
                g.error = None;
            }
            Err(e) => {
                warn!("couldn't download the peer country list: {e:#}");
                g.error = Some(format!("{e:#}"));
            }
        }
    }

    /// Off: the list leaves memory and disk.
    pub(super) fn forget_countries(&self) {
        *self.geo.lock() = GeoState::default();
        let _ = std::fs::remove_dir_all(geoip::cache_dir(&self.data_dir));
    }

    pub fn country_status(&self) -> CountryStatus {
        let g = self.geo.lock();
        CountryStatus {
            enabled: self.settings.read().peer_countries,
            ready: g.db.is_some(),
            updated: modified(&geoip::cache_file(&self.data_dir))
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            error: g.error.clone(),
            attribution: geoip::ATTRIBUTION,
        }
    }

    /// Adds `country` to each peer in librqbit's peer snapshot.
    pub(super) fn add_countries(&self, peers: &mut serde_json::Value) {
        if let Some(db) = self.geo.lock().db.clone() {
            add_countries(&db, peers);
        }
    }
}

fn add_countries(db: &GeoDb, peers: &mut serde_json::Value) {
    let Some(map) = peers.get_mut("peers").and_then(|p| p.as_object_mut()) else {
        return;
    };
    for (addr, peer) in map.iter_mut() {
        let ip = addr
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip())
            .or_else(|_| addr.parse::<std::net::IpAddr>());
        if let (Ok(ip), Some(obj)) = (ip, peer.as_object_mut()) {
            if let Some(cc) = db.lookup(ip) {
                obj.insert("country".into(), cc.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_get_a_country() {
        let db =
            GeoDb::parse(&b"1.0.0.0,1.0.0.255,AU\n2a00:1450::,2a00:1450:ffff::,IE\n"[..]).unwrap();
        let mut v = serde_json::json!({ "peers": {
            "1.0.0.7:6881": { "state": "live" },
            "[2a00:1450::1]:51413": { "state": "live" },
            "9.9.9.9:1": { "state": "live" },
            "not an address": { "state": "live" },
            "1.0.0.8:1": "not an object",
        }});
        add_countries(&db, &mut v);
        let p = &v["peers"];
        assert_eq!(p["1.0.0.7:6881"]["country"], "AU");
        assert_eq!(p["[2a00:1450::1]:51413"]["country"], "IE");
        assert!(p["9.9.9.9:1"].get("country").is_none());
        assert!(p["not an address"].get("country").is_none());
        // Anything else is left alone.
        let mut other = serde_json::json!(null);
        add_countries(&db, &mut other);
        assert!(other.is_null());
    }
}
