//! What "Open folder" shows for a torrent.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevealTarget {
    Folder(PathBuf),
    /// Shown selected in its folder.
    File(PathBuf),
}

/// Revealing a folder would open its parent, so folders are opened instead.
/// Never returns `Folder` for a file: opening it would run it.
pub fn reveal_target(root: &Path, files: &[PathBuf]) -> anyhow::Result<RevealTarget> {
    if let [single] = files {
        if single.is_file() {
            return Ok(RevealTarget::File(single.clone()));
        }
    }
    anyhow::ensure!(root.is_dir(), "the download folder does not exist yet");
    Ok(RevealTarget::Folder(root.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_file_torrent_opens_its_own_folder() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("Some Album");
        std::fs::create_dir(&root).unwrap();
        let files = [root.join("01.flac"), root.join("02.flac")];
        for f in &files {
            std::fs::write(f, b"x").unwrap();
        }
        assert_eq!(
            reveal_target(&root, &files).unwrap(),
            RevealTarget::Folder(root.clone())
        );
    }

    #[test]
    fn single_file_torrent_selects_the_file() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("debian.iso");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            reveal_target(d.path(), std::slice::from_ref(&file)).unwrap(),
            RevealTarget::File(file.clone())
        );
    }

    #[test]
    fn falls_back_to_the_folder_and_never_opens_a_file() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("not-yet.iso");
        assert_eq!(
            reveal_target(d.path(), std::slice::from_ref(&missing)).unwrap(),
            RevealTarget::Folder(d.path().to_path_buf())
        );
        let exe = d.path().join("evil.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(reveal_target(&exe, &[]).is_err());
        assert!(reveal_target(&d.path().join("gone"), &[]).is_err());
    }
}
