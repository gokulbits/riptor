//! Internet Archive. Every item has an `{id}_archive.torrent`.

use serde::Deserialize;

use crate::{time, SearchResult};

pub const ID: &str = "archive";
pub const NAME: &str = "Internet Archive";
pub const URL: &str = "https://archive.org";

fn mediatype(category: &str) -> Option<&'static str> {
    match category {
        "movies" | "tv" | "anime" => Some("movies"),
        "music" => Some("audio"),
        "books" => Some("texts"),
        "software" | "games" => Some("software"),
        "pictures" => Some("image"),
        _ => None,
    }
}

pub fn categories() -> Vec<String> {
    [
        "all", "movies", "tv", "music", "books", "software", "games", "pictures",
    ]
    .map(String::from)
    .to_vec()
}

#[derive(Deserialize)]
struct Response {
    response: Docs,
}

#[derive(Deserialize)]
struct Docs {
    docs: Vec<Doc>,
}

#[derive(Deserialize)]
struct Doc {
    identifier: String,
    #[serde(default)]
    title: Option<serde_json::Value>,
    #[serde(default)]
    item_size: Option<u64>,
    #[serde(default)]
    publicdate: Option<String>,
}

fn safe_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

pub fn parse(body: &str) -> anyhow::Result<Vec<SearchResult>> {
    let r: Response = serde_json::from_str(body)?;
    Ok(r.response
        .docs
        .into_iter()
        .filter(|d| safe_identifier(&d.identifier))
        .map(|d| {
            let title = match d.title {
                Some(serde_json::Value::String(s)) => s,
                Some(serde_json::Value::Array(a)) => a
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                _ => d.identifier.clone(),
            };
            SearchResult {
                name: title,
                link: format!(
                    "https://archive.org/download/{0}/{0}_archive.torrent",
                    d.identifier
                ),
                size: d.item_size,
                seeders: None,
                leechers: None,
                desc_link: Some(format!("https://archive.org/details/{}", d.identifier)),
                pub_date: d.publicdate.as_deref().and_then(time::parse_iso8601),
                engine: ID.to_string(),
                via_plugin: false,
            }
        })
        .collect())
}

pub async fn search(
    http: &reqwest::Client,
    query: &str,
    category: &str,
) -> anyhow::Result<Vec<SearchResult>> {
    let q = match mediatype(category) {
        Some(m) => format!("({query}) AND mediatype:({m})"),
        None => format!(
            "({query}) AND mediatype:(movies OR audio OR texts OR software OR image OR data)"
        ),
    };
    let body = http
        .get("https://archive.org/advancedsearch.php")
        .query(&[
            ("q", q.as_str()),
            ("fl[]", "identifier"),
            ("fl[]", "title"),
            ("fl[]", "item_size"),
            ("fl[]", "publicdate"),
            ("sort[]", "downloads desc"),
            ("rows", "100"),
            ("page", "1"),
            ("output", "json"),
        ])
        .send()
        .await;
    riptor_net::ledger::record(
        riptor_net::ledger::Purpose::Search,
        "archive.org",
        body.is_ok(),
    );
    let body = body?.error_for_status()?.text().await?;
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_filters() {
        let body = r#"{"response":{"numFound":3,"docs":[
          {"identifier":"debian-12","title":"Debian 12","item_size":1024,"publicdate":"2023-06-10T00:00:00Z"},
          {"identifier":"multi","title":["First title","Second"]},
          {"identifier":"../../evil","title":"bad"}
        ]}}"#;
        let r = parse(body).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].link,
            "https://archive.org/download/debian-12/debian-12_archive.torrent"
        );
        assert_eq!(r[0].size, Some(1024));
        assert!(r[0].pub_date.is_some());
        assert_eq!(r[1].name, "First title");
    }
}
