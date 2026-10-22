//! Connected peers, typed and cleaned before they reach the UI.

use serde::Serialize;

use super::{parse_hash, Engine};

// Client names come from the peer's handshake, so they're capped.
const MAX_CLIENT_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PeerInfo {
    pub addr: String,
    pub client: Option<String>,
    pub state: &'static str,
    /// "tcp", "utp" or "socks".
    pub conn: Option<&'static str>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub country: Option<String>,
}

/// Drops control and bidi characters, collapses whitespace, and caps the length.
pub(crate) fn clean_client(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = 0;
    for word in name
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
    {
        let word: String = word.chars().filter(|c| !is_format_char(*c)).collect();
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            if chars + 1 >= MAX_CLIENT_CHARS {
                break;
            }
            out.push(' ');
            chars += 1;
        }
        for c in word.chars() {
            if chars == MAX_CLIENT_CHARS {
                out.push('…');
                return Some(out);
            }
            out.push(c);
            chars += 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

// Bidi overrides and other invisible marks that can reorder or hide text.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}'
    )
}

fn conn_name(kind: impl Serialize) -> Option<&'static str> {
    match serde_json::to_value(kind).ok()?.as_str()? {
        "tcp" => Some("tcp"),
        "utp" => Some("utp"),
        "socks" => Some("socks"),
        _ => None,
    }
}

impl Engine {
    pub fn peers(&self, id: &str) -> anyhow::Result<Vec<PeerInfo>> {
        let t = self.handle(&parse_hash(id)?)?;
        let Some(live) = t.live() else {
            return Ok(Vec::new());
        };
        let mut peers: Vec<PeerInfo> = live
            .per_peer_stats_snapshot(Default::default())
            .peers
            .into_iter()
            .map(|(addr, p)| PeerInfo {
                addr,
                client: p.client_name.as_deref().and_then(clean_client),
                state: p.state,
                conn: p.conn_kind.and_then(conn_name),
                downloaded: p.counters.fetched_bytes,
                uploaded: p.counters.uploaded_bytes,
                country: None,
            })
            .collect();
        self.add_countries(&mut peers);
        Ok(peers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_names_are_cleaned() {
        assert_eq!(
            clean_client("  Client  1.2.3 ").as_deref(),
            Some("Client 1.2.3")
        );
        assert_eq!(
            clean_client("Tr\u{202E}exe.mp4").as_deref(),
            Some("Trexe.mp4")
        );
        assert_eq!(clean_client("a\n\r\tb\u{0}c").as_deref(), Some("a b c"));
        assert_eq!(clean_client("\u{200B}\u{FEFF}"), None);
        assert_eq!(clean_client(""), None);
        let long = clean_client(&"x".repeat(10_000)).unwrap();
        assert_eq!(long.chars().count(), MAX_CLIENT_CHARS + 1);
        assert!(long.ends_with('…'));
        let words = clean_client(&"ab ".repeat(100)).unwrap();
        assert!(words.chars().count() <= MAX_CLIENT_CHARS + 1);
    }

    #[test]
    fn random_client_names_stay_clean() {
        use rand::{RngExt, SeedableRng};
        let alphabet: Vec<char> =
            "ab 1.\t\n\r\u{0}\u{7}\u{1b}\u{202E}\u{2066}\u{200B}\u{FEFF}\u{061C}é漢😀"
                .chars()
                .collect();
        let mut rng = rand::rngs::StdRng::seed_from_u64(31);
        for _ in 0..5_000 {
            let len = rng.random_range(0..300);
            let name: String = (0..len)
                .map(|_| alphabet[rng.random_range(0..alphabet.len())])
                .collect();
            if let Some(c) = clean_client(&name) {
                assert!(c.chars().count() <= MAX_CLIENT_CHARS + 1, "{c:?}");
                assert!(
                    !c.chars().any(|ch| ch.is_control() || is_format_char(ch)),
                    "{c:?}"
                );
                assert_eq!(c.trim(), c);
                assert!(!c.contains("  "));
            }
        }
    }

    #[test]
    fn connection_kinds_map_to_known_names() {
        assert_eq!(conn_name("utp"), Some("utp"));
        assert_eq!(conn_name("quic"), None);
        assert_eq!(conn_name(7), None);
    }
}
