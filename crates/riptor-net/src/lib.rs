//! Addresses untrusted links may make us contact: public ones only.

#![forbid(unsafe_code)]

use std::net::IpAddr;

use url::{Host, Url};

pub mod ledger;

pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b)) // NAT64 to anything
        }
    }
}

/// Pre-DNS check; names are checked again once resolved.
pub fn host_allowed(u: &Url) -> bool {
    match u.host() {
        Some(Host::Ipv4(ip)) => is_public(ip.into()),
        Some(Host::Ipv6(ip)) => is_public(ip.into()),
        Some(Host::Domain(d)) => d.parse::<IpAddr>().map_or(true, is_public),
        None => false,
    }
}

/// Loopback or LAN by address, so the request never leaves this network.
pub fn stays_local(u: &Url) -> bool {
    match u.host() {
        Some(Host::Ipv4(ip)) => !is_public(ip.into()),
        Some(Host::Ipv6(ip)) => !is_public(ip.into()),
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts_are_recognised() {
        for u in [
            "http://127.0.0.1:9117/api",
            "http://192.168.1.5/torznab",
            "http://[::1]:9696/",
            "http://localhost:9117/",
            "http://jackett.localhost./",
        ] {
            assert!(stays_local(&Url::parse(u).unwrap()), "{u}");
        }
        for u in [
            "https://archive.org/",
            "https://8.8.8.8/",
            "https://localhost.example.com/",
            "https://10.0.0.1.example.com/",
        ] {
            assert!(!stays_local(&Url::parse(u).unwrap()), "{u}");
        }
    }

    #[test]
    fn private_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn literal_hosts_are_checked() {
        for bad in [
            "http://127.0.0.1/x",
            "http://[::1]/",
            "http://10.0.0.1:8080/",
        ] {
            assert!(!host_allowed(&Url::parse(bad).unwrap()), "{bad}");
        }
        assert!(host_allowed(&Url::parse("https://example.org/a").unwrap()));
        assert!(host_allowed(&Url::parse("http://93.184.216.34/").unwrap()));
    }
}
