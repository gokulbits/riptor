//! Crash-safe, owner-only JSON files (temp file, fsync, rename).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use serde::{de::DeserializeOwned, Serialize};

pub fn load<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {path:?}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("reading {path:?}")),
    }
}

/// Saves share a `.tmp` name per file, so they take turns.
static SAVING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub fn save_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let _turn = SAVING.lock();
    let dir = path.parent().context("file has no parent folder")?;
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = create_private(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {path:?}"))?;
    Ok(())
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    save_bytes(path, &bytes)
}

fn create_private(path: &Path) -> std::io::Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x").join("state.json");
        let v: Vec<u32> = load(&p).unwrap();
        assert!(v.is_empty());
        save(&p, &vec![1u32, 2, 3]).unwrap();
        assert_eq!(load::<Vec<u32>>(&p).unwrap(), vec![1, 2, 3]);
        assert!(!p.with_extension("tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
