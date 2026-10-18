//! Client-wide statistics: persisted all-time tallies (survive restarts and
//! removed torrents, included in backups) plus per-session figures.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Days of per-day history kept.
const HISTORY_DAYS: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTotals {
    pub uploaded: u64,
    pub downloaded: u64,
}

/// Persisted in `statistics.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Statistics {
    pub all_time_upload: u64,
    pub all_time_download: u64,
    /// Bytes received that did not end up in verified pieces (duplicates and
    /// failed hash checks), measured on torrents once they finish.
    pub all_time_wasted: u64,
    pub torrents_added: u64,
    pub torrents_completed: u64,
    pub torrents_removed: u64,
    pub total_uptime_secs: u64,
    pub sessions: u64,
    /// Unix time Riptor first ran.
    pub first_started: u64,
    pub peak_download_rate: u64,
    pub peak_upload_rate: u64,
    /// "YYYY-MM-DD" (UTC) -> totals.
    pub daily: BTreeMap<String, DayTotals>,
}

impl Statistics {
    pub fn record_transfer(&mut self, downloaded: u64, uploaded: u64, now_unix: u64) {
        if downloaded == 0 && uploaded == 0 {
            return;
        }
        self.all_time_download += downloaded;
        self.all_time_upload += uploaded;
        let day = self.daily.entry(day_key(now_unix)).or_default();
        day.downloaded += downloaded;
        day.uploaded += uploaded;
        while self.daily.len() > HISTORY_DAYS {
            let oldest = self.daily.keys().next().cloned().expect("non-empty");
            self.daily.remove(&oldest);
        }
    }

    pub fn record_rates(&mut self, dl: u64, ul: u64) {
        self.peak_download_rate = self.peak_download_rate.max(dl);
        self.peak_upload_rate = self.peak_upload_rate.max(ul);
    }

    /// Sum of the last `days` days, including today.
    pub fn window(&self, now_unix: u64, days: u64) -> DayTotals {
        let from = day_key(now_unix.saturating_sub((days.saturating_sub(1)) * 86_400));
        self.daily
            .range(from..)
            .fold(DayTotals::default(), |mut acc, (_, d)| {
                acc.uploaded += d.uploaded;
                acc.downloaded += d.downloaded;
                acc
            })
    }

    /// Reject obviously corrupt or hostile values (e.g. from a crafted backup).
    pub fn sanitize(&mut self) {
        self.daily.retain(|k, _| is_day_key(k));
        while self.daily.len() > HISTORY_DAYS {
            let oldest = self.daily.keys().next().cloned().expect("non-empty");
            self.daily.remove(&oldest);
        }
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC calendar date for a unix timestamp, without pulling in a date crate.
pub fn day_key(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn is_day_key(k: &str) -> bool {
    let b = k.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && k.chars()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Everything the statistics page shows.
#[derive(Clone, Debug, Serialize)]
pub struct StatsReport {
    pub all_time: Statistics,
    pub all_time_ratio: f64,
    pub session_upload: u64,
    pub session_download: u64,
    pub session_wasted: u64,
    pub session_ratio: f64,
    pub session_uptime_secs: u64,
    pub today: DayTotals,
    pub last_7_days: DayTotals,
    pub last_30_days: DayTotals,
    pub dl_rate: u64,
    pub ul_rate: u64,
    pub torrents_total: usize,
    pub torrents_downloading: usize,
    pub torrents_seeding: usize,
    pub torrents_completed: usize,
    pub torrents_stopped: usize,
    pub torrents_queued: usize,
    pub torrents_errored: usize,
    pub total_size: u64,
    pub total_seeding_secs: u64,
    pub peers_connected: u32,
    pub peers_known: u32,
    pub blocked_incoming: u64,
    pub blocked_outgoing: u64,
    pub listen_port: Option<u16>,
    pub read_cache: riptor_storage::cache::CacheStats,
    pub write_buffer_used: usize,
    pub write_buffer_capacity: usize,
    pub disk: riptor_storage::store::MetricsSnapshot,
}

pub fn ratio(up: u64, down: u64) -> f64 {
    riptor_seeding::real_ratio(up, down, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_keys() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(day_key(951_782_400), "2000-02-29");
        assert_eq!(day_key(1_792_281_600), "2026-10-18");
        assert!(is_day_key("2026-10-18"));
        assert!(!is_day_key("../../etc"));
    }

    #[test]
    fn totals_and_windows() {
        let mut s = Statistics::default();
        let day = 86_400;
        let now = 1_792_281_600 + 3600;
        s.record_transfer(100, 10, now - 10 * day);
        s.record_transfer(5, 1, now - day);
        s.record_transfer(7, 2, now);
        s.record_transfer(0, 0, now);
        assert_eq!((s.all_time_download, s.all_time_upload), (112, 13));
        assert_eq!(
            s.window(now, 1),
            DayTotals {
                downloaded: 7,
                uploaded: 2
            }
        );
        assert_eq!(
            s.window(now, 7),
            DayTotals {
                downloaded: 12,
                uploaded: 3
            }
        );
        assert_eq!(s.window(now, 30).downloaded, 112);
    }

    #[test]
    fn history_is_capped_and_sanitized() {
        let mut s = Statistics::default();
        for d in 0..200u64 {
            s.record_transfer(1, 1, d * 86_400);
        }
        assert_eq!(s.daily.len(), HISTORY_DAYS);
        s.daily.insert("garbage".into(), DayTotals::default());
        s.sanitize();
        assert!(!s.daily.contains_key("garbage"));
    }
}
