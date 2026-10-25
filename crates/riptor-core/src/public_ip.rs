//! Our public address, from an IP-echo service (through the tunnel on the VPN).

use std::net::IpAddr;
use std::time::Duration;

use anyhow::{bail, Context};
use futures::StreamExt;

const SERVICES: [&str; 2] = ["https://api.ipify.org", "https://ipv4.icanhazip.com"];
const MAX_REPLY: usize = 64;

pub fn parse(body: &[u8]) -> anyhow::Result<IpAddr> {
    anyhow::ensure!(body.len() <= MAX_REPLY, "reply too long");
    let text = std::str::from_utf8(body)
        .context("reply is not text")?
        .trim();
    text.parse().context("reply is not an IP address")
}

pub async fn lookup(local: Option<&riptor_net::TunnelEnd>) -> anyhow::Result<IpAddr> {
    let mut b = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent("Riptor");
    if let Some(end) = local {
        let resolver = crate::dns::TunnelResolver::new(end, &crate::dns::SERVERS)?;
        b = end.http(b).dns_resolver(std::sync::Arc::new(resolver));
    }
    let client = b.build()?;
    let mut last = None;
    let route = if local.is_some() {
        riptor_net::ledger::Route::Tunnel
    } else {
        riptor_net::ledger::Route::Direct
    };
    for url in SERVICES {
        let got = fetch(&client, url).await;
        riptor_net::ledger::record_via(
            riptor_net::ledger::Purpose::IpCheck,
            url,
            route,
            got.is_ok(),
        );
        match got {
            Ok(ip) => return Ok(ip),
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e),
        None => bail!("no IP service configured"),
    }
}

async fn fetch(client: &reqwest::Client, url: &str) -> anyhow::Result<IpAddr> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        anyhow::ensure!(body.len() <= MAX_REPLY, "reply too long");
    }
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_must_be_a_bare_ip() {
        assert_eq!(parse(b"203.0.113.7\n").unwrap().to_string(), "203.0.113.7");
        assert_eq!(parse(b" 2001:db8::1 ").unwrap().to_string(), "2001:db8::1");
        for bad in [
            &b""[..],
            b"<html>blocked</html>",
            b"203.0.113.7; rm -rf /",
            b"\xff\xfe",
            &[b'1'; 200],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
