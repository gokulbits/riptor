//! Riptor's app-only VPN.
//!
//! The tunnel is brought up *without* taking over the system's default
//! route, and only Riptor's sockets are pinned to the tunnel adapter (see the
//! `BindDevice` patch in `vendor/librqbit-dualstack-sockets`). Everything else
//! on the computer keeps using the normal connection.
//!
//! * [`ovpn`]: turn an untrusted `.ovpn` file into a safe, app-only config.
//! * [`wg`]: the same for WireGuard `.conf` files (app-only on Linux only;
//!   the Windows WireGuard client can't skip installing routes).
//! * [`mgmt`]: OpenVPN management-interface protocol (credentials, state).
//! * [`natpmp`]: NAT-PMP port mapping (VPN port forwarding).
#![forbid(unsafe_code)]

pub mod mgmt;
pub mod natpmp;
pub mod ovpn;
pub mod profiles;
pub mod wg;
