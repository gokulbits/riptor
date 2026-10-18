//! Seeding policy for Riptor, modelled on qBittorrent.
//!
//! Everything here is pure: callers pass a snapshot of every torrent plus the
//! preferences, and get back a list of [`Action`]s to apply to the engine.
//! The only state kept between ticks is the slow-torrent timer in [`Scheduler`].
//!
//! * [`ratio::real_ratio`] — qBittorrent's `TorrentImpl::realRatio()`.
//! * [`limits`] — max ratio / seeding time / inactive seeding time with
//!   per-torrent overrides (`UseGlobal`, `Unlimited`, custom value).
//! * [`queue`] — active download/upload/torrent limits with "don't count slow
//!   torrents" and force-start, following libtorrent's auto-manage rules.

pub mod limits;
pub mod queue;
pub mod ratio;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use limits::{Limit, LimitKind, ShareLimitAction, ShareLimits};
pub use queue::QueuePrefs;
pub use ratio::{real_ratio, MAX_RATIO};

pub type TorrentId = usize;

/// What the user asked for. The scheduler decides whether a `Started`
/// torrent actually runs or waits in the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Stopped by the user (or by a share-limit Stop). Never started by the queue.
    Stopped,
    /// Wants to run; subject to queueing.
    Started,
    /// Force-started: always runs, ignores the queue, still uses up slots.
    Forced,
}

/// Snapshot of one torrent as the scheduler needs it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TorrentView {
    pub id: TorrentId,
    pub intent: Intent,
    /// Is the engine currently running it (not paused)?
    pub running: bool,
    /// All wanted pieces are downloaded.
    pub finished: bool,
    /// Errored or still initialising/checking: left alone by the queue.
    pub busy_or_error: bool,
    /// Queue position for downloading torrents (0 = first). Finished torrents
    /// have none, as in qBittorrent.
    pub queue_pos: Option<u32>,

    pub total_done: u64,
    pub all_time_upload: u64,
    pub all_time_download: u64,
    /// Seconds spent running while finished.
    pub seeding_time: u64,
    /// Seconds since the last payload transfer in either direction.
    pub time_since_activity: u64,
    /// Seconds since the engine last started it (for the slow-torrent grace period).
    pub running_for: u64,
    pub dl_rate: u64,
    pub ul_rate: u64,

    pub limits: ShareLimits,
    /// `None` uses the global action.
    pub limit_action: Option<ShareLimitAction>,
}

impl TorrentView {
    pub fn ratio(&self) -> f64 {
        real_ratio(
            self.all_time_upload,
            self.all_time_download,
            self.total_done,
        )
    }
}

/// Global preferences, serialised into settings.json.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Global max ratio; `None` = no limit.
    pub max_ratio: Option<f64>,
    /// Global max seeding time in minutes; `None` = no limit.
    pub max_seeding_minutes: Option<u64>,
    /// Global max inactive seeding time in minutes; `None` = no limit.
    pub max_inactive_seeding_minutes: Option<u64>,
    pub limit_action: ShareLimitAction,
    pub queue: QueuePrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            max_ratio: None,
            max_seeding_minutes: None,
            max_inactive_seeding_minutes: None,
            limit_action: ShareLimitAction::Stop,
            queue: QueuePrefs::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Resume in the engine (slot available or force-started).
    Start { id: TorrentId },
    /// Pause in the engine because the queue is full; intent stays `Started`.
    Queue { id: TorrentId },
    /// A share limit was reached: apply `action` (Stop also sets intent to Stopped).
    ShareLimit {
        id: TorrentId,
        action: ShareLimitAction,
        reason: LimitKind,
    },
}

/// Holds the only cross-tick state: when each torrent first dropped below the
/// slow-rate thresholds.
#[derive(Default, Debug)]
pub struct Scheduler {
    slow_since: HashMap<TorrentId, u64>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run one scheduling pass. `now` is a monotonic seconds counter.
    ///
    /// Share limits are applied first; torrents hit by one are excluded from
    /// the queue pass so a removed torrent never gets started in the same tick.
    pub fn tick(&mut self, torrents: &[TorrentView], prefs: &Prefs, now: u64) -> Vec<Action> {
        let mut actions = limits::check_all(torrents, prefs);
        let limited: Vec<TorrentId> = actions
            .iter()
            .filter_map(|a| match a {
                Action::ShareLimit { id, .. } => Some(*id),
                _ => None,
            })
            .collect();

        self.update_slow(torrents, &prefs.queue, now);
        let remaining: Vec<&TorrentView> = torrents
            .iter()
            .filter(|t| !limited.contains(&t.id))
            .collect();
        actions.extend(queue::plan(&remaining, &prefs.queue, &|id| {
            self.is_slow(id, &prefs.queue, now)
        }));
        actions
    }

    fn update_slow(&mut self, torrents: &[TorrentView], q: &QueuePrefs, now: u64) {
        self.slow_since
            .retain(|id, _| torrents.iter().any(|t| t.id == *id));
        for t in torrents {
            let below = t.running && queue::below_thresholds(t, q);
            if below {
                self.slow_since.entry(t.id).or_insert(now);
            } else {
                self.slow_since.remove(&t.id);
            }
        }
    }

    fn is_slow(&self, id: TorrentId, q: &QueuePrefs, now: u64) -> bool {
        self.slow_since
            .get(&id)
            .is_some_and(|since| now.saturating_sub(*since) >= q.inactivity_timer_secs)
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub fn view(id: TorrentId) -> TorrentView {
        TorrentView {
            id,
            intent: Intent::Started,
            running: true,
            finished: false,
            busy_or_error: false,
            queue_pos: Some(id as u32),
            total_done: 0,
            all_time_upload: 0,
            all_time_download: 0,
            seeding_time: 0,
            time_since_activity: 0,
            running_for: 1000,
            dl_rate: 1_000_000,
            ul_rate: 1_000_000,
            limits: ShareLimits::default(),
            limit_action: None,
        }
    }

    pub fn seed(id: TorrentId) -> TorrentView {
        TorrentView {
            finished: true,
            queue_pos: None,
            total_done: 1000,
            all_time_download: 1000,
            ..view(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    #[test]
    fn limited_torrent_is_not_started_by_queue() {
        let mut t = seed(1);
        t.running = false;
        t.all_time_upload = 5000; // ratio 5
        let prefs = Prefs {
            max_ratio: Some(2.0),
            ..Prefs::default()
        };
        let actions = Scheduler::new().tick(&[t], &prefs, 0);
        assert_eq!(
            actions,
            vec![Action::ShareLimit {
                id: 1,
                action: ShareLimitAction::Stop,
                reason: LimitKind::Ratio
            }]
        );
    }

    #[test]
    fn slow_timer_needs_continuous_low_rate() {
        let prefs = Prefs {
            queue: QueuePrefs {
                max_active_downloads: Some(1),
                max_active_torrents: None,
                dont_count_slow: true,
                dl_rate_threshold: 2048,
                inactivity_timer_secs: 60,
                ..QueuePrefs::default()
            },
            ..Prefs::default()
        };
        let mut a = view(0);
        a.dl_rate = 10; // slow
        let mut b = view(1);
        b.running = false;

        let mut s = Scheduler::new();
        // Before the timer elapses, `a` still counts, so `b` stays queued.
        assert!(s.tick(&[a.clone(), b.clone()], &prefs, 0).is_empty());
        assert!(s.tick(&[a.clone(), b.clone()], &prefs, 59).is_empty());
        // After 60 s of being slow, `a` no longer counts and `b` gets its slot.
        assert_eq!(
            s.tick(&[a.clone(), b.clone()], &prefs, 60),
            vec![Action::Start { id: 1 }]
        );

        // Speeding up resets the timer.
        a.dl_rate = 1_000_000;
        b.running = true;
        let acts = s.tick(&[a.clone(), b.clone()], &prefs, 61);
        assert_eq!(acts, vec![Action::Queue { id: 1 }]);
    }

    #[test]
    fn prefs_roundtrip_json() {
        let p = Prefs {
            max_ratio: Some(1.5),
            ..Prefs::default()
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&s).unwrap(), p);
        // Missing fields fall back to defaults.
        assert_eq!(
            serde_json::from_str::<Prefs>("{}").unwrap(),
            Prefs::default()
        );
    }
}
