//! Holding torrents for the connection's sake: metered networks and the monthly data cap.

use super::{unix_now, Engine};
use crate::model::CapUsage;
use crate::netinfo::NetInfo;
use crate::settings::CapCounts;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Hold {
    /// Everything waits.
    pub all: Option<String>,
    /// Only unfinished torrents wait (the cap counts downloads only).
    pub downloads: Option<String>,
}

impl Engine {
    pub(super) async fn refresh_net_info(&self) {
        if let Ok(info) = tokio::task::spawn_blocking(crate::netinfo::current).await {
            *self.net_info.lock() = Some(info);
        }
        if let Err(e) = self.apply_kill_switch().await {
            tracing::warn!("kill switch: {e:#}");
        }
    }

    /// The kill switch setting, or the rule for networks that aren't trusted.
    pub fn kill_switch_on(&self) -> bool {
        let names = self.net_info.lock().as_ref().map(|n| n.names.clone());
        self.settings.read().kill_switch(names.as_deref())
    }

    /// Only the rule for untrusted networks is holding torrents.
    pub(super) fn untrusted_network(&self) -> bool {
        self.kill_switch_on() && !self.settings.read().vpn_required
    }

    /// Holds or frees traffic when the settings or the network change; the VPN itself is left alone.
    pub(super) async fn apply_kill_switch(&self) -> anyhow::Result<()> {
        use crate::network::NetworkMode;
        let want = self.kill_switch_on();
        match self.network() {
            NetworkMode::Normal if want => self.set_network(NetworkMode::Blocked).await,
            NetworkMode::Blocked if !want => self.set_network(NetworkMode::Normal).await,
            _ => Ok(()),
        }
    }

    /// `None` until the OS has been asked.
    pub fn net_info(&self) -> Option<NetInfo> {
        self.net_info.lock().clone()
    }

    pub fn data_cap_usage(&self) -> Option<CapUsage> {
        let cap = self.settings.read().data_cap?;
        let now = unix_now();
        let t = self
            .statistics
            .lock()
            .since(&crate::stats::period_start(now, cap.reset_day));
        let used = match cap.counts {
            CapCounts::Download => t.downloaded,
            CapCounts::Upload => t.uploaded,
            CapCounts::Both => t.downloaded.saturating_add(t.uploaded),
        };
        Some(CapUsage {
            used,
            limit: u64::from(cap.gib) * GIB,
        })
    }

    pub(super) fn hold(&self) -> Hold {
        let s = self.settings.read();
        let (metered, cap) = (s.pause_on_metered, s.data_cap);
        drop(s);
        let mut h = Hold::default();
        if metered && self.net_info.lock().as_ref().and_then(|n| n.metered) == Some(true) {
            h.all = Some("Waiting: the connection is metered".into());
        }
        if let (Some(cap), Some(u)) = (cap, self.data_cap_usage()) {
            if u.used >= u.limit {
                let why = Some("Waiting: this month's data cap is used up".to_string());
                match cap.counts {
                    CapCounts::Download => h.downloads = h.downloads.or(why),
                    _ => h.all = h.all.or(why),
                }
            }
        }
        h
    }
}
