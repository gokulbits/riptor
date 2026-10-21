//! Swarm sizes reported by trackers, kept from every announce.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use librqbit_core::hash_id::Id20;
use librqbit_tracker_comms::observe::Announce;
use parking_lot::Mutex;

/// Older counts are dropped: a swarm changes.
const FRESH: Duration = Duration::from_secs(6 * 3600);
const MAX_TRACKERS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Swarm {
    pub seeders: u32,
    pub leechers: u32,
}

type PerTracker = HashMap<String, (Swarm, Instant)>;

static SWARMS: LazyLock<Mutex<HashMap<Id20, PerTracker>>> = LazyLock::new(Default::default);

pub fn record(a: &Announce) {
    let (Some(seeders), Some(leechers)) = (a.seeders, a.leechers) else {
        return;
    };
    let mut all = SWARMS.lock();
    let per = all.entry(a.info_hash).or_default();
    if per.len() >= MAX_TRACKERS && !per.contains_key(&a.host) {
        return;
    }
    per.insert(
        a.host.clone(),
        (Swarm { seeders, leechers }, Instant::now()),
    );
}

/// The biggest swarm any tracker reported lately (trackers often see different peers).
pub fn get(h: &Id20) -> Option<Swarm> {
    let all = SWARMS.lock();
    let fresh = all
        .get(h)?
        .values()
        .filter(|(_, at)| at.elapsed() < FRESH)
        .map(|(s, _)| *s);
    fresh.reduce(|a, b| Swarm {
        seeders: a.seeders.max(b.seeders),
        leechers: a.leechers.max(b.leechers),
    })
}

pub fn forget(h: &Id20) {
    SWARMS.lock().remove(h);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announce(hash: Id20, host: &str, s: Option<u32>, l: Option<u32>) -> Announce {
        Announce {
            info_hash: hash,
            host: host.into(),
            ok: true,
            seeders: s,
            leechers: l,
            external_ip: None,
        }
    }

    #[test]
    fn the_largest_recent_count_wins() {
        let h = Id20::new([7; 20]);
        assert_eq!(get(&h), None);
        record(&announce(h, "a.example", Some(3), Some(10)));
        record(&announce(h, "b.example", Some(1), Some(40)));
        // No counts in the reply: ignored, not zero.
        record(&announce(h, "c.example", None, None));
        assert_eq!(
            get(&h),
            Some(Swarm {
                seeders: 3,
                leechers: 40
            })
        );
        record(&announce(h, "a.example", Some(0), Some(0)));
        assert_eq!(get(&h).unwrap().seeders, 1);
        forget(&h);
        assert_eq!(get(&h), None);
    }

    #[test]
    fn one_swarm_cannot_grow_without_bound() {
        let h = Id20::new([8; 20]);
        for i in 0..1000 {
            record(&announce(h, &format!("t{i}.example"), Some(i), Some(0)));
        }
        assert_eq!(SWARMS.lock()[&h].len(), MAX_TRACKERS);
        forget(&h);
    }
}
