//! A saved DHT port that's no longer usable must not stop Riptor from starting.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use librqbit::dht::{DhtConfig, DhtState};

fn config(addr: SocketAddr, remembered: bool) -> DhtConfig<'static> {
    DhtConfig {
        listen_addr: Some(addr),
        // Nothing to reach, and nothing leaves loopback.
        bootstrap_addrs: Some(Vec::new()),
        remembered_port: remembered,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_busy_remembered_port_falls_back_to_another() {
    let held = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let busy = held.local_addr().unwrap();

    let dht = DhtState::with_config(config(busy, true)).await.unwrap();
    assert_ne!(dht.listen_addr().port(), busy.port());
    assert_ne!(dht.listen_addr().port(), 0);
    dht.cancellation_token().cancel();
}

#[tokio::test]
async fn a_port_the_user_chose_still_fails_loudly() {
    let held = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let busy = held.local_addr().unwrap();
    assert!(DhtState::with_config(config(busy, false)).await.is_err());
}

#[tokio::test]
async fn a_free_remembered_port_is_kept() {
    let free = {
        let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        s.local_addr().unwrap()
    };
    let dht = DhtState::with_config(config(free, true)).await.unwrap();
    assert_eq!(dht.listen_addr().port(), free.port());
    dht.cancellation_token().cancel();
}
