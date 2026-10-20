//! The watched folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::input::MAX_TORRENT_BYTES;

/// Files handled per scan, so a folder full of junk can't stall the engine.
pub const MAX_PER_SCAN: usize = 16;
/// Skip files that may still be being written.
const SETTLE: Duration = Duration::from_secs(2);

/// `.torrent` files ready to add, sorted by name.
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let p = e.path();
            let is_torrent = p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("torrent"));
            // DirEntry::metadata doesn't follow symlinks.
            let Ok(meta) = e.metadata() else {
                return false;
            };
            let settled = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= SETTLE);
            is_torrent
                && meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() <= MAX_TORRENT_BYTES as u64
                && settled
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found.truncate(MAX_PER_SCAN);
    found
}

/// Renames to `.added` or `.invalid` so it isn't picked up again.
pub fn mark(path: &Path, added: bool) -> std::io::Result<PathBuf> {
    let suffix = if added { "added" } else { "invalid" };
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_string_lossy()
        .into_owned();
    for n in 0..100 {
        let candidate = if n == 0 {
            path.with_file_name(format!("{name}.{suffix}"))
        } else {
            path.with_file_name(format!("{name}.{n}.{suffix}"))
        };
        if !candidate.exists() {
            std::fs::rename(path, &candidate)?;
            return Ok(candidate);
        }
    }
    Err(std::io::Error::other(
        "too many handled copies of this file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn age(p: &Path) {
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    #[test]
    fn picks_only_settled_torrent_files() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a.torrent", "B.TORRENT", "notes.txt", "fresh.torrent"] {
            std::fs::write(d.path().join(n), b"d4:infod4:name1:xee").unwrap();
        }
        std::fs::create_dir(d.path().join("dir.torrent")).unwrap();
        std::fs::write(
            d.path().join("huge.torrent"),
            vec![0u8; MAX_TORRENT_BYTES + 1],
        )
        .unwrap();
        for n in ["a.torrent", "B.TORRENT", "notes.txt", "huge.torrent"] {
            age(&d.path().join(n));
        }
        let names: Vec<String> = scan(d.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["B.TORRENT", "a.torrent"]);
    }

    #[test]
    fn handled_files_are_renamed_without_clobbering() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.torrent");
        std::fs::write(d.path().join("x.torrent.added"), b"older").unwrap();
        std::fs::write(&p, b"new").unwrap();
        let to = mark(&p, true).unwrap();
        assert_eq!(to.file_name().unwrap(), "x.torrent.1.added");
        assert_eq!(
            std::fs::read(d.path().join("x.torrent.added")).unwrap(),
            b"older"
        );
        std::fs::write(&p, b"bad").unwrap();
        assert_eq!(
            mark(&p, false).unwrap().file_name().unwrap(),
            "x.torrent.invalid"
        );
        assert!(scan(d.path()).is_empty());
    }
}
