//! Riptor's search engine: several providers queried in parallel, results
//! streamed back as they arrive.
//!
//! * [`archive`]: Internet Archive (built in, needs no setup).
//! * [`torznab`]: Torznab indexers.
//! * [`nova`]: nova3-compatible Python plugins, offered from a
//!   [`catalog`] fetched from GitHub, run by a system Python or one
//!   [`pyembed`] sets up.
//!
//! Results without seed/peer counts get them from tracker [`scrape`]s.
//!
//! Every result is untrusted input from the network; [`clean`] normalises it
//! before it reaches the UI.
#![forbid(unsafe_code)]

pub mod archive;
pub mod catalog;
pub mod manager;
pub mod nova;
pub mod pyembed;
pub mod scrape;
pub mod secrets;
pub mod time;
pub mod torznab;

use serde::{Deserialize, Serialize};

pub use manager::{SearchConfig, SearchManager, TorznabEndpoint};

pub const CATEGORIES: [&str; 9] = [
    "all", "anime", "books", "games", "movies", "music", "pictures", "software", "tv",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    /// `magnet:` link or http(s) URL of a `.torrent` file.
    pub link: String,
    pub size: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    /// Web page for the result, opened in the browser.
    pub desc_link: Option<String>,
    pub pub_date: Option<u64>,
    /// Provider id that returned it.
    pub engine: String,
    /// For plugin results whose link must be fetched through the plugin.
    #[serde(default)]
    pub via_plugin: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Builtin,
    Torznab,
    Plugin,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub url: String,
    pub enabled: bool,
    pub categories: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchEvent {
    Results {
        search_id: u64,
        provider: String,
        results: Vec<SearchResult>,
    },
    ProviderDone {
        search_id: u64,
        provider: String,
        count: usize,
        error: Option<String>,
    },
    /// Every provider has answered (tracker lookups may still be running).
    Finished { search_id: u64 },
    /// Seed/peer counts from tracker scrapes, for results that had none.
    Counts {
        search_id: u64,
        counts: Vec<scrape::ResultCounts>,
    },
    /// Tracker lookups for this search are over.
    LookupsDone { search_id: u64 },
}

pub fn validate_query(q: &str) -> anyhow::Result<String> {
    let q = q.trim();
    anyhow::ensure!(!q.is_empty(), "type something to search for");
    anyhow::ensure!(q.chars().count() <= 200, "search text is too long");
    anyhow::ensure!(
        !q.chars().any(|c| c.is_control()),
        "search text contains invalid characters"
    );
    Ok(q.to_string())
}

pub fn validate_category(c: &str) -> anyhow::Result<()> {
    anyhow::ensure!(CATEGORIES.contains(&c), "unknown category");
    Ok(())
}

fn http_url(s: &str) -> Option<String> {
    let u = url::Url::parse(s.trim()).ok()?;
    (matches!(u.scheme(), "http" | "https") && u.host_str().is_some()).then(|| u.to_string())
}

/// Normalise one untrusted result; `None` drops it.
pub fn clean(mut r: SearchResult) -> Option<SearchResult> {
    r.name = r
        .name
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect::<String>()
        .trim()
        .to_string();
    if r.name.is_empty() {
        return None;
    }
    let link = r.link.trim();
    r.link = if link.starts_with("magnet:?") && link.len() <= 64 * 1024 {
        link.to_string()
    } else {
        http_url(link)?
    };
    r.desc_link = r.desc_link.as_deref().and_then(http_url);
    // A year 2100+ "publish date" is junk.
    r.pub_date = r.pub_date.filter(|d| *d < 4_102_444_800);
    r.seeders = r.seeders.filter(|s| *s < 10_000_000);
    r.leechers = r.leechers.filter(|s| *s < 10_000_000);
    r.size = r.size.filter(|s| *s < 1 << 50);
    Some(r)
}

/// Dedupe key: the BitTorrent info hash when visible, else the link.
pub fn dedupe_key(r: &SearchResult) -> String {
    if let Some(i) = r.link.to_ascii_lowercase().find("xt=urn:btih:") {
        let h: String = r.link[i + 12..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        return h.to_ascii_lowercase();
    }
    r.link.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(link: &str) -> SearchResult {
        SearchResult {
            name: "  Ubuntu\u{0007} 24.04 ".into(),
            link: link.into(),
            size: Some(5),
            seeders: Some(3),
            leechers: Some(99_999_999),
            desc_link: Some("javascript:alert(1)".into()),
            pub_date: Some(9_999_999_999),
            engine: "x".into(),
            via_plugin: false,
        }
    }

    #[test]
    fn cleaning() {
        let c = clean(r(
            "magnet:?xt=urn:btih:ABCDEF0123456789ABCDEF0123456789ABCDEF01&dn=x",
        ))
        .unwrap();
        assert_eq!(c.name, "Ubuntu 24.04");
        assert_eq!(c.desc_link, None);
        assert_eq!(c.pub_date, None);
        assert_eq!(c.leechers, None);
        assert_eq!(dedupe_key(&c), "abcdef0123456789abcdef0123456789abcdef01");
        assert!(clean(r("file:///etc/passwd")).is_none());
        assert!(clean(r("javascript:alert(1)")).is_none());
        assert!(clean(r("https://example.org/a.torrent")).is_some());
        let mut empty = r("https://example.org/a.torrent");
        empty.name = "\u{0001}".into();
        assert!(clean(empty).is_none());
    }

    #[test]
    fn queries() {
        assert_eq!(validate_query("  debian ").unwrap(), "debian");
        assert!(validate_query("").is_err());
        assert!(validate_query("a\nb").is_err());
        assert!(validate_query(&"x".repeat(201)).is_err());
        assert!(validate_category("tv").is_ok());
        assert!(validate_category("../../x").is_err());
    }
}
