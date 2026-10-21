//! What peers (`yourip`) and trackers (BEP 24) say they see us as; several must agree to count.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

const MAX_REPORTS: usize = 1024;
const KEEP: Duration = Duration::from_secs(30 * 60);
/// Unrelated sources needed before a report counts.
pub const QUORUM: usize = 3;

#[derive(Clone, Debug)]
struct Report {
    seen: IpAddr,
    /// A peer's network (/16 or /48) or a tracker's host: one vote each.
    source: String,
    tracker: bool,
    at: Instant,
}

static REPORTS: LazyLock<Mutex<VecDeque<Report>>> = LazyLock::new(Default::default);

fn network_of(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            format!("{}.{}.0.0/16", o[0], o[1])
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2])
        }
    }
}

fn push(r: Report) {
    let mut all = REPORTS.lock();
    while all.front().is_some_and(|f| f.at.elapsed() > KEEP) || all.len() >= MAX_REPORTS {
        all.pop_front();
    }
    all.push_back(r);
}

/// LAN peers see LAN addresses, which say nothing about leaks.
pub fn record_peer(peer: IpAddr, seen: IpAddr) {
    if riptor_net::is_public(peer) && riptor_net::is_public(seen) {
        push(Report {
            seen,
            source: network_of(peer),
            tracker: false,
            at: Instant::now(),
        });
    }
}

pub fn record_tracker(host: &str, seen: IpAddr) {
    if riptor_net::is_public(seen) && !host.is_empty() {
        push(Report {
            seen,
            source: host.to_ascii_lowercase(),
            tracker: true,
            at: Instant::now(),
        });
    }
}

/// When the network changes, old reports describe the old route.
pub fn reset() {
    REPORTS.lock().clear();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Seen {
    pub ip: String,
    /// Separate peer networks that reported it.
    pub peers: usize,
    pub trackers: usize,
}

impl Seen {
    pub fn votes(&self) -> usize {
        self.peers + self.trackers
    }
}

/// Most-reported first.
pub fn summary() -> Vec<Seen> {
    summarize(REPORTS.lock().iter().filter(|r| r.at.elapsed() <= KEEP))
}

fn summarize<'a>(reports: impl Iterator<Item = &'a Report>) -> Vec<Seen> {
    let mut by_ip: Vec<(IpAddr, HashSet<(bool, String)>)> = Vec::new();
    for r in reports {
        let key = (r.tracker, r.source.clone());
        match by_ip.iter_mut().find(|(ip, _)| *ip == r.seen) {
            Some((_, s)) => {
                s.insert(key);
            }
            None => by_ip.push((r.seen, HashSet::from([key]))),
        }
    }
    let mut out: Vec<Seen> = by_ip
        .into_iter()
        .map(|(ip, s)| Seen {
            ip: ip.to_string(),
            peers: s.iter().filter(|(t, _)| !t).count(),
            trackers: s.iter().filter(|(t, _)| *t).count(),
        })
        .collect();
    out.sort_by(|a, b| b.votes().cmp(&a.votes()).then_with(|| a.ip.cmp(&b.ip)));
    out
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    /// Not enough reports yet.
    Unknown,
    /// The swarm sees the address we expect.
    Expected,
    /// Several sources see another address.
    Unexpected { ip: String },
    /// Several sources see your normal address: traffic is leaking.
    Leak { ip: String },
}

/// `expected`: the VPN's public address; `normal`: the address without it, if known.
pub fn verdict(seen: &[Seen], expected: Option<IpAddr>, normal: Option<IpAddr>) -> Verdict {
    let counts = |ip: IpAddr| seen.iter().find(|s| s.ip == ip.to_string());
    if let Some(n) = normal.filter(|n| Some(*n) != expected) {
        if let Some(s) = counts(n).filter(|s| s.votes() >= QUORUM.min(2)) {
            return Verdict::Leak { ip: s.ip.clone() };
        }
    }
    let Some(top) = seen.iter().find(|s| s.votes() >= QUORUM) else {
        return Verdict::Unknown;
    };
    match expected {
        Some(e) if top.ip == e.to_string() => Verdict::Expected,
        Some(e) => {
            let expected_votes = counts(e).map_or(0, Seen::votes);
            if top.votes() > expected_votes {
                Verdict::Unexpected { ip: top.ip.clone() }
            } else {
                Verdict::Expected
            }
        }
        None => Verdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn report(seen: &str, source: &str, tracker: bool) -> Report {
        Report {
            seen: ip(seen),
            source: source.into(),
            tracker,
            at: Instant::now(),
        }
    }

    #[test]
    fn votes_count_unrelated_sources_once() {
        let r = [
            report("198.51.100.7", "1.2.0.0/16", false),
            report("198.51.100.7", "1.2.0.0/16", false),
            report("198.51.100.7", "5.6.0.0/16", false),
            report("198.51.100.7", "tracker.example", true),
            report("203.0.113.1", "9.9.0.0/16", false),
        ];
        let s = summarize(r.iter());
        assert_eq!(
            s[0],
            Seen {
                ip: "198.51.100.7".into(),
                peers: 2,
                trackers: 1
            }
        );
        assert_eq!(s[1].votes(), 1);
    }

    #[test]
    fn a_leak_needs_more_than_one_liar() {
        let vpn = ip("198.51.100.7");
        let home = ip("203.0.113.50");
        let seen = |list: &[(&str, usize)]| -> Vec<Seen> {
            list.iter()
                .map(|(i, n)| Seen {
                    ip: i.to_string(),
                    peers: *n,
                    trackers: 0,
                })
                .collect()
        };
        assert_eq!(verdict(&[], Some(vpn), None), Verdict::Unknown);
        assert_eq!(
            verdict(&seen(&[("198.51.100.7", 5)]), Some(vpn), Some(home)),
            Verdict::Expected
        );
        // One peer claiming the home address is noise; two unrelated ones aren't.
        assert_eq!(
            verdict(
                &seen(&[("198.51.100.7", 5), ("203.0.113.50", 1)]),
                Some(vpn),
                Some(home)
            ),
            Verdict::Expected
        );
        assert_eq!(
            verdict(
                &seen(&[("198.51.100.7", 5), ("203.0.113.50", 2)]),
                Some(vpn),
                Some(home)
            ),
            Verdict::Leak {
                ip: "203.0.113.50".into()
            }
        );
        // A different address seen by most of the swarm, with the home one unknown.
        assert_eq!(
            verdict(
                &seen(&[("192.0.2.9", 6), ("198.51.100.7", 1)]),
                Some(vpn),
                None
            ),
            Verdict::Unexpected {
                ip: "192.0.2.9".into()
            }
        );
        // A few liars against a swarm that agrees with us.
        assert_eq!(
            verdict(
                &seen(&[("198.51.100.7", 9), ("192.0.2.9", 3)]),
                Some(vpn),
                None
            ),
            Verdict::Expected
        );
    }

    #[test]
    fn private_addresses_are_ignored() {
        record_peer(ip("192.168.1.20"), ip("198.51.100.77"));
        record_peer(ip("8.8.8.8"), ip("10.0.0.5"));
        record_tracker("t.example", ip("127.0.0.1"));
        assert!(summary()
            .iter()
            .all(|s| s.ip != "198.51.100.77" && s.ip != "10.0.0.5" && s.ip != "127.0.0.1"));
    }
}
