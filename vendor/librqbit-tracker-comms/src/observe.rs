// RIPTOR PATCH: tells Riptor about each announce: swarm size, the address the tracker saw, the host.

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use librqbit_core::hash_id::Id20;

#[derive(Clone, Debug)]
pub struct Announce {
    pub info_hash: Id20,
    pub host: String,
    pub ok: bool,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    /// BEP 24: the address the tracker saw the request come from.
    pub external_ip: Option<IpAddr>,
}

pub type Observer = Arc<dyn Fn(&Announce) + Send + Sync>;

static OBSERVER: RwLock<Option<Observer>> = RwLock::new(None);

pub fn set_observer(o: Option<Observer>) {
    *OBSERVER.write().unwrap_or_else(|e| e.into_inner()) = o;
}

pub(crate) fn report(a: Announce) {
    let o = OBSERVER.read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(o) = o {
        o(&a);
    }
}

pub(crate) fn failed(info_hash: Id20, host: &str) {
    report(Announce {
        info_hash,
        host: host.to_string(),
        ok: false,
        seeders: None,
        leechers: None,
        external_ip: None,
    });
}

pub(crate) fn ip_from_bytes(b: &[u8]) -> Option<IpAddr> {
    match b.len() {
        4 => Some(IpAddr::from(<[u8; 4]>::try_from(b).ok()?)),
        16 => Some(IpAddr::from(<[u8; 16]>::try_from(b).ok()?)),
        _ => None,
    }
}
