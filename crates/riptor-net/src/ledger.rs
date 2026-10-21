//! Every host Riptor contacted this session and how. Memory only, never saved.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 2000;
const MAX_HOST: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Purpose {
    Tracker,
    WebSeed,
    TorrentLink,
    Feed,
    Search,
    Scrape,
    Plugin,
    Catalog,
    IpCheck,
    Dns,
    Blocklist,
    Update,
    Python,
    VpnServer,
}

impl Purpose {
    pub fn label(self) -> &'static str {
        match self {
            Purpose::Tracker => "tracker",
            Purpose::WebSeed => "web_seed",
            Purpose::TorrentLink => "torrent_link",
            Purpose::Feed => "feed",
            Purpose::Search => "search",
            Purpose::Scrape => "scrape",
            Purpose::Plugin => "plugin",
            Purpose::Catalog => "catalog",
            Purpose::IpCheck => "ip_check",
            Purpose::Dns => "dns",
            Purpose::Blocklist => "blocklist",
            Purpose::Update => "update",
            Purpose::Python => "python",
            Purpose::VpnServer => "vpn_server",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// The normal connection.
    Direct,
    /// Bound to the VPN adapter.
    Tunnel,
    /// Loopback or the LAN.
    Local,
}

impl Route {
    pub fn label(self) -> &'static str {
        match self {
            Route::Direct => "direct",
            Route::Tunnel => "tunnel",
            Route::Local => "local",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub purpose: Purpose,
    pub host: String,
    pub route: Route,
    pub count: u64,
    pub failures: u64,
    /// Unix seconds.
    pub first: u64,
    pub last: u64,
}

type Key = (Purpose, String, Route);

static ENTRIES: Mutex<Option<HashMap<Key, Entry>>> = Mutex::new(None);
static DEFAULT_ROUTE: AtomicU8 = AtomicU8::new(0);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How traffic leaves right now: the engine updates it when the VPN goes up or down.
pub fn set_default_route(r: Route) {
    DEFAULT_ROUTE.store(r as u8, Ordering::Relaxed);
}

pub fn default_route() -> Route {
    match DEFAULT_ROUTE.load(Ordering::Relaxed) {
        1 => Route::Tunnel,
        2 => Route::Local,
        _ => Route::Direct,
    }
}

/// `host` may be a bare host or a URL; only the host is kept, never paths or keys.
pub fn record(purpose: Purpose, host: &str, ok: bool) {
    let route = host_of(host)
        .and_then(|h| url::Url::parse(&format!("http://{h}/")).ok())
        .filter(crate::stays_local)
        .map_or_else(default_route, |_| Route::Local);
    record_via(purpose, host, route, ok);
}

pub fn record_via(purpose: Purpose, host: &str, route: Route, ok: bool) {
    let Some(host) = host_of(host) else {
        return;
    };
    let mut guard = ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    add(
        guard.get_or_insert_with(HashMap::new),
        purpose,
        host,
        route,
        ok,
        now(),
    );
}

fn add(
    entries: &mut HashMap<Key, Entry>,
    purpose: Purpose,
    host: String,
    route: Route,
    ok: bool,
    t: u64,
) {
    let key = (purpose, host.clone(), route);
    if !entries.contains_key(&key) && entries.len() >= MAX_ENTRIES {
        if let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, e)| e.last)
            .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
    }
    let e = entries.entry(key).or_insert_with(|| Entry {
        purpose,
        host,
        route,
        count: 0,
        failures: 0,
        first: t,
        last: t,
    });
    e.count += 1;
    if !ok {
        e.failures += 1;
    }
    e.last = t;
}

/// Most recent first.
pub fn snapshot() -> Vec<Entry> {
    let guard = ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<Entry> = guard.iter().flat_map(|m| m.values().cloned()).collect();
    v.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.host.cmp(&b.host)));
    v
}

pub fn clear() {
    *ENTRIES.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The host part of a URL or `host[:port]`, lowercased and without credentials.
fn host_of(s: &str) -> Option<String> {
    let s = s.trim();
    let host = if s.contains("://") {
        url::Url::parse(s).ok()?.host_str()?.to_string()
    } else {
        let s = s.rsplit('@').next()?;
        match s.strip_prefix('[') {
            Some(rest) => format!("[{}]", rest.split(']').next()?),
            None if s.matches(':').count() == 1 => s.split(':').next()?.to_string(),
            None => s.to_string(),
        }
    };
    let host = host.to_ascii_lowercase();
    let ok = !host.is_empty()
        && host.len() <= MAX_HOST
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'));
    ok.then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests share the one ledger, so each uses its own hosts.
    fn find(host: &str) -> Vec<Entry> {
        snapshot().into_iter().filter(|e| e.host == host).collect()
    }

    #[test]
    fn hosts_are_counted_and_secrets_dropped() {
        record(
            Purpose::Tracker,
            "https://T1.example:443/announce?passkey=abc",
            true,
        );
        record(Purpose::Tracker, "udp://t1.example:6969", false);
        let e = find("t1.example");
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].count, e[0].failures), (2, 1));
        assert!(snapshot().iter().all(|e| !e.host.contains("passkey")));

        record(Purpose::Search, "user:pw@s1.example:8080", true);
        assert_eq!(find("s1.example").len(), 1);
        record(Purpose::Dns, "[2606:4700::1111]:53", true);
        assert_eq!(find("[2606:4700::1111]").len(), 1);
    }

    #[test]
    fn local_hosts_and_routes_are_told_apart() {
        record(Purpose::Search, "http://127.0.0.1:9117/api", true);
        assert_eq!(find("127.0.0.1")[0].route, Route::Local);
        record_via(Purpose::IpCheck, "r1.example", Route::Tunnel, true);
        record_via(Purpose::IpCheck, "r1.example", Route::Direct, true);
        let mut routes: Vec<Route> = find("r1.example").iter().map(|e| e.route).collect();
        routes.sort_by_key(|r| *r as u8);
        assert_eq!(routes, [Route::Direct, Route::Tunnel]);
    }

    #[test]
    fn junk_is_ignored_and_the_ledger_stays_bounded() {
        for bad in [
            "",
            "a b",
            "<script>",
            "javascript:alert(1)",
            &"x".repeat(300),
        ] {
            record(Purpose::Feed, bad, true);
        }
        assert!(snapshot()
            .iter()
            .all(|e| e.host.len() <= MAX_HOST && !e.host.contains(' ')));
        // On a private map, so other tests' entries aren't pushed out.
        let mut m = HashMap::new();
        for i in 0..(MAX_ENTRIES + 50) {
            add(
                &mut m,
                Purpose::Scrape,
                format!("bulk{i}.example"),
                Route::Direct,
                true,
                i as u64,
            );
        }
        assert_eq!(m.len(), MAX_ENTRIES);
        assert!(!m.values().any(|e| e.host == "bulk0.example"));
    }
}
