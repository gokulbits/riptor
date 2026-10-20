//! DNS lookups through the VPN tunnel.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hickory_resolver::config::{
    ConnectionConfig, NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::TokioResolver;

/// Public resolvers, reached through the tunnel.
pub const SERVERS: [SocketAddr; 2] = [
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53),
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)), 53),
];

#[derive(Clone)]
pub struct TunnelResolver {
    inner: TokioResolver,
}

impl TunnelResolver {
    pub fn new(bind: IpAddr, servers: &[SocketAddr]) -> anyhow::Result<Self> {
        let name_servers = servers
            .iter()
            .map(|s| {
                let connections = [ConnectionConfig::udp(), ConnectionConfig::tcp()]
                    .into_iter()
                    .map(|mut c| {
                        c.port = s.port();
                        c.bind_addr = Some(SocketAddr::new(bind, 0));
                        c
                    })
                    .collect();
                NameServerConfig::new(s.ip(), true, connections)
            })
            .collect();
        let cfg = ResolverConfig::from_name_servers(name_servers);
        let mut opts = ResolverOpts::default();
        opts.timeout = Duration::from_secs(4);
        opts.attempts = 2;
        opts.use_hosts_file = ResolveHosts::Never;
        let inner = TokioResolver::builder_with_config(cfg, TokioRuntimeProvider::default())
            .with_options(opts)
            .build()?;
        Ok(Self { inner })
    }

    pub async fn lookup(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let ips = self.inner.lookup_ip(host).await.map_err(io::Error::other)?;
        Ok(ips.iter().map(|ip| SocketAddr::new(ip, port)).collect())
    }

    /// For the patched tracker and DHT crates.
    pub fn hook(&self) -> librqbit_tracker_comms::resolve_hook::Resolver {
        let me = self.clone();
        Arc::new(move |host, port| {
            let me = me.clone();
            Box::pin(async move { me.lookup(&host, port).await })
        })
    }
}

impl reqwest::dns::Resolve for TunnelResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let me = self.clone();
        Box::pin(async move {
            let addrs = me.lookup(name.as_str(), 0).await?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// `None` goes back to the system resolver.
pub fn install(r: Option<&TunnelResolver>) {
    let hook = r.map(TunnelResolver::hook);
    librqbit_tracker_comms::resolve_hook::set_resolver(hook.clone());
    librqbit_dht::resolve_hook::set_resolver(hook);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-answer DNS server on loopback: every A query gets 203.0.113.9.
    async fn fake_dns() -> SocketAddr {
        let sock = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                    return;
                };
                let q = &buf[..n];
                let mut r = Vec::from(&q[..2]); // id
                r.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
                // The question only, not the EDNS record that follows it.
                let mut end = 12;
                while end < n && q[end] != 0 {
                    end += 1 + q[end] as usize;
                }
                r.extend_from_slice(&q[12..(end + 5).min(n)]);
                r.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 203, 0, 113, 9]);
                let _ = sock.send_to(&r, from).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn queries_go_to_the_given_servers_from_the_bound_address() {
        let server = fake_dns().await;
        let r = TunnelResolver::new(IpAddr::V4(Ipv4Addr::LOCALHOST), &[server]).unwrap();
        let got = r.lookup("tracker.example", 6969).await.unwrap();
        assert_eq!(got, vec!["203.0.113.9:6969".parse().unwrap()]);
        // Literal addresses never hit the network.
        assert_eq!(
            r.lookup("192.0.2.1", 80).await.unwrap(),
            vec!["192.0.2.1:80".parse().unwrap()]
        );
    }

    #[tokio::test]
    async fn the_hook_reaches_the_patched_crates() {
        let server = fake_dns().await;
        let r = TunnelResolver::new(IpAddr::V4(Ipv4Addr::LOCALHOST), &[server]).unwrap();
        install(Some(&r));
        let got = librqbit_dht::resolve_hook::lookup("router.example", 6881)
            .await
            .unwrap();
        install(None);
        assert_eq!(got, vec!["203.0.113.9:6881".parse().unwrap()]);
    }
}
