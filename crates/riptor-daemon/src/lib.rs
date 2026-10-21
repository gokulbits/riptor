//! Headless mode: `riptord` runs the engine; `riptor-cli` talks to it over a private Unix socket.
#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

/// One request per line; longer lines are refused.
pub const MAX_LINE: usize = 64 * 1024;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    List,
    Stats,
    Add {
        /// A magnet link, an http(s) .torrent link or a .torrent file path.
        source: String,
        dir: Option<PathBuf>,
        category: Option<String>,
        paused: bool,
    },
    Stop {
        ids: Vec<String>,
    },
    Start {
        ids: Vec<String>,
    },
    Remove {
        ids: Vec<String>,
        delete_files: bool,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl Response {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            error: None,
            data,
        }
    }

    pub fn err(e: &anyhow::Error) -> Self {
        Self {
            ok: false,
            error: Some(format!("{e:#}")),
            data: serde_json::Value::Null,
        }
    }
}

/// `all`, full info hashes, or unique prefixes of at least 6 characters.
pub fn resolve_ids(known: &[String], wanted: &[String]) -> anyhow::Result<Vec<String>> {
    ensure!(!wanted.is_empty(), "name at least one torrent, or all");
    if wanted.iter().any(|w| w == "all") {
        return Ok(known.to_vec());
    }
    let mut out = Vec::new();
    for w in wanted {
        let w = w.to_ascii_lowercase();
        ensure!(
            w.len() >= 6 && w.chars().all(|c| c.is_ascii_hexdigit()),
            "{w}: use at least 6 characters of the info hash"
        );
        let hits: Vec<&String> = known.iter().filter(|k| k.starts_with(&w)).collect();
        match hits.as_slice() {
            [one] => out.push((*one).clone()),
            [] => bail!("no torrent matches {w}"),
            _ => bail!("{w} matches more than one torrent; use more characters"),
        }
    }
    out.dedup();
    Ok(out)
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Separate from the desktop app's data, so both can be installed.
pub fn default_data_dir() -> anyhow::Result<PathBuf> {
    let base = env_dir("XDG_DATA_HOME")
        .or_else(|| env_dir("HOME").map(|h| h.join(".local").join("share")))
        .ok_or_else(|| anyhow::anyhow!("set HOME or pass --data-dir"))?;
    Ok(base.join("riptor-headless"))
}

#[cfg(unix)]
pub fn socket_dir() -> PathBuf {
    match env_dir("XDG_RUNTIME_DIR") {
        Some(d) => d.join("riptor"),
        None => std::env::temp_dir().join(format!("riptor-{}", rustix::process::getuid().as_raw())),
    }
}

#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    socket_dir().join("control.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let r = Request::Add {
            source: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
            dir: None,
            category: Some("Linux".into()),
            paused: true,
        };
        let line = serde_json::to_string(&r).unwrap();
        assert!(line.starts_with("{\"cmd\":\"add\""));
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
        assert!(serde_json::from_str::<Request>("{\"cmd\":\"rm -rf\"}").is_err());
    }

    #[test]
    fn ids_resolve_by_unique_prefix() {
        let known = vec![
            "abcdef0001".to_string(),
            "abcdef0002".to_string(),
            "123456ff".to_string(),
        ];
        let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve_ids(&known, &w(&["all"])).unwrap(), known);
        assert_eq!(
            resolve_ids(&known, &w(&["123456"])).unwrap(),
            w(&["123456ff"])
        );
        assert_eq!(
            resolve_ids(&known, &w(&["ABCDEF0002"])).unwrap(),
            w(&["abcdef0002"])
        );
        assert!(resolve_ids(&known, &w(&["abcdef"])).is_err(), "ambiguous");
        assert!(resolve_ids(&known, &w(&["abc"])).is_err(), "too short");
        assert!(resolve_ids(&known, &w(&["zzzzzz"])).is_err());
        assert!(resolve_ids(&known, &[]).is_err());
    }
}
