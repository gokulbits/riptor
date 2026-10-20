//! App-only VPN. The tunnel never becomes the default route; only Riptor's
//! sockets are pinned to it.
#![forbid(unsafe_code)]

pub mod mgmt;
pub mod natpmp;
pub mod ovpn;
pub mod profiles;
pub mod wg;
