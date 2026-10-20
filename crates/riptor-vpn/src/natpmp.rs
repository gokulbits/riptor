//! NAT-PMP (RFC 6886) port forwarding through the VPN gateway. Mappings
//! expire and must be renewed.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{bail, ensure, Context};

pub const DEFAULT_GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 2, 0, 1);
pub const PORT: u16 = 5351;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Udp = 1,
    Tcp = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub proto: Proto,
    pub internal_port: u16,
    pub external_port: u16,
    pub lifetime: u32,
}

pub fn encode_request(
    proto: Proto,
    internal_port: u16,
    suggested_external: u16,
    lifetime: u32,
) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[1] = proto as u8;
    b[4..6].copy_from_slice(&internal_port.to_be_bytes());
    b[6..8].copy_from_slice(&suggested_external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime.to_be_bytes());
    b
}

pub fn decode_response(b: &[u8], proto: Proto, internal_port: u16) -> anyhow::Result<Mapping> {
    ensure!(
        b.len() == 16,
        "unexpected NAT-PMP response length {}",
        b.len()
    );
    ensure!(b[0] == 0, "unsupported NAT-PMP version {}", b[0]);
    ensure!(
        b[1] == 128 + proto as u8,
        "response for a different request"
    );
    let code = u16::from_be_bytes([b[2], b[3]]);
    if code != 0 {
        bail!(match code {
            1 => "the gateway does not support NAT-PMP version 0".to_string(),
            2 => "port forwarding is not allowed on this server (pick a server that supports port forwarding)".to_string(),
            3 => "the gateway has no network connection".to_string(),
            4 => "the gateway is out of ports".to_string(),
            c => format!("NAT-PMP error {c}"),
        });
    }
    let internal = u16::from_be_bytes([b[8], b[9]]);
    ensure!(internal == internal_port, "response for a different port");
    let external = u16::from_be_bytes([b[10], b[11]]);
    ensure!(external != 0, "gateway returned port 0");
    Ok(Mapping {
        proto,
        internal_port,
        external_port: external,
        lifetime: u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
    })
}

/// Bound to `local` so the request goes through the tunnel.
pub async fn map(
    gateway: SocketAddr,
    local: Ipv4Addr,
    proto: Proto,
    internal_port: u16,
    lifetime: u32,
) -> anyhow::Result<Mapping> {
    let sock = tokio::net::UdpSocket::bind((local, 0))
        .await
        .context("binding to the VPN address")?;
    sock.connect(gateway).await?;
    let req = encode_request(proto, internal_port, 0, lifetime);
    let mut wait = Duration::from_millis(250);
    let mut buf = [0u8; 64];
    for _ in 0..4 {
        sock.send(&req).await?;
        match tokio::time::timeout(wait, sock.recv(&mut buf)).await {
            Ok(Ok(n)) => return decode_response(&buf[..n], proto, internal_port),
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => wait *= 2,
        }
    }
    bail!("no answer from the VPN's port-forwarding gateway")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(proto: Proto, code: u16, internal: u16, external: u16, life: u32) -> Vec<u8> {
        let mut b = vec![0u8, 128 + proto as u8];
        b.extend(code.to_be_bytes());
        b.extend(1234u32.to_be_bytes());
        b.extend(internal.to_be_bytes());
        b.extend(external.to_be_bytes());
        b.extend(life.to_be_bytes());
        b
    }

    #[test]
    fn codec() {
        assert_eq!(
            encode_request(Proto::Tcp, 6881, 0, 60),
            [0, 2, 0, 0, 0x1a, 0xe1, 0, 0, 0, 0, 0, 60]
        );
        let m =
            decode_response(&response(Proto::Tcp, 0, 6881, 41234, 60), Proto::Tcp, 6881).unwrap();
        assert_eq!((m.external_port, m.lifetime), (41234, 60));
        assert!(
            decode_response(&response(Proto::Udp, 0, 6881, 41234, 60), Proto::Tcp, 6881).is_err()
        );
        assert!(
            decode_response(&response(Proto::Tcp, 0, 1111, 41234, 60), Proto::Tcp, 6881).is_err()
        );
        assert!(decode_response(&response(Proto::Tcp, 0, 6881, 0, 60), Proto::Tcp, 6881).is_err());
        let e =
            decode_response(&response(Proto::Tcp, 2, 6881, 0, 0), Proto::Tcp, 6881).unwrap_err();
        assert!(e.to_string().contains("not allowed on this server"));
        assert!(decode_response(&[0u8; 15], Proto::Tcp, 6881).is_err());
    }

    #[tokio::test]
    async fn client_round_trip() {
        let gw = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = gw.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 12];
            let (_, _) = gw.recv_from(&mut buf).await.unwrap();
            let (_, from) = gw.recv_from(&mut buf).await.unwrap();
            let internal = u16::from_be_bytes([buf[4], buf[5]]);
            gw.send_to(&response(Proto::Udp, 0, internal, 50000, 60), from)
                .await
                .unwrap();
        });
        let m = map(addr, Ipv4Addr::LOCALHOST, Proto::Udp, 6881, 60)
            .await
            .unwrap();
        assert_eq!(m.external_port, 50000);
    }
}
