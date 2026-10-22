//! Remote-control secrets in their own owner-only file, never in settings or backups: the
//! password (Argon2id), the API key (SHA-256) and the dynamic DNS password. Also sessions and
//! bans after repeated failures.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

pub const FILE: &str = "webui-auth.json";
pub const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 256;
pub const MAX_FAILURES: u32 = 5;
const MAX_SESSIONS: usize = 32;
const MAX_TRACKED_IPS: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    salt: String,
    hash: String,
    m_cost: u32,
    t_cost: u32,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Secrets {
    password: Option<Stored>,
    /// SHA-256 of the API key, hex; the key itself is shown once.
    api_key: Option<String>,
    ddns_password: Option<String>,
}

fn read_secrets(path: &Path) -> Secrets {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_secrets(data_dir: &Path, s: &Secrets) -> anyhow::Result<()> {
    riptor_core::state_file::save(&data_dir.join(FILE), s)
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A new API key, returned once; clients send it as `Authorization: Bearer <key>`.
pub fn new_api_key(data_dir: &Path) -> anyhow::Result<String> {
    let mut raw = [0u8; 24];
    rand::fill(&mut raw);
    let key = format!(
        "rpt_{}",
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let mut s = read_secrets(&data_dir.join(FILE));
    s.api_key = Some(sha256_hex(&key));
    write_secrets(data_dir, &s)?;
    Ok(key)
}

pub fn remove_api_key(data_dir: &Path) -> anyhow::Result<()> {
    let mut s = read_secrets(&data_dir.join(FILE));
    s.api_key = None;
    write_secrets(data_dir, &s)
}

pub fn has_api_key(data_dir: &Path) -> bool {
    read_secrets(&data_dir.join(FILE)).api_key.is_some()
}

pub fn set_ddns_password(data_dir: &Path, password: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        password.len() <= 256 && !password.contains(['\r', '\n']),
        "that password is too long"
    );
    let mut s = read_secrets(&data_dir.join(FILE));
    s.ddns_password = (!password.is_empty()).then(|| password.to_string());
    write_secrets(data_dir, &s)
}

pub fn ddns_password(data_dir: &Path) -> Option<String> {
    read_secrets(&data_dir.join(FILE)).ddns_password
}

/// Ban and session limits from the settings.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// 0 never bans.
    pub max_failures: u32,
    pub ban: Duration,
    pub idle: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_failures: MAX_FAILURES,
            ban: Duration::from_secs(3600),
            idle: Duration::from_secs(3600),
        }
    }
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn derive(password: &str, salt: &[u8], m_cost: u32, t_cost: u32) -> anyhow::Result<[u8; 32]> {
    let params = Params::new(m_cost, t_cost, 1, Some(32)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(out)
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Stores a new password; existing sessions end because the server restarts on the change.
pub fn set_password(data_dir: &Path, password: &str) -> anyhow::Result<()> {
    let n = password.chars().count();
    anyhow::ensure!(
        (MIN_PASSWORD..=MAX_PASSWORD).contains(&n),
        "use a password of {MIN_PASSWORD} to {MAX_PASSWORD} characters"
    );
    let mut salt = [0u8; 16];
    rand::fill(&mut salt);
    // OWASP baseline, as for backups.
    let (m_cost, t_cost) = (64 * 1024, 3);
    let hash = derive(password, &salt, m_cost, t_cost)?;
    let mut s = read_secrets(&data_dir.join(FILE));
    s.password = Some(Stored {
        salt: b64().encode(salt),
        hash: b64().encode(hash),
        m_cost,
        t_cost,
    });
    write_secrets(data_dir, &s)
}

pub fn has_password(data_dir: &Path) -> bool {
    load(&data_dir.join(FILE)).is_some()
}

fn load(path: &Path) -> Option<Stored> {
    let s = read_secrets(path).password?;
    // Only parameters Riptor itself writes; a forged file can't make each check expensive.
    let sane = (8 * 1024..=256 * 1024).contains(&s.m_cost) && (1..=10).contains(&s.t_cost);
    sane.then_some(s)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Login {
    Ok(String),
    Fails,
    Banned,
}

pub struct Auth {
    file: PathBuf,
    username: String,
    policy: Policy,
    api_key: Option<String>,
    sessions: Mutex<HashMap<String, Instant>>,
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    // Argon2 is meant to be slow; two at a time keeps it from tying up the CPU.
    checks: Arc<Semaphore>,
}

impl Auth {
    pub fn new(data_dir: &Path, username: &str, policy: Policy) -> Self {
        Self {
            file: data_dir.join(FILE),
            username: username.to_string(),
            policy,
            api_key: read_secrets(&data_dir.join(FILE)).api_key,
            sessions: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            checks: Arc::new(Semaphore::new(2)),
        }
    }

    pub fn banned(&self, ip: IpAddr) -> bool {
        let mut f = self.failures.lock();
        match f.get(&ip) {
            Some((n, since)) if self.policy.max_failures > 0 && *n >= self.policy.max_failures => {
                if since.elapsed() < self.policy.ban {
                    return true;
                }
                f.remove(&ip);
                false
            }
            _ => false,
        }
    }

    fn fail(&self, ip: IpAddr) {
        let mut f = self.failures.lock();
        if f.len() >= MAX_TRACKED_IPS {
            f.retain(|_, (_, since)| since.elapsed() < self.policy.ban);
            if f.len() >= MAX_TRACKED_IPS {
                f.clear();
            }
        }
        let e = f.entry(ip).or_insert((0, Instant::now()));
        e.0 += 1;
        e.1 = Instant::now();
    }

    pub async fn login(&self, ip: IpAddr, username: &str, password: &str) -> Login {
        if self.banned(ip) {
            return Login::Banned;
        }
        let ok = match (load(&self.file), password.len() <= MAX_PASSWORD * 4) {
            (Some(stored), true) => {
                let Ok(_slot) = self.checks.clone().acquire_owned().await else {
                    return Login::Fails;
                };
                let password = password.to_string();
                let check = tokio::task::spawn_blocking(move || {
                    let salt = b64().decode(&stored.salt).ok()?;
                    let want = b64().decode(&stored.hash).ok()?;
                    let got = derive(&password, &salt, stored.m_cost, stored.t_cost).ok()?;
                    Some(same(&got, &want))
                })
                .await;
                // Both are always checked, so timing doesn't say which was wrong.
                matches!(check, Ok(Some(true)))
                    & same(username.as_bytes(), self.username.as_bytes())
            }
            _ => false,
        };
        if !ok {
            self.fail(ip);
            return Login::Fails;
        }
        self.failures.lock().remove(&ip);
        Login::Ok(self.new_session())
    }

    /// A session without a password check, for clients whose authentication is bypassed.
    pub fn new_session(&self) -> String {
        let mut id = [0u8; 16];
        rand::fill(&mut id);
        let sid: String = id.iter().map(|b| format!("{b:02x}")).collect();
        let mut s = self.sessions.lock();
        let idle = self.policy.idle;
        s.retain(|_, seen| seen.elapsed() < idle);
        if s.len() >= MAX_SESSIONS {
            if let Some(oldest) = s
                .iter()
                .min_by_key(|(_, seen)| **seen)
                .map(|(k, _)| k.clone())
            {
                s.remove(&oldest);
            }
        }
        s.insert(sid.clone(), Instant::now());
        sid
    }

    /// True for the API key, compared by its hash.
    pub fn api_key_ok(&self, key: &str) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|want| same(sha256_hex(key).as_bytes(), want.as_bytes()))
    }

    /// True for a live session, which this also keeps alive.
    pub fn check(&self, sid: &str) -> bool {
        let mut s = self.sessions.lock();
        match s.get_mut(sid) {
            Some(seen) if seen.elapsed() < self.policy.idle => {
                *seen = Instant::now();
                true
            }
            Some(_) => {
                s.remove(sid);
                false
            }
            None => false,
        }
    }

    pub fn logout(&self, sid: &str) {
        self.sessions.lock().remove(sid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logins_sessions_and_bans() {
        let d = tempfile::tempdir().unwrap();
        assert!(set_password(d.path(), "short").is_err());
        assert!(!has_password(d.path()));
        set_password(d.path(), "correct horse").unwrap();
        assert!(has_password(d.path()));
        let auth = Auth::new(d.path(), "admin", Policy::default());
        let me: IpAddr = "192.168.1.5".parse().unwrap();

        let Login::Ok(sid) = auth.login(me, "admin", "correct horse").await else {
            panic!("right password refused")
        };
        assert!(auth.check(&sid));
        assert!(!auth.check("not-a-session"));
        auth.logout(&sid);
        assert!(!auth.check(&sid));

        assert_eq!(auth.login(me, "Admin", "correct horse").await, Login::Fails);
        for _ in 1..MAX_FAILURES {
            assert_eq!(auth.login(me, "admin", "wrong").await, Login::Fails);
        }
        // Banned now, even with the right password; other addresses aren't.
        assert_eq!(
            auth.login(me, "admin", "correct horse").await,
            Login::Banned
        );
        let other: IpAddr = "192.168.1.6".parse().unwrap();
        assert!(matches!(
            auth.login(other, "admin", "correct horse").await,
            Login::Ok(_)
        ));
    }

    #[test]
    fn api_keys_are_shown_once_and_stored_hashed() {
        let d = tempfile::tempdir().unwrap();
        set_password(d.path(), "correct horse").unwrap();
        let key = new_api_key(d.path()).unwrap();
        let file = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert!(!file.contains(&key), "the key is stored in plain text");
        let auth = Auth::new(d.path(), "admin", Policy::default());
        assert!(auth.api_key_ok(&key));
        assert!(!auth.api_key_ok("rpt_wrong"));
        // The password survives key changes, and removing the key works.
        assert!(has_password(d.path()));
        remove_api_key(d.path()).unwrap();
        assert!(!Auth::new(d.path(), "admin", Policy::default()).api_key_ok(&key));
        set_ddns_password(d.path(), "dns-secret").unwrap();
        assert_eq!(ddns_password(d.path()).as_deref(), Some("dns-secret"));
        assert!(has_password(d.path()));
    }

    #[tokio::test]
    async fn forged_password_files_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let auth = Auth::new(d.path(), "admin", Policy::default());
        let ip: IpAddr = "10.0.0.2".parse().unwrap();
        assert_eq!(
            auth.login(ip, "admin", "").await,
            Login::Fails,
            "no password set"
        );
        for forged in [
            r#"{"password":{"salt":"AAAA","hash":"AAAA","m_cost":4294967295,"t_cost":1}}"#,
            r#"{"password":{"salt":"AAAA","hash":"AAAA","m_cost":65536,"t_cost":1000}}"#,
            "not json",
        ] {
            std::fs::write(d.path().join(FILE), forged).unwrap();
            assert!(!has_password(d.path()), "{forged}");
        }
    }
}
