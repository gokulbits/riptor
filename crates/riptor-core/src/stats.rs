//! All-time and per-session statistics.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const HISTORY_DAYS: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTotals {
    pub uploaded: u64,
    pub downloaded: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Statistics {
    pub all_time_upload: u64,
    pub all_time_download: u64,
    /// Duplicates and failed hash checks, counted when a torrent finishes.
    pub all_time_wasted: u64,
    pub torrents_added: u64,
    pub torrents_completed: u64,
    pub torrents_removed: u64,
    pub total_uptime_secs: u64,
    pub sessions: u64,
    pub first_started: u64,
    pub peak_download_rate: u64,
    pub peak_upload_rate: u64,
    /// Keyed by UTC "YYYY-MM-DD".
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

    /// Totals since the day `key` (inclusive).
    pub fn since(&self, key: &str) -> DayTotals {
        self.daily
            .range(key.to_string()..)
            .fold(DayTotals::default(), |mut acc, (_, d)| {
                acc.uploaded += d.uploaded;
                acc.downloaded += d.downloaded;
                acc
            })
    }

    /// Backups can be crafted.
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

/// The day a monthly period that restarts on `reset_day` began, as a day key.
pub fn period_start(now_unix: u64, reset_day: u8) -> String {
    let today = day_key(now_unix);
    let (y, m, d) = (
        today[..4].parse::<i32>().unwrap_or(1970),
        today[5..7].parse::<u32>().unwrap_or(1),
        today[8..].parse::<u8>().unwrap_or(1),
    );
    let reset = reset_day.clamp(1, 28);
    let (y, m) = if d >= reset {
        (y, m)
    } else if m == 1 {
        (y - 1, 12)
    } else {
        (y, m - 1)
    };
    format!("{y:04}-{m:02}-{reset:02}")
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
    fn data_cap_periods() {
        // 2026-03-15 and 2026-01-03 (UTC noon).
        let mar15 = 1_773_576_000;
        let jan3 = 1_767_441_600;
        assert_eq!(day_key(mar15), "2026-03-15");
        assert_eq!(day_key(jan3), "2026-01-03");
        assert_eq!(period_start(mar15, 1), "2026-03-01");
        assert_eq!(period_start(mar15, 15), "2026-03-15");
        assert_eq!(period_start(mar15, 20), "2026-02-20");
        assert_eq!(period_start(jan3, 10), "2025-12-10");
        assert_eq!(period_start(jan3, 0), "2026-01-01");

        let mut s = Statistics::default();
        s.record_transfer(100, 1, mar15 - 30 * 86_400);
        s.record_transfer(5, 2, mar15 - 86_400);
        s.record_transfer(7, 3, mar15);
        let t = s.since(&period_start(mar15, 10));
        assert_eq!((t.downloaded, t.uploaded), (12, 5));
    }

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
