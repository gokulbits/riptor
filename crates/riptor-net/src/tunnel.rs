//! Sockets pinned to the VPN tunnel: its address, and on Linux (which routes by destination) its interface.

use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::net::{TcpSocket, UdpSocket};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelEnd {
    pub ip: IpAddr,
    /// The tunnel's interface, such as `tun0`.
    pub device: String,
}

impl TunnelEnd {
    pub fn new(ip: IpAddr, device: impl Into<String>) -> Self {
        Self {
            ip,
            device: device.into(),
        }
    }

    /// A TCP socket, not yet connected, that can only leave through the tunnel.
    pub fn tcp_socket(&self) -> io::Result<TcpSocket> {
        let s = if self.ip.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        #[cfg(target_os = "linux")]
        s.bind_device(Some(self.device.as_bytes()))?;
        s.bind(SocketAddr::new(self.ip, 0))?;
        Ok(s)
    }

    /// A UDP socket on `port` (0 for any) that can only leave through the tunnel.
    pub async fn udp_socket(&self, port: u16) -> io::Result<UdpSocket> {
        let s = UdpSocket::bind(SocketAddr::new(self.ip, port)).await?;
        #[cfg(target_os = "linux")]
        s.bind_device(Some(self.device.as_bytes()))?;
        Ok(s)
    }

    /// An HTTP client builder whose connections can only leave through the tunnel.
    pub fn http(&self, b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        let b = b.local_address(self.ip);
        #[cfg(target_os = "linux")]
        let b = b.interface(&self.device);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn loopback() -> TunnelEnd {
        TunnelEnd::new(IpAddr::V4(Ipv4Addr::LOCALHOST), "lo")
    }

    #[tokio::test]
    async fn sockets_use_the_tunnel_address() {
        let end = loopback();
        let udp = end.udp_socket(0).await.unwrap();
        assert_eq!(udp.local_addr().unwrap().ip(), end.ip);

        let server = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let to = server.local_addr().unwrap();
        let tcp = end.tcp_socket().unwrap().connect(to).await.unwrap();
        assert_eq!(tcp.local_addr().unwrap().ip(), end.ip);
    }

    // With the tunnel gone, sockets fail instead of falling back to the normal connection.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_missing_tunnel_interface_is_refused() {
        let gone = TunnelEnd::new(IpAddr::V4(Ipv4Addr::LOCALHOST), "riptor-none");
        assert!(gone.tcp_socket().is_err());
        assert!(gone.udp_socket(0).await.is_err());
    }
}
