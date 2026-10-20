//! Moving a torrent's files. All or nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context};

/// Missing sources are skipped; existing targets are never overwritten.
pub fn move_files(pairs: &[(PathBuf, PathBuf)]) -> anyhow::Result<u64> {
    let mut seen = std::collections::HashSet::new();
    for (from, to) in pairs {
        ensure!(from != to, "source and destination are the same");
        ensure!(
            seen.insert(to.clone()),
            "two files would land on {}",
            to.display()
        );
        ensure!(
            !to.exists(),
            "{} already exists; Riptor won't overwrite it",
            to.display()
        );
    }

    let mut done: Vec<(&Path, &Path)> = Vec::new();
    let mut bytes = 0;
    for (from, to) in pairs {
        if !from.is_file() {
            continue;
        }
        if let Err(e) = move_one(from, to) {
            for (f, t) in done.iter().rev() {
                if let Err(back) = move_one(t, f) {
                    tracing::error!("couldn't put {} back: {back:#}", t.display());
                }
            }
            return Err(e.context(format!("moving {}", from.display())));
        }
        bytes += fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        done.push((from, to));
    }
    Ok(bytes)
}

fn move_one(from: &Path, to: &Path) -> anyhow::Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Another drive: copy, make sure it's on disk, then delete.
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            let copied = fs::copy(from, to).and_then(|_| fs::File::open(to)?.sync_all());
            if let Err(c) = copied {
                let _ = fs::remove_file(to);
                return Err(anyhow::Error::from(c)
                    .context(format!("rename failed ({e}), and so did copying")));
            }
            fs::remove_file(from).with_context(|| format!("removing {}", from.display()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Removes `dirs` that are empty, deepest first.
pub fn remove_empty_dirs(mut dirs: Vec<PathBuf>) {
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs {
        let _ = fs::remove_dir(&d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn moves_everything_and_skips_missing_files() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        write(&a.join("x/1.bin"), "one");
        write(&a.join("2.bin"), "two");
        let pairs = vec![
            (a.join("x/1.bin"), b.join("x/1.bin")),
            (a.join("2.bin"), b.join("2.bin")),
            (a.join("not-downloaded.bin"), b.join("not-downloaded.bin")),
        ];
        assert_eq!(move_files(&pairs).unwrap(), 6);
        assert_eq!(fs::read_to_string(b.join("x/1.bin")).unwrap(), "one");
        assert!(!a.join("2.bin").exists());
        assert!(!b.join("not-downloaded.bin").exists());
        remove_empty_dirs(vec![a.join("x"), a.clone()]);
        assert!(!a.exists());
    }

    #[test]
    fn never_overwrites_and_rolls_back() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        write(&a.join("1.bin"), "one");
        write(&a.join("2.bin"), "two");
        write(&b.join("2.bin"), "user's own file");
        let pairs = vec![
            (a.join("1.bin"), b.join("1.bin")),
            (a.join("2.bin"), b.join("2.bin")),
        ];
        assert!(move_files(&pairs).is_err());
        // Nothing moved, the existing file untouched.
        assert!(a.join("1.bin").exists() && !b.join("1.bin").exists());
        assert_eq!(
            fs::read_to_string(b.join("2.bin")).unwrap(),
            "user's own file"
        );

        // The second move fails, so the first is undone.
        fs::remove_file(b.join("2.bin")).unwrap();
        write(&b.join("blocker"), "file, not a folder");
        let pairs = vec![
            (a.join("1.bin"), b.join("1.bin")),
            (a.join("2.bin"), b.join("blocker/2.bin")),
        ];
        assert!(move_files(&pairs).is_err());
        assert!(a.join("1.bin").exists() && !b.join("1.bin").exists());
        assert!(a.join("2.bin").exists());
    }

    #[test]
    fn refuses_two_files_on_one_destination() {
        let d = tempfile::tempdir().unwrap();
        let pairs = vec![
            (d.path().join("1"), d.path().join("out")),
            (d.path().join("2"), d.path().join("out")),
        ];
        assert!(move_files(&pairs).is_err());
    }
}
