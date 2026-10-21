// RIPTOR PATCH: tells Riptor which address each peer says it sees us on (BEP 10 `yourip`).

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

/// (peer, the address that peer saw us on).
pub type YourIpObserver = Arc<dyn Fn(IpAddr, IpAddr) + Send + Sync>;

static OBSERVER: RwLock<Option<YourIpObserver>> = RwLock::new(None);

pub fn set_yourip_observer(o: Option<YourIpObserver>) {
    *OBSERVER.write().unwrap_or_else(|e| e.into_inner()) = o;
}

pub(crate) fn yourip(peer: IpAddr, seen: IpAddr) {
    let o = OBSERVER.read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(o) = o {
        o(peer, seen);
    }
}
