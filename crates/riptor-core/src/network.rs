//! Normal network, VPN adapter only, or nothing (kill switch).

use std::net::Ipv4Addr;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VpnBinding {
    pub adapter: String,
    pub local_ip: Ipv4Addr,
    /// From NAT-PMP.
    pub forwarded_port: Option<u16>,
}

impl VpnBinding {
    /// What Riptor's own sockets bind to on the VPN.
    pub fn end(&self) -> riptor_net::TunnelEnd {
        riptor_net::TunnelEnd::new(self.local_ip.into(), self.adapter.clone())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Normal,
    Vpn(VpnBinding),
    /// VPN required but down: no DHT, listener, trackers or peers.
    Blocked,
}

/// "#<index>" on Windows (our socket patch); SO_BINDTODEVICE needs a name.
fn adapter_id(i: &network_interface::NetworkInterface) -> String {
    if cfg!(windows) {
        format!("#{}", i.index)
    } else {
        i.name.clone()
    }
}

pub fn loopback_adapter() -> Option<String> {
    use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
    NetworkInterface::show()
        .ok()?
        .into_iter()
        .find(|i| {
            i.addr
                .iter()
                .any(|a| matches!(a, Addr::V4(v) if v.ip.is_loopback()))
        })
        .map(|i| adapter_id(&i))
}

pub fn adapter_for_ip(ip: Ipv4Addr) -> Option<String> {
    use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
    NetworkInterface::show()
        .ok()?
        .into_iter()
        .find(|i| {
            i.addr
                .iter()
                .any(|a| matches!(a, Addr::V4(v) if v.ip == ip))
        })
        .map(|i| adapter_id(&i))
}

/// This computer's addresses on local networks (private IPv4, not loopback or link-local).
pub fn lan_addresses() -> Vec<Ipv4Addr> {
    use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
    let mut out: Vec<Ipv4Addr> = NetworkInterface::show()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|i| i.addr)
        .filter_map(|a| match a {
            Addr::V4(v) if v.ip.is_private() => Some(v.ip),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

impl NetworkMode {
    /// Tells the connections ledger how traffic leaves now.
    pub fn note_route(&self) {
        use riptor_net::ledger::{set_default_route, Route};
        set_default_route(match self {
            NetworkMode::Vpn(_) => Route::Tunnel,
            _ => Route::Direct,
        });
    }

    pub fn label(&self) -> &'static str {
        match self {
            NetworkMode::Normal => "normal",
            NetworkMode::Vpn(_) => "vpn",
            NetworkMode::Blocked => "blocked",
        }
    }
}
