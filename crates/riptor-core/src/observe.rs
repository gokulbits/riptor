//! Hooks Riptor's bookkeeping into the vendored tracker client.

use std::sync::{Arc, Once};

use librqbit_tracker_comms::observe::{set_observer, Announce};

fn on_announce(a: &Announce) {
    crate::swarm::record(a);
    riptor_net::ledger::record(riptor_net::ledger::Purpose::Tracker, &a.host, a.ok);
    if let Some(ip) = a.external_ip {
        crate::seen_ip::record_tracker(&a.host, ip);
    }
}

/// Once per process; every engine shares it.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        set_observer(Some(Arc::new(on_announce)));
        librqbit::riptor_observe::set_yourip_observer(Some(Arc::new(crate::seen_ip::record_peer)));
    });
}
