//! Mutated indexer replies: no panics, and only safe links get through.

use riptor_search::{archive, catalog, clean, dedupe_key, scrape, time, torznab, SearchResult};

/// xorshift64*, seeded, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SNIPPETS: &[&str] = &[
    "<item>",
    "</item>",
    "<![CDATA[",
    "]]>",
    "&amp;",
    "&#0;",
    "&#xFFFFFFFF;",
    "javascript:alert(1)",
    "file:///C:/Windows/win.ini",
    "magnet:?xt=urn:btih:",
    "\"",
    "{",
    "[",
    "99999999999999999999999",
    "é",
    "\u{202e}",
];

fn mutate(rng: &mut Rng, base: &str) -> String {
    let mut s: Vec<char> = base.chars().collect();
    for _ in 0..1 + rng.below(4) {
        let i = rng.below(s.len() + 1).min(s.len());
        match rng.below(3) {
            0 => s.truncate(i),
            1 if i < s.len() => {
                s.remove(i);
            }
            _ => {
                let snip = SNIPPETS[rng.below(SNIPPETS.len())];
                s.splice(i..i, snip.chars());
            }
        }
    }
    s.into_iter().collect()
}

fn check(results: Vec<SearchResult>) {
    for r in results {
        let Some(r) = clean(r) else { continue };
        assert!(
            r.link.starts_with("magnet:?")
                || r.link.starts_with("http://")
                || r.link.starts_with("https://"),
            "{}",
            r.link
        );
        if let Some(d) = &r.desc_link {
            assert!(d.starts_with("http://") || d.starts_with("https://"), "{d}");
        }
        assert!(!r.name.chars().any(char::is_control));
        let _ = dedupe_key(&r);
    }
}

const TORZNAB: &str = r#"<?xml version="1.0"?>
<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>Debian &amp; friends</title><comments>https://i.example/1</comments>
<pubDate>Sun, 18 Oct 2026 00:00:00 +0000</pubDate><size>1048576</size>
<link>https://i.example/dl/1.torrent</link>
<torznab:attr name="seeders" value="12"/><torznab:attr name="peers" value="15"/>
<torznab:attr name="magneturl" value="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"/>
</item></channel></rss>"#;

const ARCHIVE: &str = r#"{"response":{"numFound":1,"docs":[{"identifier":"debian-12","title":"Debian 12","item_size":123,"publicdate":"2026-10-18T00:00:00Z"}]}}"#;

#[test]
fn mutated_torznab_replies() {
    let mut rng = Rng(1);
    for _ in 0..20_000 {
        if let Ok(r) = torznab::parse(&mutate(&mut rng, TORZNAB), "indexer") {
            check(r);
        }
    }
    // Deep nesting is just ignored structure, not recursion.
    let deep = format!(
        "<rss>{}{}</rss>",
        "<a>".repeat(200_000),
        "</a>".repeat(200_000)
    );
    let _ = torznab::parse(&deep, "indexer");
}

#[test]
fn mutated_archive_replies() {
    let mut rng = Rng(2);
    for _ in 0..20_000 {
        if let Ok(r) = archive::parse(&mutate(&mut rng, ARCHIVE)) {
            check(r);
        }
    }
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(archive::parse(&deep).is_err());
}

#[test]
fn hostile_results_are_cleaned() {
    let mut rng = Rng(3);
    for _ in 0..20_000 {
        let r = SearchResult {
            name: mutate(&mut rng, "Some \u{7}name"),
            link: mutate(
                &mut rng,
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            ),
            size: Some(rng.next()),
            seeders: Some(rng.next() as u32),
            leechers: None,
            desc_link: Some(mutate(&mut rng, "https://i.example/details")),
            pub_date: Some(rng.next()),
            engine: "x".into(),
            via_plugin: true,
        };
        check(vec![r]);
    }
}

#[test]
fn scrape_and_catalog_parsers() {
    let mut rng = Rng(4);
    let hash = [7u8; 20];
    for _ in 0..20_000 {
        let len = rng.below(120);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = scrape::parse_udp_scrape(&bytes);
        let _ = scrape::parse_http_reply(&bytes, &hash);
        let text = String::from_utf8_lossy(&bytes);
        let _ = scrape::parse_btih(&text);
        let _ = time::parse_iso8601(&text);
        let _ = time::parse_rfc2822(&text);
        let page = mutate(
            &mut rng,
            "| [Name](https://x.example/a.py) | 1.0 | [link](https://x.example/raw/a.py) |\n",
        );
        let _ = catalog::parse_community(&page);
        let _ = catalog::parse_official_index(&page);
    }
    let mut deep = b"d5:filesd20:".to_vec();
    deep.extend_from_slice(&hash);
    deep.extend(std::iter::repeat_n(b'l', 100_000));
    assert!(scrape::parse_http_reply(&deep, &hash).is_err());
}
