//! How the torrent session may use the network: normally, only through the
//! VPN adapter (app-only VPN), or not at all (kill switch engaged).

use std::net::Ipv4Addr;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VpnBinding {
    /// Adapter to pin every socket to ("#<index>" on Windows, a name elsewhere).
    pub adapter: String,
    /// The adapter's tunnel address; listener and HTTP trackers bind to it.
    pub local_ip: Ipv4Addr,
    /// Public port mapped by the VPN (NAT-PMP), announced to trackers/peers.
    pub forwarded_port: Option<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkMode {
    /// No VPN: the normal connection.
    #[default]
    Normal,
    /// App-only VPN: all of the session's traffic is pinned to the tunnel.
    Vpn(VpnBinding),
    /// Kill switch: the VPN is required but not connected. No DHT, no
    /// listener, no trackers, no peer connections; torrents stay paused.
    Blocked,
}

/// How sockets name an adapter: "#<index>" on Windows (the patched socket
/// layer applies IP_UNICAST_IF), the interface name elsewhere (SO_BINDTODEVICE
/// only takes names).
fn adapter_id(i: &network_interface::NetworkInterface) -> String {
    if cfg!(windows) {
        format!("#{}", i.index)
    } else {
        i.name.clone()
    }
}

/// The loopback adapter, used to pin sockets when blocked.
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

/// Find the adapter that owns `ip` (the VPN tunnel address).
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
