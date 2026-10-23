//! Remote-control secrets in their own owner-only file, never in settings or backups: the
//! password (Argon2id), the API key (SHA-256) and the dynamic DNS password. Also sessions and
//! bans after repeated failures.

use std::collections::{HashMap, VecDeque};
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
/// Each ban on the same client lasts twice as long as the last, up to this.
const MAX_BAN: Duration = Duration::from_secs(7 * 86_400);
/// A client's ban history is forgotten after this long without a failure.
const FORGET_AFTER: Duration = Duration::from_secs(7 * 86_400);
/// Failures from all clients together: past this many in the window, logins pause.
pub const GLOBAL_FAILURES: usize = 30;
const GLOBAL_WINDOW: Duration = Duration::from_secs(3600);

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
    /// Failures in a row before a ban; at least 1.
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
    /// Too many failures from all clients together; nobody can sign in with a password for now.
    Paused,
}

/// One ban covers a whole IPv6 /64, which a single home or server usually has.
fn client_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let mut seg = v6.segments();
                seg[4..].fill(0);
                IpAddr::V6(seg.into())
            }
        },
        v4 => v4,
    }
}

#[derive(Clone, Copy)]
struct Record {
    /// Failures since the last ban or success.
    fails: u32,
    /// Checks under way, counted as failures until they finish.
    pending: u32,
    /// Bans so far, for doubling the next one.
    strikes: u32,
    until: Option<Instant>,
    last: Instant,
}

impl Record {
    fn banned(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now < u)
    }
}

#[derive(Default)]
struct ThrottleState {
    clients: HashMap<IpAddr, Record>,
    recent: VecDeque<Instant>,
    pending: usize,
}

/// Failed sign-ins (password or API key), kept while Riptor runs, across server restarts.
#[derive(Default)]
pub struct Throttle(Mutex<ThrottleState>);

impl ThrottleState {
    fn prune(&mut self, now: Instant) {
        while self
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= GLOBAL_WINDOW)
        {
            self.recent.pop_front();
        }
    }

    /// Room for a new client: quiet, unbanned ones go first; bans are never dropped.
    fn make_room(&mut self, now: Instant) -> bool {
        if self.clients.len() < MAX_TRACKED_IPS {
            return true;
        }
        self.clients.retain(|_, r| {
            r.pending > 0 || r.banned(now) || now.duration_since(r.last) < FORGET_AFTER
        });
        if self.clients.len() < MAX_TRACKED_IPS {
            return true;
        }
        let victim = self
            .clients
            .iter()
            .filter(|(_, r)| r.pending == 0 && !r.banned(now) && r.strikes == 0)
            .min_by_key(|(_, r)| r.last)
            .map(|(k, _)| *k);
        victim.is_some_and(|k| self.clients.remove(&k).is_some())
    }
}

/// A sign-in attempt under way; dropped unfinished (the client hung up), it counts as a failure.
pub struct Attempt {
    throttle: Arc<Throttle>,
    ip: IpAddr,
    policy: Policy,
    done: bool,
}

impl Attempt {
    pub fn finish(mut self, ok: bool) {
        self.done = true;
        self.throttle.end(self.ip, ok, &self.policy);
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.done {
            self.throttle.end(self.ip, false, &self.policy);
        }
    }
}

impl Throttle {
    /// Starts a sign-in attempt, or says why it can't be tried now.
    pub fn begin(self: &Arc<Self>, ip: IpAddr, policy: &Policy) -> Result<Attempt, Login> {
        self.reserve(ip, policy)?;
        Ok(Attempt {
            throttle: self.clone(),
            ip,
            policy: *policy,
            done: false,
        })
    }

    fn reserve(&self, ip: IpAddr, policy: &Policy) -> Result<(), Login> {
        let key = client_key(ip);
        let now = Instant::now();
        let mut st = self.0.lock();
        st.prune(now);
        if st.clients.get(&key).is_some_and(|r| r.banned(now)) {
            return Err(Login::Banned);
        }
        if st.recent.len() + st.pending >= GLOBAL_FAILURES {
            return Err(Login::Paused);
        }
        if !st.clients.contains_key(&key) && !st.make_room(now) {
            // Every slot holds a ban or a live attempt.
            return Err(Login::Paused);
        }
        let r = st.clients.entry(key).or_insert(Record {
            fails: 0,
            pending: 0,
            strikes: 0,
            until: None,
            last: now,
        });
        // Attempts already in flight count, so parallel requests can't slip past the limit.
        if r.fails + r.pending >= policy.max_failures.max(1) {
            return Err(Login::Banned);
        }
        r.pending += 1;
        st.pending += 1;
        Ok(())
    }

    fn end(&self, ip: IpAddr, ok: bool, policy: &Policy) {
        let key = client_key(ip);
        let now = Instant::now();
        let mut st = self.0.lock();
        st.pending = st.pending.saturating_sub(1);
        if !ok {
            st.recent.push_back(now);
        }
        let Some(r) = st.clients.get_mut(&key) else {
            return;
        };
        r.pending = r.pending.saturating_sub(1);
        r.last = now;
        if ok {
            r.fails = 0;
            r.strikes = 0;
            return;
        }
        r.fails += 1;
        if r.fails >= policy.max_failures.max(1) {
            r.fails = 0;
            r.strikes = r.strikes.saturating_add(1);
            let doubling = 2u32.saturating_pow(r.strikes.saturating_sub(1).min(20));
            let ban = policy
                .ban
                .saturating_mul(doubling)
                .min(MAX_BAN)
                .max(policy.ban);
            r.until = Some(now + ban);
        }
    }

    pub fn banned(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        self.0
            .lock()
            .clients
            .get(&client_key(ip))
            .is_some_and(|r| r.banned(now))
    }
}

pub struct Auth {
    file: PathBuf,
    username: String,
    policy: Policy,
    api_key: Option<String>,
    sessions: Mutex<HashMap<String, Instant>>,
    throttle: Arc<Throttle>,
    // Argon2 is meant to be slow; two at a time keeps it from tying up the CPU.
    checks: Arc<Semaphore>,
}

impl Auth {
    pub fn new(data_dir: &Path, username: &str, policy: Policy) -> Self {
        Self::with_throttle(data_dir, username, policy, Arc::default())
    }

    pub fn with_throttle(
        data_dir: &Path,
        username: &str,
        policy: Policy,
        throttle: Arc<Throttle>,
    ) -> Self {
        Self {
            file: data_dir.join(FILE),
            username: username.to_string(),
            policy,
            api_key: read_secrets(&data_dir.join(FILE)).api_key,
            sessions: Mutex::new(HashMap::new()),
            throttle,
            checks: Arc::new(Semaphore::new(2)),
        }
    }

    pub fn banned(&self, ip: IpAddr) -> bool {
        self.throttle.banned(ip)
    }

    pub async fn login(&self, ip: IpAddr, username: &str, password: &str) -> Login {
        let attempt = match self.throttle.begin(ip, &self.policy) {
            Ok(a) => a,
            Err(why) => return why,
        };
        let ok = match (load(&self.file), password.len() <= MAX_PASSWORD * 4) {
            (Some(stored), true) => match self.checks.clone().acquire_owned().await {
                Ok(_slot) => {
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
                Err(_) => false,
            },
            _ => false,
        };
        attempt.finish(ok);
        if !ok {
            return Login::Fails;
        }
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

    /// An API key sent by `ip`; wrong keys count as failed sign-ins, like passwords.
    pub fn api_key_from(&self, ip: IpAddr, key: &str) -> bool {
        match self.throttle.begin(ip, &self.policy) {
            Ok(attempt) => {
                let ok = self.api_key_ok(key);
                attempt.finish(ok);
                ok
            }
            // A pause is for password guessing; a 192-bit key can't be guessed.
            Err(Login::Paused) => self.api_key_ok(key),
            Err(_) => false,
        }
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

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn policy(max_failures: u32, ban_ms: u64) -> Policy {
        Policy {
            max_failures,
            ban: Duration::from_millis(ban_ms),
            idle: Duration::from_secs(3600),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_guesses_cant_slip_past_the_limit() {
        let d = tempfile::tempdir().unwrap();
        set_password(d.path(), "correct horse").unwrap();
        let auth = Arc::new(Auth::new(d.path(), "admin", policy(3, 3_600_000)));
        let mut set = tokio::task::JoinSet::new();
        for i in 0..40 {
            let auth = auth.clone();
            set.spawn(async move {
                auth.login(ip("192.168.1.9"), "admin", &format!("guess{i}"))
                    .await
            });
        }
        let (mut checked, mut refused) = (0, 0);
        while let Some(r) = set.join_next().await {
            match r.unwrap() {
                Login::Fails => checked += 1,
                Login::Banned => refused += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!((checked, refused), (3, 37), "only 3 passwords may be tried");
        assert_eq!(
            auth.login(ip("192.168.1.9"), "admin", "correct horse")
                .await,
            Login::Banned
        );
    }

    #[tokio::test]
    async fn bans_double_each_time() {
        let d = tempfile::tempdir().unwrap();
        let auth = Auth::new(d.path(), "admin", policy(1, 200));
        let me = ip("10.0.0.7");
        assert_eq!(auth.login(me, "admin", "x").await, Login::Fails);
        assert_eq!(auth.login(me, "admin", "x").await, Login::Banned);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            auth.login(me, "admin", "x").await,
            Login::Fails,
            "first ban over"
        );
        // The second ban lasts 400 ms, not 200.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(auth.login(me, "admin", "x").await, Login::Banned);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(auth.login(me, "admin", "x").await, Login::Fails);
    }

    #[tokio::test]
    async fn one_ban_covers_an_ipv6_slash_64() {
        let d = tempfile::tempdir().unwrap();
        let auth = Auth::new(d.path(), "admin", policy(1, 3_600_000));
        assert_eq!(
            auth.login(ip("2001:db8::1"), "admin", "x").await,
            Login::Fails
        );
        assert_eq!(
            auth.login(ip("2001:db8::ffff:2"), "admin", "x").await,
            Login::Banned
        );
        assert_eq!(
            auth.login(ip("2001:db8:0:1::1"), "admin", "x").await,
            Login::Fails
        );
        // A v4-mapped address is the IPv4 client.
        assert_eq!(auth.login(ip("10.1.1.1"), "admin", "x").await, Login::Fails);
        assert_eq!(
            auth.login(ip("::ffff:10.1.1.1"), "admin", "x").await,
            Login::Banned
        );
    }

    #[tokio::test]
    async fn guesses_spread_over_many_addresses_pause_sign_ins() {
        let d = tempfile::tempdir().unwrap();
        set_password(d.path(), "correct horse").unwrap();
        let key = new_api_key(d.path()).unwrap();
        let auth = Auth::new(d.path(), "admin", policy(20, 3_600_000));
        for i in 0..GLOBAL_FAILURES {
            let who = ip(&format!("10.9.{}.{}", i / 200, i % 200 + 1));
            // A wrong user name skips nothing: the password is still checked.
            assert_eq!(auth.login(who, "nobody", "x").await, Login::Fails);
        }
        assert_eq!(
            auth.login(ip("10.99.0.1"), "admin", "guess").await,
            Login::Paused
        );
        assert_eq!(
            auth.login(ip("10.99.0.2"), "admin", "correct horse").await,
            Login::Paused,
            "even the right password waits"
        );
        assert!(!auth.api_key_from(ip("10.99.0.3"), "rpt_guess"));
        // The owner's API key still works.
        assert!(auth.api_key_from(ip("10.99.0.4"), &key));
    }

    #[tokio::test]
    async fn hanging_up_mid_check_still_counts() {
        let throttle = Arc::new(Throttle::default());
        let p = policy(2, 3_600_000);
        let me = ip("192.168.4.4");
        drop(throttle.begin(me, &p).unwrap());
        drop(throttle.begin(me, &p).unwrap());
        assert!(throttle.banned(me));
        assert!(matches!(throttle.begin(me, &p), Err(Login::Banned)));
        // Nothing is left counted as in flight.
        assert_eq!(throttle.0.lock().pending, 0);
    }

    #[test]
    fn wrong_api_keys_count_as_failures() {
        let d = tempfile::tempdir().unwrap();
        let key = new_api_key(d.path()).unwrap();
        let auth = Auth::new(d.path(), "admin", policy(3, 3_600_000));
        let me = ip("192.168.2.2");
        for _ in 0..3 {
            assert!(!auth.api_key_from(me, "rpt_wrong"));
        }
        assert!(auth.banned(me));
        assert!(
            !auth.api_key_from(me, &key),
            "a banned client can't use the key either"
        );
        assert!(auth.api_key_from(ip("192.168.2.3"), &key));
    }

    #[tokio::test]
    async fn bans_outlive_a_server_restart() {
        let d = tempfile::tempdir().unwrap();
        let throttle = Arc::new(Throttle::default());
        let me = ip("192.168.3.3");
        let first = Auth::with_throttle(d.path(), "admin", policy(1, 3_600_000), throttle.clone());
        assert_eq!(first.login(me, "admin", "x").await, Login::Fails);
        drop(first);
        let second = Auth::with_throttle(d.path(), "admin", policy(1, 3_600_000), throttle);
        assert_eq!(second.login(me, "admin", "x").await, Login::Banned);
    }

    #[test]
    fn a_full_table_never_drops_a_ban() {
        let now = Instant::now();
        let mut st = ThrottleState::default();
        let rec = |until| Record {
            fails: 0,
            pending: 0,
            strikes: 1,
            until,
            last: now,
        };
        let banned = ip("172.16.0.1");
        st.clients
            .insert(banned, rec(Some(now + Duration::from_secs(3600))));
        for i in 1..MAX_TRACKED_IPS as u32 {
            let a = std::net::Ipv4Addr::from(0x0b00_0000 + i);
            st.clients.insert(IpAddr::V4(a), rec(None));
        }
        // Every other entry has a ban history, so there's no room, and the ban stays.
        assert!(!st.make_room(now));
        assert!(st.clients.get(&banned).is_some_and(|r| r.banned(now)));
        // Without history the quietest one makes way.
        for r in st.clients.values_mut() {
            if r.until.is_none() {
                r.strikes = 0;
            }
        }
        assert!(st.make_room(now));
        assert!(st.clients.get(&banned).is_some_and(|r| r.banned(now)));
    }
}
