//! Torrents from another client's folder of `.torrent` + `.fastresume` pairs.

use std::path::{Path, PathBuf};

use librqbit_bencode::BencodeValue;
use serde::Serialize;

const MAX_ENTRIES: usize = 5000;
const MAX_RESUME_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Found {
    pub torrent: PathBuf,
    pub save_path: PathBuf,
    pub paused: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    pub added: usize,
    pub already_present: usize,
    /// Resume files without a usable .torrent or save folder.
    pub skipped: usize,
    pub failed: Vec<String>,
}

/// The save folder and paused flag from a resume file, if it has a usable one.
pub fn parse_fastresume(bytes: &[u8]) -> Option<(PathBuf, bool)> {
    let BencodeValue::Dict(top) = librqbit_bencode::dyn_from_bytes::<Vec<u8>>(bytes).ok()? else {
        return None;
    };
    let save = match top.get(&b"save_path"[..])? {
        BencodeValue::Bytes(b) => String::from_utf8(b.clone()).ok()?,
        _ => return None,
    };
    let save = PathBuf::from(save.trim());
    if !save.is_absolute() || save.as_os_str().len() > 4096 {
        return None;
    }
    let paused = matches!(top.get(&b"paused"[..]), Some(BencodeValue::Integer(1)));
    Some((save, paused))
}

/// `(found, skipped)`; only plain files directly in `dir` are read.
pub fn scan(dir: &Path) -> anyhow::Result<(Vec<Found>, usize)> {
    let mut found = Vec::new();
    let mut skipped = 0;
    for entry in std::fs::read_dir(dir)?.flatten().take(MAX_ENTRIES * 2) {
        let path = entry.path();
        let is_resume = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fastresume"));
        if !is_resume || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if found.len() + skipped >= MAX_ENTRIES {
            break;
        }
        let torrent = path.with_extension("torrent");
        let usable = entry.metadata().is_ok_and(|m| m.len() <= MAX_RESUME_BYTES)
            && std::fs::symlink_metadata(&torrent).is_ok_and(|m| m.is_file());
        let parsed = usable
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|b| parse_fastresume(&b));
        match parsed {
            Some((save_path, paused)) => found.push(Found {
                torrent,
                save_path,
                paused,
            }),
            None => skipped += 1,
        }
    }
    Ok((found, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs() -> String {
        std::env::temp_dir()
            .join("dl")
            .to_string_lossy()
            .into_owned()
    }

    fn resume(save: &str, paused: i64) -> Vec<u8> {
        format!("d6:pausedi{paused}e9:save_path{}:{save}e", save.len()).into_bytes()
    }

    #[test]
    fn reads_save_folder_and_paused_flag() {
        let save = abs();
        assert_eq!(
            parse_fastresume(&resume(&save, 1)),
            Some((PathBuf::from(&save), true))
        );
        assert_eq!(
            parse_fastresume(&resume(&save, 0)),
            Some((PathBuf::from(&save), false))
        );
    }

    #[test]
    fn hostile_resume_files_are_ignored() {
        assert_eq!(parse_fastresume(b"not bencode"), None);
        assert_eq!(parse_fastresume(&resume("relative/path", 0)), None);
        assert_eq!(parse_fastresume(b"d9:save_pathi5ee"), None);
        assert_eq!(parse_fastresume(b"d9:save_path2:\xff\xfee"), None);
        let deep = format!("{}{}", "l".repeat(100_000), "e".repeat(100_000));
        assert_eq!(parse_fastresume(deep.as_bytes()), None);
    }

    #[test]
    fn pairs_resume_files_with_torrents() {
        let d = tempfile::tempdir().unwrap();
        let save = abs();
        std::fs::write(d.path().join("aaa.fastresume"), resume(&save, 0)).unwrap();
        std::fs::write(d.path().join("aaa.torrent"), b"d4:infod4:name1:xee").unwrap();
        // No .torrent next to it.
        std::fs::write(d.path().join("bbb.fastresume"), resume(&save, 0)).unwrap();
        std::fs::write(d.path().join("ccc.fastresume"), b"garbage").unwrap();
        std::fs::write(d.path().join("ccc.torrent"), b"x").unwrap();
        let (found, skipped) = scan(d.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].torrent, d.path().join("aaa.torrent"));
        assert_eq!(skipped, 2);
    }
}
