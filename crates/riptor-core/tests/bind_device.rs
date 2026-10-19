//! Adapter binding, the basis of the app-only VPN and its kill switch.
//! Everything binds to loopback, so no firewall prompt is triggered.

use std::net::{Ipv4Addr, SocketAddr};

use librqbit_dualstack_sockets::{tcp_connect, BindDevice, ConnectOpts};
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};

/// Name of the adapter that carries 127.0.0.1 ("Loopback Pseudo-Interface 1"
/// on Windows, "lo" on Linux).
fn loopback() -> NetworkInterface {
    NetworkInterface::show()
        .unwrap()
        .into_iter()
        .find(|i| {
            i.addr
                .iter()
                .any(|a| matches!(a, Addr::V4(v) if v.ip.is_loopback()))
        })
        .expect("a loopback adapter")
}

#[tokio::test]
async fn binds_sockets_to_an_adapter() {
    let lo = loopback();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });

    // "#<index>" is the Windows-only form (Riptor's socket patch); Linux
    // binds by name only.
    let mut names = vec![lo.name.clone()];
    if cfg!(windows) {
        names.push(format!("#{}", lo.index));
    }
    for name in names {
        let bd =
            BindDevice::new_from_name(&name).unwrap_or_else(|e| panic!("binding to {name:?}: {e}"));
        assert_eq!(bd.index().get(), lo.index);
        let stream = tcp_connect(
            addr,
            ConnectOpts {
                source_port: None,
                bind_device: Some(&bd),
            },
        )
        .await;
        assert!(
            stream.is_ok(),
            "connect through {name:?} failed: {:?}",
            stream.err()
        );
    }
    accept.await.unwrap().unwrap();
}

#[test]
fn unknown_adapters_are_refused() {
    assert!(BindDevice::new_from_name("no-such-adapter-riptor").is_err());
    assert!(BindDevice::new_from_name("#0").is_err());
}
