//! RSS 2.0 and Atom. quick-xml never resolves external entities.

use anyhow::ensure;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};

use riptor_search::time::{parse_iso8601, parse_rfc2822};

pub const MAX_FEED_BYTES: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 1000;
const MAX_TITLE: usize = 500;
const MAX_LINK: usize = 8 * 1024;
const MAX_TEXT: usize = 64 * 1024;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Article {
    pub id: String,
    pub title: String,
    /// magnet: or an http(s) .torrent link.
    pub torrent: String,
    /// The item's web page, http(s) only.
    pub page: Option<String>,
    pub date: Option<u64>,
    pub size: Option<u64>,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub downloaded: bool,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub title: Option<String>,
    pub articles: Vec<Article>,
}

#[derive(Default)]
struct Item {
    title: String,
    link: Option<String>,
    alt_link: Option<String>,
    guid: Option<String>,
    enclosure: Option<String>,
    magnet: Option<String>,
    date: Option<u64>,
    size: Option<u64>,
}

fn resolve_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

fn clean(s: &str, max: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn link(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() > MAX_LINK || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    if s.starts_with("magnet:?") {
        return Some(s.to_string());
    }
    let u = url::Url::parse(s).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.to_string())
}

fn web(s: &str) -> Option<String> {
    link(s).filter(|l| !l.starts_with("magnet:"))
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

pub fn parse(xml: &[u8]) -> anyhow::Result<Parsed> {
    ensure!(xml.len() <= MAX_FEED_BYTES, "feed is too large");
    let text = String::from_utf8_lossy(xml);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    let mut out = Parsed::default();
    let mut item: Option<Item> = None;
    let mut field = String::new();
    let mut buf = String::new();
    let mut depth = 0usize;
    let mut saw_feed = false;

    loop {
        let ev = reader.read_event()?;
        let (start, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            _ => (None, false),
        };
        if let Some(e) = start {
            if !empty {
                depth += 1;
                ensure!(depth <= MAX_DEPTH, "feed is nested too deeply");
            }
            let name = e.local_name().as_ref().to_string();
            match name.as_str() {
                "rss" | "feed" | "RDF" => saw_feed = true,
                "item" | "entry" => {
                    ensure!(out.articles.len() < MAX_ITEMS, "feed has too many items");
                    item = Some(Item::default());
                }
                "enclosure" => {
                    if let Some(it) = item.as_mut() {
                        it.enclosure = attr(&e, "url");
                        it.size = attr(&e, "length").and_then(|l| l.parse().ok());
                    }
                }
                // Atom: rel="enclosure" is the torrent, anything else the page.
                "link" if empty || attr(&e, "href").is_some() => {
                    if let (Some(it), Some(href)) = (item.as_mut(), attr(&e, "href")) {
                        if attr(&e, "rel").as_deref() == Some("enclosure") {
                            it.enclosure = Some(href);
                            it.size = attr(&e, "length").and_then(|l| l.parse().ok());
                        } else {
                            it.alt_link.get_or_insert(href);
                        }
                    }
                    if !empty {
                        field = name;
                        buf.clear();
                    }
                }
                _ => {
                    field = name;
                    buf.clear();
                }
            }
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Text(t) => {
                if buf.len() < MAX_TEXT {
                    buf.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(r) => {
                if let Some(c) = resolve_entity(&r.into_inner()) {
                    buf.push(c);
                }
            }
            Event::CData(c) => {
                if buf.len() < MAX_TEXT {
                    buf.push_str(&c.into_inner());
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let name = e.local_name().as_ref().to_string();
                let value = std::mem::take(&mut buf);
                match item.as_mut() {
                    Some(it) if name == field => match name.as_str() {
                        "title" => it.title = clean(&value, MAX_TITLE),
                        "link" => it.link = Some(value.trim().to_string()),
                        "guid" | "id" => it.guid = Some(clean(&value, 512)),
                        "magnetURI" => it.magnet = Some(value.trim().to_string()),
                        "pubDate" => it.date = parse_rfc2822(&value).or(it.date),
                        "date" | "updated" | "published" => {
                            it.date = it.date.or(parse_iso8601(&value))
                        }
                        "contentLength" | "size" => it.size = value.trim().parse().ok(),
                        _ => {}
                    },
                    None if name == "title" && out.title.is_none() => {
                        out.title = Some(clean(&value, MAX_TITLE)).filter(|t| !t.is_empty());
                    }
                    _ => {}
                }
                if name == "item" || name == "entry" {
                    if let Some(it) = item.take() {
                        if let Some(a) = finish(it) {
                            out.articles.push(a);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ensure!(saw_feed, "not an RSS or Atom feed");
    Ok(out)
}

fn finish(it: Item) -> Option<Article> {
    let link_text = it.link.clone().or(it.alt_link.clone());
    let torrent = [
        it.magnet.as_deref(),
        it.enclosure.as_deref(),
        link_text.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(link)?;
    let page = [link_text.as_deref(), it.alt_link.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(web)
        .find(|p| *p != torrent);
    let title = if it.title.is_empty() {
        clean(&torrent, MAX_TITLE)
    } else {
        it.title
    };
    let id = it
        .guid
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| torrent.chars().take(512).collect());
    Some(Article {
        id,
        title,
        torrent,
        page,
        date: it.date,
        size: it.size,
        read: false,
        downloaded: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_items() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:torrent="http://xmlns.ezrss.it/0.1/">
<channel><title>Linux &amp; friends</title>
<item><title>Distro 12 &lt;x64&gt;</title><link>https://example.org/page/1</link>
<guid>abc-1</guid><pubDate>Mon, 20 Oct 2026 10:00:00 +0000</pubDate>
<enclosure url="https://example.org/1.torrent" length="1234" type="application/x-bittorrent"/></item>
<item><title><![CDATA[Magnet only]]></title><torrent:magnetURI>magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567</torrent:magnetURI></item>
<item><title>Bad link</title><link>javascript:alert(1)</link></item>
</channel></rss>"#;
        let p = parse(xml).unwrap();
        assert_eq!(p.title.as_deref(), Some("Linux & friends"));
        assert_eq!(p.articles.len(), 2);
        let a = &p.articles[0];
        assert_eq!(a.title, "Distro 12 <x64>");
        assert_eq!(a.torrent, "https://example.org/1.torrent");
        assert_eq!(a.page.as_deref(), Some("https://example.org/page/1"));
        assert_eq!(a.id, "abc-1");
        assert_eq!(a.size, Some(1234));
        assert!(a.date.is_some());
        assert!(p.articles[1].torrent.starts_with("magnet:?"));
    }

    #[test]
    fn atom_entries() {
        let xml = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title>
<entry><title>One</title><id>urn:1</id><updated>2026-10-20T10:00:00Z</updated>
<link rel="alternate" href="https://example.org/1"/>
<link rel="enclosure" href="https://example.org/1.torrent" length="99"/></entry></feed>"#;
        let p = parse(xml).unwrap();
        let a = &p.articles[0];
        assert_eq!(a.torrent, "https://example.org/1.torrent");
        assert_eq!(a.page.as_deref(), Some("https://example.org/1"));
        assert_eq!(a.size, Some(99));
        assert!(a.date.is_some());
    }

    #[test]
    fn hostile_feeds_are_refused_or_tamed() {
        assert!(parse(b"<html><body>hi</body></html>").is_err());
        let deep = format!("<rss>{}{}</rss>", "<a>".repeat(200), "</a>".repeat(200));
        assert!(parse(deep.as_bytes()).is_err());
        let many = format!(
            "<rss><channel>{}</channel></rss>",
            "<item><link>https://e.org/x.torrent</link></item>".repeat(MAX_ITEMS + 1)
        );
        assert!(parse(many.as_bytes()).is_err());
        assert!(parse(&vec![b' '; MAX_FEED_BYTES + 1]).is_err());
        // Entities are never expanded, so a "billion laughs" stays small.
        let laughs = br#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;">]>
<rss><channel><item><title>&b;&b;&b;</title><link>https://e.org/1.torrent</link></item></channel></rss>"#;
        let p = parse(laughs).unwrap();
        assert!(p.articles[0].title.len() < 100);
        let ctl = "<rss><channel><item><title>a\u{1b}[31mb\u{0}c</title><link>file:///etc/passwd</link><enclosure url=\"https://e.org/x y.torrent\"/></item></channel></rss>";
        assert!(parse(ctl.as_bytes()).unwrap().articles.is_empty());
        let ok = "<rss><channel><item><title>a\u{1b}[31mb</title><link>https://e.org/t</link></item></channel></rss>";
        assert_eq!(parse(ok.as_bytes()).unwrap().articles[0].title, "a [31mb");
    }
}
