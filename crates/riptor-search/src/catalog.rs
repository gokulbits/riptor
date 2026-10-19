//! A list of nova3 search plugins the user can install, fetched from
//! GitHub when the Search engines dialog asks for it:
//! * official: `nova3/engines` in the upstream search-plugins repository;
//! * community: the "Unofficial search plugins" wiki page of that repository.
//!
//! No plugin code ships with Riptor. The catalog only offers download links;
//! installing still goes through [`crate::SearchManager::preview_plugin`],
//! where the user reviews the source and SHA-256 first. Everything fetched
//! here is untrusted: text is stripped and capped, and a download link must
//! be https and name a `.py` file.

use std::time::Duration;

use anyhow::ensure;
use futures::StreamExt;
use serde::Serialize;

use crate::nova;

pub const OFFICIAL_INDEX: &str =
    "https://raw.githubusercontent.com/qbittorrent/search-plugins/master/nova3/engines/versions.txt";
pub const OFFICIAL_BASE: &str =
    "https://raw.githubusercontent.com/qbittorrent/search-plugins/master/nova3/engines/";
pub const COMMUNITY_SOURCE: &str =
    "https://raw.githubusercontent.com/wiki/qbittorrent/search-plugins/Unofficial-search-plugins.mediawiki";
pub const COMMUNITY_PAGE: &str =
    "https://github.com/qbittorrent/search-plugins/wiki/Unofficial-search-plugins";
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 400;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// One official plugin is a Torznab bridge that needs a config file beside
/// it. Riptor speaks Torznab itself, so it is left out.
const SKIP_OFFICIAL: [&str; 1] = ["jackett"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Official,
    Community,
}

/// The community list's own status marks: ✔ works, ❗ has problems,
/// ✖ broken (and may slow other plugins down).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Working,
    Issues,
    Broken,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CatalogEntry {
    /// Plugin file name without `.py`.
    pub id: String,
    pub name: String,
    pub site: Option<String>,
    pub download_url: String,
    pub origin: Origin,
    /// Needs an account on the site (and usually editing the plugin).
    pub private: bool,
    pub version: Option<String>,
    pub author: Option<String>,
    pub health: Health,
    pub note: Option<String>,
    /// Why it can't be installed, if it can't.
    pub problem: Option<String>,
    pub installed: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Catalog {
    pub entries: Vec<CatalogEntry>,
    /// Lists that could not be loaded.
    pub errors: Vec<String>,
}

fn text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn web_url(s: &str) -> Option<String> {
    let u = url::Url::parse(s.trim()).ok()?;
    (matches!(u.scheme(), "http" | "https") && u.host_str().is_some() && s.len() <= 500)
        .then(|| u.to_string())
}

/// An https link to a `.py` file, and the plugin id it names.
fn plugin_link(s: &str) -> Option<(String, String)> {
    let u = url::Url::parse(s.trim()).ok()?;
    if u.scheme() != "https" || u.host_str().is_none() || s.len() > 500 {
        return None;
    }
    let file = u.path_segments()?.next_back()?.to_string();
    let id = file.strip_suffix(".py")?.to_string();
    Some((u.to_string(), id))
}

async fn get_capped(http: &reqwest::Client, url: &str, cap: usize) -> anyhow::Result<Vec<u8>> {
    let work = async {
        let resp = http.get(url).send().await?.error_for_status()?;
        ensure!(resp.url().scheme() == "https", "redirected off https");
        if let Some(len) = resp.content_length() {
            ensure!(len <= cap as u64, "response too large");
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            ensure!(body.len() + chunk.len() <= cap, "response too large");
            body.extend_from_slice(&chunk);
        }
        anyhow::Ok(body)
    };
    tokio::time::timeout(FETCH_TIMEOUT, work)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")))
}

/// `versions.txt`: one `id: version` per line.
pub fn parse_official_index(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|l| {
            let (id, ver) = l.split_once(':')?;
            let id = id.trim();
            (nova::validate_id(id).is_ok() && !SKIP_OFFICIAL.contains(&id))
                .then(|| (id.to_string(), text(ver, 20)))
        })
        .take(MAX_ENTRIES)
        .collect()
}

async fn official(http: &reqwest::Client) -> anyhow::Result<Vec<CatalogEntry>> {
    let index = get_capped(http, OFFICIAL_INDEX, 64 * 1024).await?;
    let ids = parse_official_index(&String::from_utf8_lossy(&index));
    ensure!(!ids.is_empty(), "the official list is empty");
    // Names and sites are read from the plugin files themselves (never run).
    let entries = futures::stream::iter(ids)
        .map(|(id, version)| async move {
            let download_url = format!("{OFFICIAL_BASE}{id}.py");
            let body = get_capped(http, &download_url, nova::MAX_PLUGIN_BYTES)
                .await
                .ok();
            let src = body.as_deref().map(String::from_utf8_lossy);
            let name = src
                .as_deref()
                .and_then(|s| nova::declared(s, "name"))
                .map(|n| text(&n, 80))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            let site = src
                .as_deref()
                .and_then(|s| nova::declared(s, "url"))
                .and_then(|u| web_url(&u));
            CatalogEntry {
                id,
                name,
                site,
                download_url,
                origin: Origin::Official,
                private: false,
                version: Some(version).filter(|v| !v.is_empty()),
                author: Some("Official plugin".into()),
                health: Health::Working,
                note: None,
                problem: None,
                installed: false,
            }
        })
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
    Ok(entries)
}

/// `[url label]` external links in wiki markup, ignoring `[[image]]` links.
fn ext_links(cell: &str) -> Vec<(String, String)> {
    // Drop [[...]] (images) first; what's left are external links.
    let mut plain = String::with_capacity(cell.len());
    let mut rest = cell;
    while let Some(i) = rest.find("[[") {
        plain.push_str(&rest[..i]);
        rest = match rest[i..].find("]]") {
            Some(j) => &rest[i + j + 2..],
            None => "",
        };
    }
    plain.push_str(rest);
    let mut out = Vec::new();
    let mut rest = plain.as_str();
    while let Some(i) = rest.find('[') {
        let after = &rest[i + 1..];
        let Some(j) = after.find(']') else { break };
        let inner = after[..j].trim();
        let (url, label) = inner.split_once(' ').unwrap_or((inner, ""));
        if url.starts_with("http://") || url.starts_with("https://") {
            out.push((url.to_string(), strip_markup(label)));
        }
        rest = &after[j + 1..];
    }
    out
}

fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    let out = out.replace("'''", "").replace("''", "").replace("**", "");
    text(&out.split_whitespace().collect::<Vec<_>>().join(" "), 300)
}

fn health(comment: &str) -> Health {
    if comment.contains('✖') {
        Health::Broken
    } else if comment.contains('❗') {
        Health::Issues
    } else if comment.contains('✔') {
        Health::Working
    } else {
        Health::Unknown
    }
}

fn community_entry(cells: &[String], private: bool) -> Option<CatalogEntry> {
    if cells.len() < 5 {
        return None;
    }
    let engine = ext_links(&cells[0]);
    let (download_url, id) = ext_links(&cells[4])
        .into_iter()
        .find_map(|(u, _)| plugin_link(&u))?;
    let (site, name) = match engine.first() {
        Some((u, label)) => (web_url(u), label.clone()),
        None => (None, strip_markup(&cells[0])),
    };
    let name = text(&name, 80);
    let author = ext_links(&cells[1])
        .first()
        .map(|(_, l)| l.clone())
        .unwrap_or_else(|| strip_markup(&cells[1]));
    let comment = cells.get(5).map(String::as_str).unwrap_or_default();
    let note = strip_markup(comment.trim_start_matches(['✔', '❗', '✖', ' ']));
    let problem = nova::validate_id(&id).err().map(|e| e.to_string());
    Some(CatalogEntry {
        name: if name.is_empty() { id.clone() } else { name },
        id,
        site,
        download_url,
        origin: Origin::Community,
        private,
        version: Some(text(&strip_markup(&cells[2]), 20)).filter(|v| !v.is_empty()),
        author: Some(text(&author, 80)).filter(|a| !a.is_empty()),
        health: health(comment),
        note: Some(note).filter(|n| !n.is_empty()),
        problem,
        installed: false,
    })
}

/// The wiki page's tables: one row per plugin, one `| cell` per line.
pub fn parse_community(page: &str) -> Vec<CatalogEntry> {
    let mut out = Vec::new();
    let (mut private, mut in_table) = (false, false);
    let mut row: Vec<String> = Vec::new();
    let flush = |row: &mut Vec<String>, private: bool, out: &mut Vec<CatalogEntry>| {
        if out.len() < MAX_ENTRIES {
            if let Some(e) = community_entry(row, private) {
                out.push(e);
            }
        }
        row.clear();
    };
    for line in page.lines().take(20_000) {
        let l = line.trim();
        if l.starts_with("==") {
            private = l.to_ascii_lowercase().contains("private");
        } else if l.starts_with("{|") {
            in_table = true;
            row.clear();
        } else if !in_table {
        } else if l.starts_with("|}") {
            flush(&mut row, private, &mut out);
            in_table = false;
        } else if l.starts_with("|-") {
            flush(&mut row, private, &mut out);
        } else if l.starts_with('!') {
        } else if let Some(cell) = l.strip_prefix('|') {
            if row.len() < 12 {
                row.push(cell.trim().chars().take(2000).collect());
            }
        } else if let Some(last) = row.last_mut() {
            if last.len() < 4000 {
                last.push(' ');
                last.push_str(l);
            }
        }
    }
    out
}

async fn community(http: &reqwest::Client) -> anyhow::Result<Vec<CatalogEntry>> {
    let page = get_capped(http, COMMUNITY_SOURCE, MAX_PAGE_BYTES).await?;
    let entries = parse_community(&String::from_utf8_lossy(&page));
    ensure!(!entries.is_empty(), "the community list could not be read");
    Ok(entries)
}

/// Fetch both lists; one failing still returns the other.
pub async fn fetch(http: &reqwest::Client) -> Catalog {
    let (off, com) = futures::join!(official(http), community(http));
    let mut c = Catalog::default();
    match off {
        Ok(v) => c.entries.extend(v),
        Err(e) => c
            .errors
            .push(format!("Couldn't load the official plugin list: {e:#}")),
    }
    match com {
        Ok(v) => c.entries.extend(v),
        Err(e) => c
            .errors
            .push(format!("Couldn't load the community plugin list: {e:#}")),
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"Intro text [https://example.org not a table]
== Plugins for Public Sites ==
{|class="sortable"
|-
!  Search Engine
!  Author (Repository)
!  Version
!  Last update
!  Download link
!  Comments
|-
| [[https://www.google.com/s2/favicons?domain=a.example#.png]] [https://a.example/ Academic <b>Torrents</b>]
| [https://github.com/someone Some One]
| 1.2
| 19/Mar<br>2023
| [https://raw.githubusercontent.com/someone/p/master/academic.py [[https://raw.githubusercontent.com/x/Download.gif]]]
| ✔ qbt 4.3.x / Python 3.8.5 <br>**READ REPOSITORY NOTES**
|-
| [[https://x/icon.png]][https://b.example/ Bad Plugin]
| Plain Author
| 0.1
| 01/Jan<br>2020
| [http://insecure.example/bad.py [[https://x/Download.gif]]]
| ✖ broken
|-
| [https://c.example/ Caps]
| [https://github.com/c c]
| 2.0
| 01/Jan<br>2020
| [https://gist.githubusercontent.com/c/abc/raw/123/Caps-Engine.py [[https://x/Download.gif]]]<br>[https://github.com/c#readme [[https://x/Help.gif]]]
| ❗ qbt 4.3.0.1 / Python 3.9
continued comment line
|-
| [https://d.example/ javascript]
| x
| 1
| 1
| [https://raw.githubusercontent.com/d/d/main/dee.py?x=1 [[https://x/Download.gif]]]
| ✔
|}

== Plugins for Private Sites ==
{|
|-
| [https://priv.example PrivSite]
| [https://github.com/p p]
| 1.00
| 1
| [https://raw.githubusercontent.com/p/p/main/privsite.py [[https://x/Download.gif]]]
| ✔ requires login
|}
"#;

    #[test]
    fn parses_the_community_wiki_table() {
        let e = parse_community(PAGE);
        let ids: Vec<&str> = e.iter().map(|x| x.id.as_str()).collect();
        // The plain-http download is dropped.
        assert_eq!(ids, ["academic", "Caps-Engine", "dee", "privsite"]);
        let a = &e[0];
        assert_eq!(a.name, "Academic Torrents");
        assert_eq!(a.site.as_deref(), Some("https://a.example/"));
        assert_eq!(a.author.as_deref(), Some("Some One"));
        assert_eq!(a.version.as_deref(), Some("1.2"));
        assert_eq!(a.health, Health::Working);
        assert_eq!(
            a.note.as_deref(),
            Some("qbt 4.3.x / Python 3.8.5 READ REPOSITORY NOTES")
        );
        assert!(!a.private && a.problem.is_none());
        // Uppercase file names can't be loaded by the shim; listed, not installable.
        assert_eq!(e[1].health, Health::Issues);
        assert!(e[1].problem.is_some());
        assert!(e[1]
            .note
            .as_deref()
            .unwrap()
            .ends_with("continued comment line"));
        assert!(e[3].private);
    }

    #[test]
    fn official_index() {
        let v = parse_official_index(
            "eztv: 1.24\njackett: 4.13\n../evil: 1\nnova2: 1\n\npiratebay: 3.10\n",
        );
        assert_eq!(
            v,
            [
                ("eztv".into(), "1.24".into()),
                ("piratebay".into(), "3.10".into())
            ]
        );
    }

    #[test]
    fn hostile_markup_is_contained() {
        assert!(parse_community(
            "{|\n|-\n| [javascript:alert(1) x]\n|a\n|b\n|c\n| [file:///etc/passwd.py x]\n|}"
        )
        .is_empty());
        let huge = format!(
            "{{|\n|-\n|{}\n|a\n|b\n|c\n| [https://e.example/e.py]\n|}}",
            "[".repeat(100_000)
        );
        let e = parse_community(&huge);
        assert_eq!(e.len(), 1);
        assert!(e[0].name.len() <= 80 * 4);
        // Control characters never reach the UI.
        let e = parse_community("{|\n|-\n| [https://s.example Na\u{0007}me]\n|a\n|b\n|c\n| [https://e.example/e.py]\n|}");
        assert_eq!(e[0].name, "Name");
    }
}
