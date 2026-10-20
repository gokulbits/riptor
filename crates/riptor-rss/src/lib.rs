//! RSS and Atom feeds, with rules that add matching torrents automatically.
#![forbid(unsafe_code)]

pub mod feed;
pub mod rules;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub use feed::Article;
pub use rules::Rule;

const MAX_FEEDS: usize = 200;
const MAX_RULES: usize = 500;
const KEEP_ARTICLES: usize = 200;
const KEEP_SEEN: usize = 2000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Rules add torrents; off, feeds are only read.
    pub auto_download: bool,
    pub refresh_minutes: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auto_download: true,
            refresh_minutes: 30,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Feed {
    id: u32,
    url: String,
    name: String,
    #[serde(default)]
    last_refresh: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    /// Newest first.
    #[serde(default)]
    articles: Vec<Article>,
    /// Ids already handled, so rules fire once per article.
    #[serde(default)]
    seen: VecDeque<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    options: Options,
    feeds: Vec<Feed>,
    rules: Vec<Rule>,
    next_id: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct FeedInfo {
    pub id: u32,
    pub url: String,
    pub name: String,
    pub last_refresh: Option<u64>,
    pub error: Option<String>,
    pub unread: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub options: Options,
    pub feeds: Vec<FeedInfo>,
    pub rules: Vec<Rule>,
}

/// An article a rule picked, to be added.
#[derive(Clone, Debug)]
pub struct Download {
    pub feed: u32,
    pub title: String,
    pub torrent: String,
    pub rule: Rule,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleMatch {
    pub feed: String,
    pub title: String,
}

pub struct Rss {
    path: PathBuf,
    state: Mutex<State>,
}

pub fn validate_feed_url(raw: &str) -> anyhow::Result<String> {
    let raw = raw.trim();
    ensure!(raw.len() <= 2048, "feed URL is too long");
    let u = url::Url::parse(raw).context("not a valid URL")?;
    ensure!(
        matches!(u.scheme(), "http" | "https"),
        "only http(s) feeds are supported"
    );
    ensure!(u.host_str().is_some(), "the URL has no host");
    ensure!(
        u.username().is_empty() && u.password().is_none(),
        "user names and passwords in feed URLs aren't supported"
    );
    Ok(u.to_string())
}

fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect::<String>()
        .trim()
        .to_string()
}

impl Rss {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("rss.json");
        let state = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                tracing::warn!("rss.json unreadable ({e}); starting fresh");
                State::default()
            }),
            Err(_) => State::default(),
        };
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn save(&self, s: &State) -> anyhow::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(s)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn edit<T>(&self, f: impl FnOnce(&mut State) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut s = self.state.lock();
        let out = f(&mut s)?;
        self.save(&s)?;
        Ok(out)
    }

    pub fn view(&self) -> View {
        let s = self.state.lock();
        View {
            options: s.options.clone(),
            feeds: s
                .feeds
                .iter()
                .map(|f| FeedInfo {
                    id: f.id,
                    url: f.url.clone(),
                    name: f.name.clone(),
                    last_refresh: f.last_refresh,
                    error: f.error.clone(),
                    unread: f.articles.iter().filter(|a| !a.read).count(),
                    total: f.articles.len(),
                })
                .collect(),
            rules: s.rules.clone(),
        }
    }

    pub fn articles(&self, feed: u32) -> anyhow::Result<Vec<Article>> {
        let s = self.state.lock();
        let f = s
            .feeds
            .iter()
            .find(|f| f.id == feed)
            .context("unknown feed")?;
        Ok(f.articles.clone())
    }

    pub fn add_feed(&self, url: &str, name: Option<&str>) -> anyhow::Result<u32> {
        let url = validate_feed_url(url)?;
        self.edit(|s| {
            ensure!(s.feeds.len() < MAX_FEEDS, "too many feeds");
            ensure!(
                !s.feeds.iter().any(|f| f.url == url),
                "that feed is already added"
            );
            s.next_id += 1;
            let name = name.map(clean_name).filter(|n| !n.is_empty());
            let host = url::Url::parse(&url)
                .ok()
                .and_then(|u| u.host_str().map(String::from));
            s.feeds.push(Feed {
                id: s.next_id,
                name: name.or(host).unwrap_or_else(|| url.clone()),
                url,
                ..Default::default()
            });
            Ok(s.next_id)
        })
    }

    pub fn remove_feed(&self, id: u32) -> anyhow::Result<()> {
        self.edit(|s| {
            s.feeds.retain(|f| f.id != id);
            for r in &mut s.rules {
                r.feeds.retain(|f| *f != id);
            }
            Ok(())
        })
    }

    pub fn rename_feed(&self, id: u32, name: &str) -> anyhow::Result<()> {
        let name = clean_name(name);
        ensure!(!name.is_empty(), "give the feed a name");
        self.edit(|s| {
            s.feeds
                .iter_mut()
                .find(|f| f.id == id)
                .context("unknown feed")?
                .name = name;
            Ok(())
        })
    }

    /// `article: None` marks the whole feed.
    pub fn mark_read(&self, feed: u32, article: Option<&str>) -> anyhow::Result<()> {
        self.edit(|s| {
            let f = s
                .feeds
                .iter_mut()
                .find(|f| f.id == feed)
                .context("unknown feed")?;
            for a in f
                .articles
                .iter_mut()
                .filter(|a| article.is_none_or(|id| a.id == id))
            {
                a.read = true;
            }
            Ok(())
        })
    }

    pub fn set_options(&self, o: Options) -> anyhow::Result<()> {
        ensure!(
            (5..=24 * 60).contains(&o.refresh_minutes),
            "refresh every 5 minutes to 24 hours"
        );
        self.edit(|s| {
            s.options = o;
            Ok(())
        })
    }

    pub fn save_rule(&self, mut rule: Rule) -> anyhow::Result<u32> {
        rule.validate()?;
        rule.name = rule.name.trim().to_string();
        self.edit(|s| {
            rule.feeds.retain(|id| s.feeds.iter().any(|f| f.id == *id));
            if rule.id == 0 {
                ensure!(s.rules.len() < MAX_RULES, "too many rules");
                s.next_id += 1;
                rule.id = s.next_id;
                s.rules.push(rule.clone());
            } else {
                let r = s
                    .rules
                    .iter_mut()
                    .find(|r| r.id == rule.id)
                    .context("unknown rule")?;
                rule.last_match = r.last_match;
                *r = rule.clone();
            }
            Ok(rule.id)
        })
    }

    pub fn delete_rule(&self, id: u32) -> anyhow::Result<()> {
        self.edit(|s| {
            s.rules.retain(|r| r.id != id);
            Ok(())
        })
    }

    /// Articles already in the feeds that the rule would match.
    pub fn preview_rule(&self, rule: &Rule) -> anyhow::Result<Vec<RuleMatch>> {
        let m = rule.matcher()?;
        let s = self.state.lock();
        Ok(s.feeds
            .iter()
            .filter(|f| rule.applies_to(f.id))
            .flat_map(|f| {
                f.articles
                    .iter()
                    .filter(|a| m.matches(&a.title))
                    .map(|a| RuleMatch {
                        feed: f.name.clone(),
                        title: a.title.clone(),
                    })
            })
            .take(200)
            .collect())
    }

    /// Feeds due for a refresh; `force` takes every feed (or the ones in `only`).
    pub fn due(&self, now: u64, force: bool, only: Option<&[u32]>) -> Vec<(u32, String)> {
        let s = self.state.lock();
        let every = u64::from(s.options.refresh_minutes) * 60;
        s.feeds
            .iter()
            .filter(|f| only.is_none_or(|ids| ids.contains(&f.id)))
            .filter(|f| {
                force
                    || f.last_refresh
                        .is_none_or(|t| now.saturating_sub(t) >= every)
            })
            .map(|f| (f.id, f.url.clone()))
            .collect()
    }

    /// Stores a fetch result and returns what the rules picked.
    pub fn apply(
        &self,
        feed: u32,
        now: u64,
        fetched: anyhow::Result<Vec<u8>>,
    ) -> anyhow::Result<Vec<Download>> {
        let parsed = fetched.and_then(|b| feed::parse(&b));
        self.edit(|s| {
            let auto = s.options.auto_download;
            let matchers: Vec<(usize, rules::Matcher)> = s
                .rules
                .iter()
                .enumerate()
                .filter(|(_, r)| r.enabled && r.applies_to(feed))
                .filter_map(|(i, r)| r.matcher().ok().map(|m| (i, m)))
                .collect();
            let f = s
                .feeds
                .iter_mut()
                .find(|f| f.id == feed)
                .context("unknown feed")?;
            f.last_refresh = Some(now);
            let parsed = match parsed {
                Ok(p) => p,
                Err(e) => {
                    f.error = Some(format!("{e:#}").chars().take(300).collect());
                    return Ok(Vec::new());
                }
            };
            f.error = None;
            if f.name
                == url::Url::parse(&f.url)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_default()
            {
                if let Some(t) = parsed
                    .title
                    .as_deref()
                    .map(clean_name)
                    .filter(|t| !t.is_empty())
                {
                    f.name = t;
                }
            }
            let seen: HashSet<String> = f.seen.iter().cloned().collect();
            let mut picked = Vec::new();
            let mut fresh = Vec::new();
            for mut a in parsed.articles {
                if seen.contains(&a.id) || fresh.iter().any(|x: &Article| x.id == a.id) {
                    continue;
                }
                if auto {
                    if let Some((i, _)) = matchers.iter().find(|(_, m)| m.matches(&a.title)) {
                        a.downloaded = true;
                        picked.push((*i, a.title.clone(), a.torrent.clone()));
                    }
                }
                fresh.push(a);
            }
            for a in &fresh {
                f.seen.push_back(a.id.clone());
            }
            while f.seen.len() > KEEP_SEEN {
                f.seen.pop_front();
            }
            let old = std::mem::take(&mut f.articles);
            f.articles = fresh.into_iter().chain(old).take(KEEP_ARTICLES).collect();
            let downloads = picked
                .into_iter()
                .map(|(i, title, torrent)| {
                    s.rules[i].last_match = Some(now);
                    Download {
                        feed,
                        title,
                        torrent,
                        rule: s.rules[i].clone(),
                    }
                })
                .collect();
            Ok(downloads)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &[u8] = br#"<rss><channel><title>Shows</title>
<item><title>Show S01E01 1080p</title><guid>1</guid><link>https://e.org/1.torrent</link></item>
<item><title>Show S01E01 720p</title><guid>2</guid><link>https://e.org/2.torrent</link></item>
<item><title>Other 1080p</title><guid>3</guid><link>magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567</link></item>
</channel></rss>"#;

    #[test]
    fn rules_fire_once_per_article() {
        let d = tempfile::tempdir().unwrap();
        let rss = Rss::open(d.path());
        let feed = rss.add_feed("https://e.org/rss", None).unwrap();
        rss.save_rule(Rule {
            name: "show".into(),
            must_contain: "show 1080p".into(),
            ..Default::default()
        })
        .unwrap();
        let got = rss.apply(feed, 100, Ok(FEED.to_vec())).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].torrent, "https://e.org/1.torrent");
        let v = rss.view();
        assert_eq!(v.feeds[0].name, "Shows");
        assert_eq!(v.feeds[0].unread, 3);
        assert_eq!(v.rules[0].last_match, Some(100));
        // The same items again add nothing.
        assert!(rss.apply(feed, 200, Ok(FEED.to_vec())).unwrap().is_empty());
        assert_eq!(rss.articles(feed).unwrap().len(), 3);

        // Survives a restart.
        let again = Rss::open(d.path());
        assert!(again
            .apply(feed, 300, Ok(FEED.to_vec()))
            .unwrap()
            .is_empty());
        assert_eq!(again.preview_rule(&again.view().rules[0]).unwrap().len(), 1);
    }

    #[test]
    fn auto_download_off_and_fetch_errors() {
        let d = tempfile::tempdir().unwrap();
        let rss = Rss::open(d.path());
        let feed = rss.add_feed("https://e.org/rss", Some("Mine")).unwrap();
        rss.save_rule(Rule {
            name: "all".into(),
            must_contain: "1080p".into(),
            ..Default::default()
        })
        .unwrap();
        rss.set_options(Options {
            auto_download: false,
            refresh_minutes: 30,
        })
        .unwrap();
        assert!(rss.apply(feed, 1, Ok(FEED.to_vec())).unwrap().is_empty());
        assert_eq!(rss.view().feeds[0].name, "Mine");
        rss.apply(feed, 2, Err(anyhow::anyhow!("boom"))).unwrap();
        assert_eq!(rss.view().feeds[0].error.as_deref(), Some("boom"));
        assert_eq!(rss.due(2 + 29 * 60, false, None).len(), 0);
        assert_eq!(rss.due(2 + 30 * 60, false, None).len(), 1);
    }

    #[test]
    fn feed_urls_are_checked() {
        for bad in [
            "file:///etc/passwd",
            "javascript:x",
            "ftp://e.org/rss",
            "https://u:p@e.org/rss",
            "nope",
        ] {
            assert!(validate_feed_url(bad).is_err(), "{bad}");
        }
        assert!(validate_feed_url("https://e.org/rss?passkey=abc").is_ok());
        let d = tempfile::tempdir().unwrap();
        let rss = Rss::open(d.path());
        rss.add_feed("https://e.org/rss", None).unwrap();
        assert!(rss.add_feed("https://e.org/rss", None).is_err());
        assert!(rss
            .set_options(Options {
                auto_download: true,
                refresh_minutes: 1
            })
            .is_err());
    }
}
