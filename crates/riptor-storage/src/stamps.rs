//! What each torrent's files looked like when Riptor last knew its progress was right.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::layout::Layout;

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct Inner {
    path: Option<PathBuf>,
    known: BTreeMap<String, String>,
    dirty: bool,
}

#[derive(Default)]
pub struct Stamps(Mutex<Inner>);

impl Stamps {
    /// A missing or damaged file only means torrents get sampled once more.
    pub fn open(&self, path: PathBuf) {
        let known = read(&path).unwrap_or_default();
        *self.0.lock() = Inner {
            path: Some(path),
            known,
            dirty: false,
        };
    }

    pub fn matches(&self, hash: &str, layout: &Layout) -> bool {
        if !self.0.lock().known.contains_key(hash) {
            return false;
        }
        // Stat the files outside the lock.
        let now = fingerprint(layout);
        self.0.lock().known.get(hash) == Some(&now)
    }

    pub fn set(&self, hash: String, layout: &Layout) {
        let now = fingerprint(layout);
        let mut g = self.0.lock();
        if g.known.get(&hash) != Some(&now) {
            g.known.insert(hash, now);
            g.dirty = true;
        }
    }

    pub fn forget(&self, hash: &str) {
        let mut g = self.0.lock();
        if g.known.remove(hash).is_some() {
            g.dirty = true;
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let (path, bytes) = {
            let mut g = self.0.lock();
            let Some(path) = g.path.clone().filter(|_| g.dirty) else {
                return Ok(());
            };
            g.dirty = false;
            (path, serde_json::to_vec(&g.known)?)
        };
        let tmp = path.with_extension("tmp");
        let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path));
        if written.is_err() {
            self.0.lock().dirty = true;
        }
        written
    }
}

fn read(path: &Path) -> Option<BTreeMap<String, String>> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Every file's path, size and modified time; files not there yet count too.
fn fingerprint(layout: &Layout) -> String {
    let mut hash = Sha256::new();
    for f in layout.files.iter().filter(|f| !f.padding) {
        hash.update(f.path.to_string_lossy().as_bytes());
        hash.update([0]);
        let seen = std::fs::metadata(&f.path).ok().and_then(|m| {
            let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
            Some((m.len(), modified.as_nanos()))
        });
        match seen {
            Some((len, modified)) => {
                hash.update([1]);
                hash.update(len.to_le_bytes());
                hash.update(modified.to_le_bytes());
            }
            None => hash.update([2]),
        }
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::FileSlot;
    use std::time::{Duration, SystemTime};

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    fn layout(paths: &[&Path]) -> Layout {
        Layout {
            root: PathBuf::new(),
            owns_root: false,
            files: paths
                .iter()
                .map(|p| FileSlot {
                    path: p.to_path_buf(),
                    offset: 0,
                    len: 4,
                    padding: false,
                })
                .collect(),
            piece_len: 4,
            total_len: 4,
        }
    }

    fn set_modified(path: &Path, when: SystemTime) {
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(when).unwrap();
    }

    #[test]
    fn any_change_to_a_file_breaks_the_match() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"aaaa").unwrap();
        let l = layout(&[&a, &b]);
        let s = Stamps::default();
        assert!(!s.matches(HASH, &l), "never stamped");
        s.set(HASH.into(), &l);
        assert!(s.matches(HASH, &l), "a file that isn't there yet is fine");

        // A file appears.
        std::fs::write(&b, b"bbbb").unwrap();
        assert!(!s.matches(HASH, &l));
        s.set(HASH.into(), &l);
        assert!(s.matches(HASH, &l));

        // Same size and contents, but touched.
        set_modified(&a, SystemTime::now() + Duration::from_secs(60));
        assert!(!s.matches(HASH, &l));
        s.set(HASH.into(), &l);

        // Size changes, with the time put back.
        let when = std::fs::metadata(&a).unwrap().modified().unwrap();
        std::fs::write(&a, b"aaaaa").unwrap();
        set_modified(&a, when);
        assert!(!s.matches(HASH, &l));
        s.set(HASH.into(), &l);

        // Another torrent's stamp says nothing about this one.
        assert!(!s.matches("ffffffffffffffffffffffffffffffffffffffff", &l));

        // The file is deleted, or the torrent lives somewhere else now.
        assert!(!s.matches(HASH, &layout(&[&b, &a])));
        std::fs::remove_file(&b).unwrap();
        assert!(!s.matches(HASH, &l));
    }

    #[test]
    fn stamps_survive_a_restart_and_damage_is_harmless() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::write(&a, b"aaaa").unwrap();
        let l = layout(&[&a]);
        let path = d.path().join("stamps.json");

        let s = Stamps::default();
        s.open(path.clone());
        s.set(HASH.into(), &l);
        s.save().unwrap();

        let s = Stamps::default();
        s.open(path.clone());
        assert!(s.matches(HASH, &l));
        s.forget(HASH);
        s.save().unwrap();
        let s = Stamps::default();
        s.open(path.clone());
        assert!(!s.matches(HASH, &l));

        for junk in [&b"not json"[..], b"[1,2]", b"{\"a\":1}"] {
            std::fs::write(&path, junk).unwrap();
            let s = Stamps::default();
            s.open(path.clone());
            assert!(!s.matches(HASH, &l));
        }
    }
}
