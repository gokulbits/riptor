//! Torrent queueing, following libtorrent's auto-manage rules as used by
//! qBittorrent:
//!
//! * Downloading torrents are ordered by queue position; seeds by least
//!   seeding time first, so seeding is shared fairly.
//! * Forced torrents always run and still use up slots.
//! * With "don't count slow torrents", a running torrent that has been below
//!   the rate threshold for the inactivity timer keeps running but does not
//!   use up a slot, which lets the next queued torrent start.
//! * Downloads are placed first, then seeds take whatever total slots remain.

use serde::{Deserialize, Serialize};

use crate::{Action, Intent, TorrentId, TorrentView};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueuePrefs {
    pub enabled: bool,
    /// `None` = unlimited.
    pub max_active_downloads: Option<u32>,
    pub max_active_uploads: Option<u32>,
    pub max_active_torrents: Option<u32>,
    pub dont_count_slow: bool,
    /// Bytes/s.
    pub dl_rate_threshold: u64,
    /// Bytes/s.
    pub ul_rate_threshold: u64,
    pub inactivity_timer_secs: u64,
}

impl Default for QueuePrefs {
    // Same defaults as qBittorrent.
    fn default() -> Self {
        Self {
            enabled: true,
            max_active_downloads: Some(3),
            max_active_uploads: Some(3),
            max_active_torrents: Some(5),
            dont_count_slow: false,
            dl_rate_threshold: 2 * 1024,
            ul_rate_threshold: 2 * 1024,
            inactivity_timer_secs: 60,
        }
    }
}

/// A downloading torrent is judged by its download rate, a seed by its upload rate.
pub(crate) fn below_thresholds(t: &TorrentView, q: &QueuePrefs) -> bool {
    if t.finished {
        t.ul_rate < q.ul_rate_threshold
    } else {
        t.dl_rate < q.dl_rate_threshold
    }
}

struct Slots {
    left: Option<u32>,
}

impl Slots {
    fn new(max: Option<u32>) -> Self {
        Self { left: max }
    }
    fn available(&self) -> bool {
        self.left.is_none_or(|n| n > 0)
    }
    fn take(&mut self) {
        if let Some(n) = &mut self.left {
            *n = n.saturating_sub(1);
        }
    }
}

pub fn plan(
    torrents: &[&TorrentView],
    q: &QueuePrefs,
    is_slow: &dyn Fn(TorrentId) -> bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let want = |t: &TorrentView, run: bool, out: &mut Vec<Action>| {
        if run && !t.running {
            out.push(Action::Start { id: t.id });
        } else if !run && t.running {
            out.push(Action::Queue { id: t.id });
        }
    };

    let eligible = |t: &&&TorrentView| !t.busy_or_error && t.intent != Intent::Stopped;

    if !q.enabled {
        for t in torrents.iter().filter(eligible) {
            want(t, true, &mut out);
        }
        return out;
    }

    let mut total = Slots::new(q.max_active_torrents);
    let mut downs = Slots::new(q.max_active_downloads);
    let mut ups = Slots::new(q.max_active_uploads);

    let counts = |t: &TorrentView| !(q.dont_count_slow && t.running && is_slow(t.id));

    // Forced torrents run unconditionally but use up slots first.
    for t in torrents
        .iter()
        .filter(eligible)
        .filter(|t| t.intent == Intent::Forced)
    {
        want(t, true, &mut out);
        if counts(t) {
            total.take();
            if t.finished {
                ups.take()
            } else {
                downs.take()
            }
        }
    }

    let mut downloading: Vec<&TorrentView> = torrents
        .iter()
        .filter(eligible)
        .filter(|t| t.intent == Intent::Started && !t.finished)
        .copied()
        .collect();
    downloading.sort_by_key(|t| (t.queue_pos.unwrap_or(u32::MAX), t.id));

    let mut seeding: Vec<&TorrentView> = torrents
        .iter()
        .filter(eligible)
        .filter(|t| t.intent == Intent::Started && t.finished)
        .copied()
        .collect();
    seeding.sort_by_key(|t| (t.seeding_time, t.id));

    for (list, class) in [(downloading, &mut downs), (seeding, &mut ups)] {
        for t in list {
            if !counts(t) {
                // Slow and already running: keep it going without a slot.
                want(t, true, &mut out);
            } else if class.available() && total.available() {
                class.take();
                total.take();
                want(t, true, &mut out);
            } else {
                want(t, false, &mut out);
            }
        }
    }
    out
}

/// Queue-position editing helpers. `order` holds the ids of downloading
/// torrents, index = queue position.
pub mod order {
    use crate::TorrentId;

    pub fn move_up(order: &mut [TorrentId], ids: &[TorrentId]) {
        for i in 1..order.len() {
            if ids.contains(&order[i]) && !ids.contains(&order[i - 1]) {
                order.swap(i, i - 1);
            }
        }
    }

    pub fn move_down(order: &mut [TorrentId], ids: &[TorrentId]) {
        for i in (0..order.len().saturating_sub(1)).rev() {
            if ids.contains(&order[i]) && !ids.contains(&order[i + 1]) {
                order.swap(i, i + 1);
            }
        }
    }

    pub fn move_top(order: &mut Vec<TorrentId>, ids: &[TorrentId]) {
        let (mut sel, rest): (Vec<_>, Vec<_>) = order.iter().partition(|id| ids.contains(id));
        sel.extend(rest);
        *order = sel;
    }

    pub fn move_bottom(order: &mut Vec<TorrentId>, ids: &[TorrentId]) {
        let (sel, mut rest): (Vec<_>, Vec<_>) = order.iter().partition(|id| ids.contains(id));
        rest.extend(sel);
        *order = rest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{seed, view};

    fn limits(d: u32, u: u32, t: u32) -> QueuePrefs {
        QueuePrefs {
            max_active_downloads: Some(d),
            max_active_uploads: Some(u),
            max_active_torrents: Some(t),
            ..QueuePrefs::default()
        }
    }

    fn run(ts: &[TorrentView], q: &QueuePrefs, slow: &[TorrentId]) -> Vec<Action> {
        let refs: Vec<&TorrentView> = ts.iter().collect();
        plan(&refs, q, &|id| slow.contains(&id))
    }

    fn stopped_all(ts: &mut [TorrentView]) {
        ts.iter_mut().for_each(|t| t.running = false);
    }

    #[test]
    fn starts_in_queue_order_up_to_limit() {
        let mut ts = vec![view(0), view(1), view(2), view(3)];
        stopped_all(&mut ts);
        ts[0].queue_pos = Some(3);
        ts[3].queue_pos = Some(0);
        let acts = run(&ts, &limits(2, 2, 5), &[]);
        assert_eq!(acts, vec![Action::Start { id: 3 }, Action::Start { id: 1 }]);
    }

    #[test]
    fn queues_running_torrents_over_limit() {
        let ts = vec![view(0), view(1), view(2)];
        let acts = run(&ts, &limits(1, 1, 5), &[]);
        assert_eq!(acts, vec![Action::Queue { id: 1 }, Action::Queue { id: 2 }]);
    }

    #[test]
    fn forced_bypasses_queue_but_uses_slot() {
        let mut ts = vec![view(0), view(1)];
        stopped_all(&mut ts);
        ts[1].intent = Intent::Forced;
        let acts = run(&ts, &limits(1, 1, 5), &[]);
        // 1 is forced and takes the only download slot, so 0 stays queued.
        assert_eq!(acts, vec![Action::Start { id: 1 }]);
    }

    #[test]
    fn forced_runs_even_when_limits_are_zero() {
        let mut ts = vec![view(0)];
        stopped_all(&mut ts);
        ts[0].intent = Intent::Forced;
        assert_eq!(
            run(&ts, &limits(0, 0, 0), &[]),
            vec![Action::Start { id: 0 }]
        );
    }

    #[test]
    fn total_limit_caps_downloads_plus_seeds() {
        let mut ts = vec![view(0), view(1), seed(2), seed(3)];
        stopped_all(&mut ts);
        let acts = run(&ts, &limits(5, 5, 3), &[]);
        assert_eq!(
            acts,
            vec![
                Action::Start { id: 0 },
                Action::Start { id: 1 },
                Action::Start { id: 2 }
            ]
        );
    }

    #[test]
    fn seeds_prefer_least_seeded() {
        let mut ts = vec![seed(0), seed(1), seed(2)];
        stopped_all(&mut ts);
        ts[0].seeding_time = 500;
        ts[1].seeding_time = 100;
        ts[2].seeding_time = 300;
        let acts = run(&ts, &limits(5, 2, 5), &[]);
        assert_eq!(acts, vec![Action::Start { id: 1 }, Action::Start { id: 2 }]);
    }

    #[test]
    fn slow_torrents_do_not_count_when_enabled() {
        let mut ts = vec![view(0), view(1)];
        ts[1].running = false;
        let mut q = limits(1, 1, 5);
        q.dont_count_slow = true;
        // 0 is running and slow, so 1 can start too.
        assert_eq!(run(&ts, &q, &[0]), vec![Action::Start { id: 1 }]);
        // Without the option, 1 stays queued.
        q.dont_count_slow = false;
        assert!(run(&ts, &q, &[0]).is_empty());
    }

    #[test]
    fn stopped_errored_and_busy_are_left_alone() {
        let mut ts = vec![view(0), view(1), view(2)];
        ts[0].intent = Intent::Stopped; // running but stopped intent: not our business
        ts[1].busy_or_error = true;
        ts[2].running = false;
        assert_eq!(
            run(&ts, &limits(1, 1, 1), &[]),
            vec![Action::Start { id: 2 }]
        );
    }

    #[test]
    fn disabled_queue_starts_everything() {
        let mut ts = vec![view(0), view(1), seed(2)];
        stopped_all(&mut ts);
        let q = QueuePrefs {
            enabled: false,
            ..limits(0, 0, 0)
        };
        assert_eq!(run(&ts, &q, &[]).len(), 3);
    }

    #[test]
    fn unlimited_is_none() {
        let mut ts: Vec<_> = (0..50).map(view).collect();
        stopped_all(&mut ts);
        let q = QueuePrefs {
            max_active_downloads: None,
            max_active_uploads: None,
            max_active_torrents: None,
            ..QueuePrefs::default()
        };
        assert_eq!(run(&ts, &q, &[]).len(), 50);
    }

    #[test]
    fn order_helpers() {
        let mut o = vec![10, 11, 12, 13, 14];
        order::move_up(&mut o, &[12, 13]);
        assert_eq!(o, vec![10, 12, 13, 11, 14]);
        order::move_up(&mut o, &[10]); // already first
        assert_eq!(o, vec![10, 12, 13, 11, 14]);
        order::move_down(&mut o, &[10, 12]);
        assert_eq!(o, vec![13, 10, 12, 11, 14]);
        order::move_top(&mut o, &[11, 14]);
        assert_eq!(o, vec![11, 14, 13, 10, 12]);
        order::move_bottom(&mut o, &[11]);
        assert_eq!(o, vec![14, 13, 10, 12, 11]);
    }
}
