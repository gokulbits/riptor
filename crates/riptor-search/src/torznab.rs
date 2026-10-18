//! Torznab (Jackett, Prowlarr). Parsed with quick-xml, which never resolves
//! DTDs or external entities, so a hostile indexer cannot read local files.

use std::net::IpAddr;

use anyhow::{bail, ensure};
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::{time, SearchResult};

/// Newznab category ids per Riptor category.
fn cat_ids(category: &str) -> Option<&'static str> {
    match category {
        "movies" => Some("2000"),
        "tv" => Some("5000"),
        "anime" => Some("5070"),
        "music" => Some("3000"),
        "games" => Some("1000,4050"),
        "software" => Some("4000"),
        "books" => Some("7000"),
        "pictures" => Some("8000"),
        _ => None,
    }
}

pub fn categories() -> Vec<String> {
    crate::CATEGORIES.iter().map(|c| c.to_string()).collect()
}

/// https anywhere; plain http only to this machine or a private network,
/// since the API key travels in the URL.
pub fn validate_endpoint(raw: &str) -> anyhow::Result<url::Url> {
    let u = url::Url::parse(raw.trim())?;
    ensure!(
        u.username().is_empty() && u.password().is_none(),
        "put the API key in its own field, not in the URL"
    );
    match u.scheme() {
        "https" => {}
        "http" => {
            let local = match u.host() {
                Some(url::Host::Domain(d)) => {
                    d == "localhost" || d.ends_with(".local") || d.ends_with(".lan")
                }
                Some(url::Host::Ipv4(ip)) => {
                    ip.is_loopback() || ip.is_private() || ip.is_link_local()
                }
                Some(url::Host::Ipv6(ip)) => {
                    ip.is_loopback() || IpAddr::V6(ip).to_string().starts_with("fd")
                }
                None => false,
            };
            ensure!(
                local,
                "use https for indexers that are not on this computer or your local network"
            );
        }
        _ => bail!("only http(s) indexer URLs are supported"),
    }
    Ok(u)
}

#[derive(Default)]
struct Item {
    title: String,
    link: Option<String>,
    comments: Option<String>,
    guid: Option<String>,
    size: Option<u64>,
    pub_date: Option<u64>,
    seeders: Option<u32>,
    peers: Option<u32>,
    magnet: Option<String>,
    enclosure: Option<String>,
}

pub fn parse(xml: &str, engine: &str) -> anyhow::Result<Vec<SearchResult>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut item: Option<Item> = None;
    let mut field: Vec<u8> = Vec::new();

    loop {
        match reader.read_event()? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().as_ref().to_vec();
                match name.as_slice() {
                    b"error" if item.is_none() => {
                        let desc = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"description")
                            .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
                            .unwrap_or_else(|| "indexer error".into());
                        bail!(
                            "indexer error: {}",
                            desc.chars().take(200).collect::<String>()
                        );
                    }
                    b"item" => item = Some(Item::default()),
                    b"enclosure" | b"attr" => {
                        if let Some(it) = item.as_mut() {
                            let mut n = None;
                            let mut v = None;
                            let mut url = None;
                            let mut len = None;
                            for a in e.attributes().flatten() {
                                let val = a
                                    .unescape_value()
                                    .map(|x| x.into_owned())
                                    .unwrap_or_default();
                                match a.key.local_name().as_ref() {
                                    b"name" => n = Some(val),
                                    b"value" => v = Some(val),
                                    b"url" => url = Some(val),
                                    b"length" => len = val.parse().ok(),
                                    _ => {}
                                }
                            }
                            if name.as_slice() == b"enclosure" {
                                it.enclosure = url;
                                it.size = it.size.or(len);
                            } else if let (Some(n), Some(v)) = (n, v) {
                                match n.as_str() {
                                    "seeders" => it.seeders = v.parse().ok(),
                                    "peers" => it.peers = v.parse().ok(),
                                    "magneturl" => it.magnet = Some(v),
                                    "size" => it.size = v.parse().ok().or(it.size),
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => field = name,
                }
            }
            Event::Text(t) => {
                if let Some(it) = item.as_mut() {
                    let text = t.unescape().map(|x| x.into_owned()).unwrap_or_default();
                    set_field(it, &field, text);
                }
            }
            Event::CData(c) => {
                if let Some(it) = item.as_mut() {
                    set_field(
                        it,
                        &field,
                        String::from_utf8_lossy(&c.into_inner()).into_owned(),
                    );
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"item" {
                    if let Some(it) = item.take() {
                        // Jackett leechers = peers - seeders.
                        let leechers = match (it.peers, it.seeders) {
                            (Some(p), Some(s)) => Some(p.saturating_sub(s)),
                            _ => None,
                        };
                        let link = it.magnet.or(it.enclosure).or(it.link);
                        if let Some(link) = link {
                            out.push(SearchResult {
                                name: it.title,
                                link,
                                size: it.size,
                                seeders: it.seeders,
                                leechers,
                                desc_link: it.comments.or(it.guid),
                                pub_date: it.pub_date,
                                engine: engine.to_string(),
                                via_plugin: false,
                            });
                        }
                    }
                }
                field.clear();
            }
            _ => {}
        }
    }
    Ok(out)
}

fn set_field(it: &mut Item, field: &[u8], text: String) {
    match field {
        b"title" => it.title = text,
        b"link" => it.link = Some(text),
        b"comments" => it.comments = Some(text),
        b"guid" => it.guid = Some(text),
        b"size" => it.size = text.trim().parse().ok().or(it.size),
        b"pubDate" => it.pub_date = time::parse_rfc2822(&text),
        _ => {}
    }
}

pub async fn search(
    http: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    engine: &str,
    query: &str,
    category: &str,
) -> anyhow::Result<Vec<SearchResult>> {
    let base = validate_endpoint(endpoint)?;
    let mut req = http.get(base).query(&[
        ("t", "search"),
        ("q", query),
        ("apikey", api_key),
        ("limit", "100"),
    ]);
    if let Some(c) = cat_ids(category) {
        req = req.query(&[("cat", c)]);
    }
    let resp = req.send().await?.error_for_status()?;
    if let Some(len) = resp.content_length() {
        ensure!(len <= 16 << 20, "indexer response too large");
    }
    let body = resp.text().await?;
    ensure!(body.len() <= 16 << 20, "indexer response too large");
    parse(&body, engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE rss [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
<channel>
  <item>
    <title>Debian 12 &amp; friends</title>
    <guid>https://indexer.example/details/1</guid>
    <comments>https://indexer.example/details/1</comments>
    <pubDate>Sun, 18 Oct 2026 00:00:00 +0000</pubDate>
    <size>1048576</size>
    <link>https://indexer.example/dl/1.torrent</link>
    <enclosure url="https://indexer.example/dl/1.torrent" length="1048576" type="application/x-bittorrent"/>
    <torznab:attr name="seeders" value="12"/>
    <torznab:attr name="peers" value="15"/>
    <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"/>
  </item>
  <item>
    <title><![CDATA[Second <b>item</b>]]></title>
    <link>https://indexer.example/dl/2.torrent</link>
  </item>
  <item><title>no link, dropped</title></item>
  <item><title>&xxe;</title><link>https://x.example/3.torrent</link></item>
</channel></rss>"#;

    #[test]
    fn parses_items() {
        let r = parse(SAMPLE, "jackett").unwrap();
        assert_eq!(r[0].name, "Debian 12 & friends");
        assert!(r[0].link.starts_with("magnet:?"));
        assert_eq!(
            (r[0].seeders, r[0].leechers, r[0].size),
            (Some(12), Some(3), Some(1_048_576))
        );
        assert_eq!(r[0].pub_date, Some(1_792_281_600));
        assert_eq!(r[1].name, "Second <b>item</b>");
        assert_eq!(r[1].link, "https://indexer.example/dl/2.torrent");
        // The external entity is never expanded into file contents.
        assert!(r.iter().all(|x| !x.name.contains("root:")));
    }

    #[test]
    fn indexer_errors_surface() {
        let e = parse(r#"<error code="100" description="Invalid API key"/>"#, "j").unwrap_err();
        assert!(e.to_string().contains("Invalid API key"));
    }

    #[test]
    fn endpoint_policy() {
        assert!(validate_endpoint(
            "https://jackett.example.com/api/v2.0/indexers/all/results/torznab/"
        )
        .is_ok());
        assert!(validate_endpoint("http://127.0.0.1:9117/api").is_ok());
        assert!(validate_endpoint("http://192.168.1.5:9696/1/api").is_ok());
        assert!(validate_endpoint("http://jackett.example.com/api").is_err());
        assert!(validate_endpoint("https://user:pass@x.example/api").is_err());
        assert!(validate_endpoint("file:///etc/passwd").is_err());
    }
}
