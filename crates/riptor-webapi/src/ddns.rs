//! Keeps a dynamic DNS name pointing at this computer, using the dyndns2 update protocol most
//! providers speak. The address published is the real one (the remote control isn't reachable
//! through the VPN).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riptor_core::Ddns;
use riptor_net::ledger::{record_via, Purpose, Route};
use tracing::{info, warn};

const EVERY: Duration = Duration::from_secs(600);

/// What a dyndns2 answer means: `Ok` to keep going, `Err` to stop until the settings change.
pub fn verdict(body: &str) -> Result<&'static str, String> {
    match body.split_whitespace().next().unwrap_or("") {
        "good" => Ok("updated"),
        "nochg" => Ok("already up to date"),
        "badauth" => Err("the provider refused the username or password".into()),
        "nohost" | "notfqdn" => Err("the provider doesn't know that domain name".into()),
        "abuse" => Err("the provider blocked updates for this name".into()),
        "911" | "dnserr" => Ok("the provider had a problem; trying again later"),
        other => Err(format!(
            "unexpected answer from the provider: {}",
            other.chars().take(40).collect::<String>()
        )),
    }
}

pub async fn run(cfg: Ddns, data_dir: PathBuf, note: Arc<Mutex<Option<String>>>) {
    let Ok(http) = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Riptor/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return;
    };
    let mut published = None;
    loop {
        match riptor_core::public_ip::lookup(None).await {
            Ok(ip) if Some(ip) != published => {
                let host = url::Url::parse(&cfg.update_url)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_default();
                let result = http
                    .get(&cfg.update_url)
                    .query(&[("hostname", cfg.domain.as_str()), ("myip", &ip.to_string())])
                    .basic_auth(&cfg.username, crate::auth::ddns_password(&data_dir))
                    .send()
                    .await;
                record_via(Purpose::DynamicDns, &host, Route::Direct, result.is_ok());
                let text = match result {
                    Ok(r) => r.text().await.unwrap_or_default(),
                    Err(e) => {
                        *note.lock() = Some(format!("couldn't reach the provider: {e}"));
                        tokio::time::sleep(EVERY).await;
                        continue;
                    }
                };
                match verdict(&text) {
                    Ok(msg) => {
                        info!("dynamic DNS for {}: {msg}", cfg.domain);
                        *note.lock() = Some(format!("{}: {msg} ({ip})", cfg.domain));
                        if msg != "the provider had a problem; trying again later" {
                            published = Some(ip);
                        }
                    }
                    Err(msg) => {
                        warn!("dynamic DNS stopped: {msg}");
                        *note.lock() = Some(msg);
                        return;
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                *note.lock() = Some(format!(
                    "couldn't find this computer's public address: {e:#}"
                ))
            }
        }
        tokio::time::sleep(EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_answers() {
        assert!(verdict("good 203.0.113.5").is_ok());
        assert!(verdict("nochg 203.0.113.5").is_ok());
        assert!(verdict("badauth").is_err());
        assert!(verdict("nohost").is_err());
        assert!(verdict("<html>").is_err());
        assert!(verdict("").is_err());
    }
}
