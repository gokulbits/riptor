//! Saved VPN profiles.
//!
//! A profile is stored as its *sanitized* config (see [`crate::ovpn`] and
//! [`crate::wg`]) in `<data>/vpn/profiles/<id>.json`. Secrets never go in that
//! file: OpenVPN usernames/passwords and WireGuard private/preshared keys live
//! in the OS credential store and are only put together with the config in
//! [`ProfileStore::materialize`], right before connecting.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context};
use serde::{Deserialize, Serialize};

use crate::{ovpn, wg};

const SERVICE: &str = "Riptor";
const PRIVATE_KEY_SLOT: &str = "{{riptor:private-key}}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    OpenVpn,
    WireGuard,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Sanitized config with secrets replaced by slots.
    pub config: String,
    pub needs_credentials: bool,
    pub servers: Vec<String>,
    /// Options removed on import (shown to the user).
    pub removed: Vec<String>,
    /// DNS servers from the profile (WireGuard), used only for Riptor's own lookups.
    #[serde(default)]
    pub dns: Vec<std::net::IpAddr>,
    pub created: u64,
}

impl Profile {
    /// Whether this profile can run app-only on this platform.
    pub fn app_only_supported(&self) -> bool {
        match self.kind {
            Kind::OpenVpn => true,
            Kind::WireGuard => wg::app_only_supported(),
        }
    }
}

fn entry(id: &str, what: &str) -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, &format!("vpn:{id}:{what}"))?)
}

fn store_secret(id: &str, what: &str, value: &str) -> anyhow::Result<()> {
    entry(id, what)?
        .set_password(value)
        .map_err(|e| anyhow::anyhow!("could not save to the system credential store: {e}"))
}

fn secret(id: &str, what: &str) -> anyhow::Result<String> {
    entry(id, what)?
        .get_password()
        .map_err(|e| anyhow::anyhow!("missing from the system credential store ({what}): {e}"))
}

fn delete_secret(id: &str, what: &str) {
    if let Ok(e) = entry(id, what) {
        let _ = e.delete_credential();
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join("vpn").join("profiles");
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> anyhow::Result<PathBuf> {
        ensure!(
            id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid profile id"
        );
        Ok(self.dir.join(format!("{id}.json")))
    }

    pub fn list(&self) -> Vec<Profile> {
        let mut v: Vec<Profile> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        v.sort_by_key(|p| p.created);
        v
    }

    pub fn get(&self, id: &str) -> anyhow::Result<Profile> {
        let bytes = std::fs::read(self.path(id)?).context("unknown VPN profile")?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Import an untrusted profile file. The kind is detected from the content.
    pub fn import(&self, name: &str, bytes: &[u8]) -> anyhow::Result<Profile> {
        let name: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect::<String>()
            .trim()
            .to_string();
        ensure!(!name.is_empty(), "give the profile a name");
        let text = std::str::from_utf8(bytes).context("profile is not text")?;
        let id = {
            let mut b = [0u8; 8];
            getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        };
        let profile = if text
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case("[interface]"))
        {
            let s = wg::sanitize(bytes)?;
            // Move keys to the credential store; keep slots in the saved config.
            let mut config = String::new();
            let mut psk = 0;
            for line in s.config.lines() {
                if let Some(k) = line.strip_prefix("PrivateKey = ") {
                    store_secret(&id, "wg-private-key", k)?;
                    config.push_str(&format!("PrivateKey = {PRIVATE_KEY_SLOT}\n"));
                } else if let Some(k) = line.strip_prefix("PresharedKey = ") {
                    store_secret(&id, &format!("wg-psk-{psk}"), k)?;
                    config.push_str(&format!("PresharedKey = {{{{riptor:psk-{psk}}}}}\n"));
                    psk += 1;
                } else {
                    config.push_str(line);
                    config.push('\n');
                }
            }
            Profile {
                id: id.clone(),
                name,
                kind: Kind::WireGuard,
                config,
                needs_credentials: false,
                servers: s.endpoints,
                removed: vec![],
                dns: s.dns,
                created: now(),
            }
        } else {
            let s = ovpn::sanitize(bytes)?;
            Profile {
                id: id.clone(),
                name,
                kind: Kind::OpenVpn,
                config: s.config,
                needs_credentials: s.needs_credentials,
                servers: s
                    .remotes
                    .iter()
                    .map(|r| format!("{}:{}", r.host, r.port))
                    .collect(),
                removed: s.removed,
                dns: vec![],
                created: now(),
            }
        };
        let tmp = self.dir.join(format!("{id}.json.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&profile)?)?;
        std::fs::rename(tmp, self.path(&id)?)?;
        Ok(profile)
    }

    pub fn set_credentials(&self, id: &str, username: &str, password: &str) -> anyhow::Result<()> {
        let p = self.get(id)?;
        ensure!(
            p.kind == Kind::OpenVpn,
            "only OpenVPN profiles use a username and password"
        );
        ensure!(
            !username.is_empty() && username.len() <= 256,
            "enter a username"
        );
        ensure!(
            !password.is_empty() && password.len() <= 1024,
            "enter a password"
        );
        ensure!(
            crate::mgmt::credentials(username, password).is_some(),
            "username or password contains a line break"
        );
        store_secret(id, "username", username)?;
        store_secret(id, "password", password)
    }

    pub fn has_credentials(&self, id: &str) -> bool {
        secret(id, "username").is_ok() && secret(id, "password").is_ok()
    }

    pub fn credentials(&self, id: &str) -> anyhow::Result<(String, String)> {
        Ok((secret(id, "username")?, secret(id, "password")?))
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let p = self.get(id)?;
        for what in ["username", "password", "wg-private-key"] {
            delete_secret(id, what);
        }
        for n in 0..16 {
            delete_secret(id, &format!("wg-psk-{n}"));
        }
        std::fs::remove_file(self.path(&p.id)?)?;
        Ok(())
    }

    /// The full config to hand to the helper, secrets included. Never stored.
    pub fn materialize(&self, id: &str) -> anyhow::Result<String> {
        let p = self.get(id)?;
        match p.kind {
            Kind::OpenVpn => {
                // Re-validate: the saved file could have been edited.
                let s = ovpn::sanitize(p.config.as_bytes())?;
                Ok(s.config)
            }
            Kind::WireGuard => {
                let mut out = String::new();
                for line in p.config.lines() {
                    if line.contains(PRIVATE_KEY_SLOT) {
                        out.push_str(&format!("PrivateKey = {}\n", secret(id, "wg-private-key")?));
                    } else if let Some(rest) = line.strip_prefix("PresharedKey = {{riptor:psk-") {
                        let n = rest.trim_end_matches("}}");
                        out.push_str(&format!(
                            "PresharedKey = {}\n",
                            secret(id, &format!("wg-psk-{n}"))?
                        ));
                    } else {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                let s = wg::sanitize(out.as_bytes())?;
                if !wg::app_only_supported() {
                    bail!(
                        "WireGuard can't run app-only on this system yet; use an OpenVPN profile"
                    );
                }
                Ok(s.config)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVPN: &str = "client\ndev tun\nremote vpn.example 1194 udp\nauth-user-pass\nup /tmp/evil.sh\n<ca>\n-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n</ca>\n";

    #[test]
    fn import_list_materialize_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = ProfileStore::new(d.path()).unwrap();
        let p = s.import("  Proton NL\u{7}  ", OVPN.as_bytes()).unwrap();
        assert_eq!(p.name, "Proton NL");
        assert_eq!(p.kind, Kind::OpenVpn);
        assert!(p.needs_credentials && p.app_only_supported());
        assert_eq!(p.removed, vec!["up"]);
        assert_eq!(s.list().len(), 1);
        let cfg = s.materialize(&p.id).unwrap();
        assert!(cfg.contains("route-nopull") && !cfg.contains("evil"));

        // A saved profile edited on disk to add a script is re-validated.
        let path = d.path().join("vpn/profiles").join(format!("{}.json", p.id));
        let mut saved: Profile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        saved.config.push_str("plugin /tmp/evil.so\n");
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(!s.materialize(&p.id).unwrap().contains("evil.so"));

        s.delete(&p.id).unwrap();
        assert!(s.list().is_empty());
    }

    #[test]
    fn rejects_bad_ids_and_inputs() {
        let d = tempfile::tempdir().unwrap();
        let s = ProfileStore::new(d.path()).unwrap();
        assert!(s.get("../../etc/passwd").is_err());
        assert!(s.get("0123456789abcdeg").is_err());
        assert!(s.import("", OVPN.as_bytes()).is_err());
        assert!(s.import("x", b"plugin /evil\n").is_err());
        assert!(s.import("x", &[0xff, 0xfe]).is_err());
    }
}
