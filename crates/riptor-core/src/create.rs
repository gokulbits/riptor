//! Making a .torrent from a file or folder.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{bail, ensure, Context};
use serde::Deserialize;
use sha1::{Digest, Sha1};

const MAX_FILES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_URLS: usize = 64;
const MAX_URL_LEN: usize = 2048;
const MAX_COMMENT: usize = 4096;
const MIN_PIECE: u32 = 16 * 1024;
const MAX_PIECE: u32 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CreateOptions {
    /// One tier each, in order.
    pub trackers: Vec<String>,
    pub web_seeds: Vec<String>,
    pub private: bool,
    pub comment: Option<String>,
    /// `None` picks one from the total size.
    pub piece_length: Option<u32>,
}

impl CreateOptions {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.trackers.len() <= MAX_URLS && self.web_seeds.len() <= MAX_URLS,
            "too many trackers or web seeds"
        );
        for t in &self.trackers {
            check_url(t, &["http", "https", "udp"])?;
        }
        for w in &self.web_seeds {
            check_url(w, &["http", "https"])?;
        }
        if let Some(c) = &self.comment {
            ensure!(c.len() <= MAX_COMMENT, "comment is too long");
        }
        if let Some(p) = self.piece_length {
            ensure!(
                p.is_power_of_two() && (MIN_PIECE..=MAX_PIECE).contains(&p),
                "piece size must be a power of two from 16 KiB to 64 MiB"
            );
        }
        Ok(())
    }
}

fn check_url(u: &str, schemes: &[&str]) -> anyhow::Result<()> {
    ensure!(u.len() <= MAX_URL_LEN, "URL is too long");
    let parsed = url::Url::parse(u).with_context(|| format!("not a URL: {u}"))?;
    ensure!(
        schemes.contains(&parsed.scheme()) && parsed.host().is_some(),
        "unsupported URL: {u}"
    );
    Ok(())
}

/// Shared with the UI while hashing.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

struct Entry {
    path: PathBuf,
    /// Relative path parts, UTF-8.
    parts: Vec<String>,
    len: u64,
}

fn name_of(p: &Path) -> anyhow::Result<String> {
    let n = p
        .file_name()
        .with_context(|| format!("{} has no name", p.display()))?;
    n.to_str()
        .map(str::to_string)
        .with_context(|| format!("{} isn't a valid UTF-8 name", p.display()))
}

fn walk(dir: &Path, parts: &mut Vec<String>, out: &mut Vec<Entry>) -> anyhow::Result<()> {
    ensure!(parts.len() < MAX_DEPTH, "folders are nested too deeply");
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let meta = e.metadata()?; // not following symlinks
        if meta.file_type().is_symlink() {
            continue;
        }
        parts.push(name_of(&e.path())?);
        if meta.is_dir() {
            walk(&e.path(), parts, out)?;
        } else if meta.is_file() {
            ensure!(out.len() < MAX_FILES, "too many files (over {MAX_FILES})");
            out.push(Entry {
                path: e.path(),
                parts: parts.clone(),
                len: meta.len(),
            });
        }
        parts.pop();
    }
    Ok(())
}

/// About 1000-2000 pieces, within the allowed range.
pub fn auto_piece_length(total: u64) -> u32 {
    let mut p = MIN_PIECE;
    while p < 16 * 1024 * 1024 && total / p as u64 > 2000 {
        p *= 2;
    }
    p
}

enum B {
    Int(i64),
    Str(Vec<u8>),
    List(Vec<B>),
    Dict(BTreeMap<&'static [u8], B>),
}

impl B {
    fn str(s: impl AsRef<[u8]>) -> Self {
        B::Str(s.as_ref().to_vec())
    }

    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            B::Int(i) => out.extend_from_slice(format!("i{i}e").as_bytes()),
            B::Str(s) => {
                out.extend_from_slice(format!("{}:", s.len()).as_bytes());
                out.extend_from_slice(s);
            }
            B::List(l) => {
                out.push(b'l');
                l.iter().for_each(|v| v.encode(out));
                out.push(b'e');
            }
            B::Dict(d) => {
                out.push(b'd');
                for (k, v) in d {
                    B::str(k).encode(out);
                    v.encode(out);
                }
                out.push(b'e');
            }
        }
    }
}

/// Returns the .torrent's bytes.
pub fn create(source: &Path, o: &CreateOptions, progress: &Progress) -> anyhow::Result<Vec<u8>> {
    o.validate()?;
    let meta = std::fs::symlink_metadata(source)
        .with_context(|| format!("reading {}", source.display()))?;
    let name = name_of(source)?;
    let single = meta.is_file();
    let files = if single {
        vec![Entry {
            path: source.to_path_buf(),
            parts: vec![name.clone()],
            len: meta.len(),
        }]
    } else if meta.is_dir() {
        let mut out = Vec::new();
        walk(source, &mut Vec::new(), &mut out)?;
        out
    } else {
        bail!("{} is neither a file nor a folder", source.display());
    };
    let total: u64 = files.iter().map(|f| f.len).sum();
    ensure!(total > 0, "there's nothing to share: all files are empty");
    let piece = o.piece_length.unwrap_or_else(|| auto_piece_length(total));
    progress.total.store(total, Ordering::Relaxed);

    let mut pieces = Vec::new();
    let mut hasher = Sha1::new();
    let mut in_piece = 0u32;
    let mut buf = vec![0u8; 1 << 20];
    for f in &files {
        let mut file = std::fs::File::open(&f.path)
            .with_context(|| format!("opening {}", f.path.display()))?;
        let mut left = f.len;
        while left > 0 {
            ensure!(!progress.cancel.load(Ordering::Relaxed), "cancelled");
            let want = (buf.len() as u64).min(left) as usize;
            let n = file.read(&mut buf[..want])?;
            ensure!(
                n > 0,
                "{} changed while it was being read",
                f.path.display()
            );
            left -= n as u64;
            let mut chunk = &buf[..n];
            while !chunk.is_empty() {
                let take = chunk.len().min((piece - in_piece) as usize);
                hasher.update(&chunk[..take]);
                in_piece += take as u32;
                chunk = &chunk[take..];
                if in_piece == piece {
                    pieces.extend_from_slice(&hasher.finalize_reset());
                    in_piece = 0;
                }
            }
            progress.done.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    if in_piece > 0 {
        pieces.extend_from_slice(&hasher.finalize());
    }

    let mut info = BTreeMap::new();
    info.insert(&b"name"[..], B::str(&name));
    info.insert(&b"piece length"[..], B::Int(piece as i64));
    info.insert(&b"pieces"[..], B::Str(pieces));
    if o.private {
        info.insert(&b"private"[..], B::Int(1));
    }
    if single {
        info.insert(&b"length"[..], B::Int(total as i64));
    } else {
        let list = files
            .iter()
            .map(|f| {
                let mut d = BTreeMap::new();
                d.insert(&b"length"[..], B::Int(f.len as i64));
                d.insert(&b"path"[..], B::List(f.parts.iter().map(B::str).collect()));
                B::Dict(d)
            })
            .collect();
        info.insert(&b"files"[..], B::List(list));
    }

    let mut top = BTreeMap::new();
    if let Some(first) = o.trackers.first() {
        top.insert(&b"announce"[..], B::str(first));
        top.insert(
            &b"announce-list"[..],
            B::List(
                o.trackers
                    .iter()
                    .map(|t| B::List(vec![B::str(t)]))
                    .collect(),
            ),
        );
    }
    if let Some(c) = o.comment.as_deref().filter(|c| !c.is_empty()) {
        top.insert(&b"comment"[..], B::str(c));
    }
    top.insert(
        &b"created by"[..],
        B::str(concat!("Riptor ", env!("CARGO_PKG_VERSION"))),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    top.insert(&b"creation date"[..], B::Int(now));
    top.insert(&b"encoding"[..], B::str("UTF-8"));
    if !o.web_seeds.is_empty() {
        top.insert(
            &b"url-list"[..],
            B::List(o.web_seeds.iter().map(B::str).collect()),
        );
    }
    top.insert(&b"info"[..], B::Dict(info));
    let mut out = Vec::new();
    B::Dict(top).encode(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librqbit_core::torrent_metainfo::torrent_from_bytes;

    #[test]
    fn folder_torrent_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("album");
        std::fs::create_dir_all(src.join("disc 2")).unwrap();
        std::fs::write(src.join("b.txt"), vec![7u8; 40_000]).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("disc 2").join("c.bin"), vec![1u8; 70_000]).unwrap();
        std::fs::write(src.join("empty"), b"").unwrap();
        let opts = CreateOptions {
            trackers: vec![
                "udp://tracker.example:1337/announce".into(),
                "https://t.example/a".into(),
            ],
            web_seeds: vec!["https://files.example/".into()],
            private: true,
            comment: Some("test".into()),
            piece_length: Some(32 * 1024),
        };
        let p = Progress::default();
        let bytes = create(&src, &opts, &p).unwrap();
        assert_eq!(p.done.load(Ordering::Relaxed), 110_005);
        let t = torrent_from_bytes(&bytes).unwrap();
        let info = &t.info.data;
        assert_eq!(info.piece_length, 32 * 1024);
        assert!(info.private);
        let names: Vec<String> = info
            .files
            .as_ref()
            .unwrap()
            .iter()
            .map(|f| {
                f.path
                    .iter()
                    .map(|c| String::from_utf8_lossy(c.as_ref()).into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        assert_eq!(names, ["a.txt", "b.txt", "disc 2/c.bin", "empty"]);
        assert_eq!(
            info.pieces.as_ref().len() / 20,
            110_005usize.div_ceil(32 * 1024)
        );
        assert_eq!(crate::webseed::url_list(&bytes).len(), 1);
    }

    #[test]
    fn single_file_torrent() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("file.bin");
        std::fs::write(&f, (0..300_000u32).map(|i| i as u8).collect::<Vec<_>>()).unwrap();
        let ours = create(
            &f,
            &CreateOptions {
                piece_length: Some(64 * 1024),
                ..Default::default()
            },
            &Progress::default(),
        )
        .unwrap();
        let ours = torrent_from_bytes(&ours).unwrap();
        assert_eq!(ours.info.data.length, Some(300_000));
        assert_eq!(ours.info.data.pieces.as_ref().len(), 5 * 20);
    }

    #[test]
    fn bad_options_and_cancel() {
        for bad in [
            CreateOptions {
                trackers: vec!["file:///etc/passwd".into()],
                ..Default::default()
            },
            CreateOptions {
                web_seeds: vec!["udp://x:1".into()],
                ..Default::default()
            },
            CreateOptions {
                piece_length: Some(1000),
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x");
        std::fs::write(&f, b"data").unwrap();
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(create(&f, &CreateOptions::default(), &p).is_err());
        std::fs::write(&f, b"").unwrap();
        assert!(create(&f, &CreateOptions::default(), &Progress::default()).is_err());
    }

    #[test]
    fn piece_size_scales() {
        assert_eq!(auto_piece_length(1), MIN_PIECE);
        assert_eq!(auto_piece_length(4 << 30), 4 << 20);
        assert!(auto_piece_length(u64::MAX) <= 16 << 20);
    }
}
