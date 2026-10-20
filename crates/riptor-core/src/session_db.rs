//! Tidying librqbit's session folder.

use std::collections::HashSet;
use std::path::Path;

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
