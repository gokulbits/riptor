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

impl NetworkMode {
    pub fn label(&self) -> &'static str {
        match self {
            NetworkMode::Normal => "normal",
            NetworkMode::Vpn(_) => "vpn",
            NetworkMode::Blocked => "blocked",
        }
    }
}
