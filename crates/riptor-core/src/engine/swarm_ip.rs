//! Compares the address the swarm sees with the VPN's, and holds torrents on a confirmed leak.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use serde::Serialize;
use tracing::warn;

use super::{Engine, Job};
use crate::model::NoticeLevel;
use crate::network::NetworkMode;
use crate::seen_ip::{self, Seen, Verdict};

const RETRY_LOOKUP: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Serialize)]
pub struct SwarmView {
    /// Addresses reported by peers and trackers, most-reported first.
    pub seen: Vec<Seen>,
    pub vpn_ip: Option<String>,
    /// Known only after a leak check.
    pub normal_ip: Option<String>,
    pub verdict: Verdict,
    /// Torrents are held until the user looks at the leak.
    pub held: Option<String>,
}

impl Engine {
    /// Called on every network change: old reports and addresses no longer apply.
    pub(super) fn forget_swarm_view(&self) {
        seen_ip::reset();
        *self.vpn_public.lock() = (None, None);
        *self.swarm_verdict.lock() = Verdict::Unknown;
    }

    pub(super) fn note_public_ip(&self, ip: IpAddr, via_vpn: bool) {
        if via_vpn {
            *self.vpn_public.lock() = (Some(ip), Some(Instant::now()));
        } else {
            *self.normal_public.lock() = Some(ip);
        }
    }

    pub(super) async fn look_up_vpn_address(&self) {
        let NetworkMode::Vpn(b) = self.network() else {
            return;
        };
        match crate::public_ip::lookup(Some(IpAddr::V4(b.local_ip))).await {
            Ok(ip) if self.network() == NetworkMode::Vpn(b) => self.note_public_ip(ip, true),
            Ok(_) => {}
            Err(e) => warn!("couldn't look up the VPN's public address: {e:#}"),
        }
    }

    /// From the ticker, every few seconds.
    pub(super) fn check_swarm_view(&self) {
        let NetworkMode::Vpn(_) = self.network() else {
            return;
        };
        let (vpn, tried) = *self.vpn_public.lock();
        if vpn.is_none() && tried.is_none_or(|t| t.elapsed() > RETRY_LOOKUP) {
            self.vpn_public.lock().1 = Some(Instant::now());
            self.queue_job(Job::VpnAddress);
        }
        let normal = *self.normal_public.lock();
        let v = seen_ip::verdict(&seen_ip::summary(), vpn, normal);
        let changed = {
            let mut last = self.swarm_verdict.lock();
            let changed = *last != v;
            *last = v.clone();
            changed
        };
        if !changed {
            return;
        }
        match v {
            Verdict::Leak { ip } => {
                warn!("peers and trackers see the normal address while on the VPN");
                let hold = self.kill_switch_on();
                if hold {
                    *self.leak_hold.lock() = Some(ip.clone());
                }
                self.notice(
                    NoticeLevel::Error,
                    format!(
                        "Peers and trackers see your normal address ({ip}), not the VPN's. {}",
                        if hold {
                            "Torrents are held until you check the VPN page."
                        } else {
                            "Turn on the kill switch to hold torrents when this happens."
                        }
                    ),
                );
            }
            Verdict::Unexpected { ip } => self.notice(
                NoticeLevel::Warning,
                format!("Most peers see you as {ip}, not as the VPN's address. Run the leak check on the VPN page."),
            ),
            Verdict::Expected | Verdict::Unknown => {}
        }
    }

    pub(super) fn leak_held(&self) -> bool {
        self.leak_hold.lock().is_some()
    }

    pub fn swarm_view(&self) -> SwarmView {
        let seen = seen_ip::summary();
        SwarmView {
            seen: seen.into_iter().take(10).collect(),
            vpn_ip: self.vpn_public.lock().0.map(|i| i.to_string()),
            normal_ip: self.normal_public.lock().map(|i| i.to_string()),
            verdict: self.swarm_verdict.lock().clone(),
            held: self.leak_hold.lock().clone(),
        }
    }

    /// The user looked; torrents may run again (the check keeps watching).
    pub fn release_leak_hold(&self) {
        *self.leak_hold.lock() = None;
        seen_ip::reset();
        *self.swarm_verdict.lock() = Verdict::Unknown;
    }
}
