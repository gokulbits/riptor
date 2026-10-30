//! Tidying librqbit's session folder.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// What librqbit saved about a torrent, enough to add it again.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: usize,
    pub only_files: Option<Vec<usize>>,
    pub trackers: Vec<String>,
}

/// The saved torrents by lowercase hash; empty if the file can't be read.
pub fn entries(dir: &Path) -> HashMap<String, Entry> {
    let Some(v) = std::fs::read(dir.join("session.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    else {
        return HashMap::new();
    };
    let Some(torrents) = v.get("torrents").and_then(|t| t.as_object()) else {
        return HashMap::new();
    };
    torrents
        .iter()
        .filter_map(|(id, t)| {
            let hash = t.get("info_hash")?.as_str()?.to_ascii_lowercase();
            let entry = Entry {
                id: id.parse().ok()?,
                only_files: serde_json::from_value(t.get("only_files")?.clone()).ok()?,
                trackers: serde_json::from_value(t.get("trackers")?.clone()).ok()?,
            };
            Some((hash, entry))
        })
        .collect()
}

/// Drops entries with no Riptor record. Returns the hashes removed.
pub fn prune(dir: &Path, known: &HashSet<String>) -> anyhow::Result<Vec<String>> {
    let db = dir.join("session.json");
    let Ok(bytes) = std::fs::read(&db) else {
        return Ok(Vec::new());
    };
    let mut v: serde_json::Value = serde_json::from_slice(&bytes)?;
    let Some(torrents) = v.get_mut("torrents").and_then(|t| t.as_object_mut()) else {
        return Ok(Vec::new());
    };
    let is_hash = |h: &str| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let mut removed = Vec::new();
    torrents.retain(|_, t| {
        let hash = t
            .get("info_hash")
            .and_then(|h| h.as_str())
            .map(str::to_ascii_lowercase);
        match hash {
            Some(h) if is_hash(&h) && !known.contains(&h) => {
                removed.push(h);
                false
            }
            _ => true,
        }
    });
    if removed.is_empty() {
        return Ok(removed);
    }
    crate::state_file::save_bytes(&db, &serde_json::to_vec(&v)?)?;
    for h in &removed {
        for ext in ["torrent", "bitv"] {
            let _ = std::fs::remove_file(dir.join(format!("{h}.{ext}")));
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEPT: &str = "0123456789abcdef0123456789abcdef01234567";
    const ORPHAN: &str = "5b1e0d988fc7a0c9e99bd852071681a59974b39f";

    #[test]
    fn removes_only_unknown_torrents() {
        let d = tempfile::tempdir().unwrap();
        let db = format!(
            r#"{{"torrents":{{"0":{{"info_hash":"{KEPT}"}},"1":{{"info_hash":"{ORPHAN}"}},"2":{{"info_hash":"../../evil"}}}}}}"#
        );
        std::fs::write(d.path().join("session.json"), db).unwrap();
        for h in [KEPT, ORPHAN] {
            std::fs::write(d.path().join(format!("{h}.torrent")), b"x").unwrap();
            std::fs::write(d.path().join(format!("{h}.bitv")), b"x").unwrap();
        }
        let known = HashSet::from([KEPT.to_string()]);
        assert_eq!(prune(d.path(), &known).unwrap(), vec![ORPHAN.to_string()]);

        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.path().join("session.json")).unwrap()).unwrap();
        let left: Vec<_> = v["torrents"]
            .as_object()
            .unwrap()
            .values()
            .map(|t| t["info_hash"].as_str().unwrap())
            .collect();
        assert!(left.contains(&KEPT) && !left.contains(&ORPHAN));
        // A malformed hash is never used to build a file name; the entry stays.
        assert!(left.contains(&"../../evil"));
        assert!(d.path().join(format!("{KEPT}.torrent")).exists());
        assert!(!d.path().join(format!("{ORPHAN}.torrent")).exists());
        assert!(!d.path().join(format!("{ORPHAN}.bitv")).exists());
        // Second run: nothing to do.
        assert!(prune(d.path(), &known).unwrap().is_empty());
    }

    #[test]
    fn entries_skip_anything_malformed() {
        let d = tempfile::tempdir().unwrap();
        assert!(entries(d.path()).is_empty());
        std::fs::write(d.path().join("session.json"), b"[1,2,3]").unwrap();
        assert!(entries(d.path()).is_empty());
        let db = format!(
            r#"{{"torrents":{{
                "7":{{"info_hash":"{KEPT}","only_files":[0,2],"trackers":["http://t/"]}},
                "x":{{"info_hash":"{ORPHAN}","only_files":null,"trackers":[]}},
                "8":{{"info_hash":"{ORPHAN}","only_files":"nope","trackers":[]}}
            }}}}"#
        );
        std::fs::write(d.path().join("session.json"), db).unwrap();
        let got = entries(d.path());
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[KEPT],
            Entry {
                id: 7,
                only_files: Some(vec![0, 2]),
                trackers: vec!["http://t/".into()],
            }
        );
    }

    #[test]
    fn leaves_unexpected_files_alone() {
        let d = tempfile::tempdir().unwrap();
        assert!(prune(d.path(), &HashSet::new()).unwrap().is_empty());
        std::fs::write(d.path().join("session.json"), b"[1,2,3]").unwrap();
        assert!(prune(d.path(), &HashSet::new()).unwrap().is_empty());
        assert_eq!(
            std::fs::read(d.path().join("session.json")).unwrap(),
            b"[1,2,3]"
        );
    }
}
