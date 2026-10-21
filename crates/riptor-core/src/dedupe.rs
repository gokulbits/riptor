//! Files a new torrent shares with ones already on disk, proven by its piece hashes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha1::{Digest, Sha1};

/// Smaller files aren't worth the reads.
pub const MIN_LEN: u64 = 1024 * 1024;
const SAMPLES: usize = 3;
const MAX_CANDIDATES_PER_FILE: usize = 4;
const MAX_FILES: usize = 2000;

/// One file of the new torrent.
#[derive(Clone, Debug)]
pub struct NewFile {
    pub index: usize,
    pub offset: u64,
    pub len: u64,
    pub padding: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Match {
    pub index: usize,
    /// The file already on disk.
    pub existing: PathBuf,
    /// No piece of the new torrent spans this file and another, so it can share the file safely.
    pub aligned: bool,
}

fn full_pieces(f: &NewFile, piece_len: u64) -> Option<(u64, u64)> {
    let first = f.offset.div_ceil(piece_len);
    let end = (f.offset + f.len) / piece_len;
    (end > first).then_some((first, end - 1))
}

fn same_data(path: &Path, f: &NewFile, piece_len: u64, hashes: &[u8]) -> bool {
    let Some((first, last)) = full_pieces(f, piece_len) else {
        return false;
    };
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut samples = vec![first, first + (last - first) / 2, last];
    samples.dedup();
    let mut buf = vec![0u8; piece_len as usize];
    samples.into_iter().take(SAMPLES).all(|p| {
        let Some(expected) = hashes.get(p as usize * 20..p as usize * 20 + 20) else {
            return false;
        };
        file.seek(SeekFrom::Start(p * piece_len - f.offset)).is_ok()
            && file.read_exact(&mut buf).is_ok()
            && Sha1::digest(&buf).as_slice() == expected
    })
}

/// Whether the file's bytes start and end on piece boundaries (or next to padding / the end).
fn aligned(files: &[NewFile], i: usize, piece_len: u64, total: u64) -> bool {
    let f = &files[i];
    let end = f.offset + f.len;
    let starts =
        f.offset.is_multiple_of(piece_len) || i.checked_sub(1).is_some_and(|p| files[p].padding);
    let ends = end.is_multiple_of(piece_len)
        || end == total
        || files.get(i + 1).is_some_and(|n| n.padding);
    starts && ends
}

/// `candidates`: existing complete files by length. Reads a few pieces per candidate.
pub fn find(
    files: &[NewFile],
    piece_len: u64,
    hashes: &[u8],
    candidates: &HashMap<u64, Vec<PathBuf>>,
) -> Vec<Match> {
    let total = files.last().map_or(0, |f| f.offset + f.len);
    let mut out = Vec::new();
    for (i, f) in files.iter().enumerate().take(MAX_FILES) {
        if f.padding || f.len < MIN_LEN {
            continue;
        }
        let Some(paths) = candidates.get(&f.len) else {
            continue;
        };
        if let Some(p) = paths
            .iter()
            .take(MAX_CANDIDATES_PER_FILE)
            .find(|p| same_data(p, f, piece_len, hashes))
        {
            out.push(Match {
                index: f.index,
                existing: p.clone(),
                aligned: aligned(files, i, piece_len, total),
            });
        }
    }
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Placed {
    pub linked: u64,
    pub copied: u64,
}

/// Links (when aligned) or copies each match into place; never overwrites.
pub fn place(matches: &[(PathBuf, &Match)], copy_budget: u64) -> Placed {
    let mut placed = Placed::default();
    let mut budget = copy_budget;
    for (target, m) in matches {
        if target.exists() {
            continue;
        }
        let Some(dir) = target.parent() else {
            continue;
        };
        if std::fs::create_dir_all(dir).is_err() {
            continue;
        }
        let len = std::fs::metadata(&m.existing).map_or(0, |md| md.len());
        if m.aligned && std::fs::hard_link(&m.existing, target).is_ok() {
            placed.linked += len;
        } else if len <= budget && std::fs::copy(&m.existing, target).is_ok() {
            budget -= len;
            placed.copied += len;
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    const PL: u64 = 64 * 1024;

    fn hashes_of(data: &[u8]) -> Vec<u8> {
        data.chunks(PL as usize)
            .flat_map(|c| Sha1::digest(c).to_vec())
            .collect()
    }

    fn data(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn identical_files_are_found_and_lookalikes_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let a = data(MIN_LEN as usize + 100_000, 1);
        let b = data(MIN_LEN as usize * 2, 2);
        // The new torrent: a small file, then a, then b; pieces straddle files.
        let small = data(1000, 3);
        let mut all = small.clone();
        all.extend(&a);
        all.extend(&b);
        let hashes = hashes_of(&all);
        let files = vec![
            NewFile {
                index: 0,
                offset: 0,
                len: 1000,
                padding: false,
            },
            NewFile {
                index: 1,
                offset: 1000,
                len: a.len() as u64,
                padding: false,
            },
            NewFile {
                index: 2,
                offset: 1000 + a.len() as u64,
                len: b.len() as u64,
                padding: false,
            },
        ];
        let have_a = dir.path().join("have_a.bin");
        std::fs::write(&have_a, &a).unwrap();
        // Same size as b, different bytes.
        let fake_b = dir.path().join("fake_b.bin");
        std::fs::write(&fake_b, data(b.len(), 9)).unwrap();
        let real_b = dir.path().join("real_b.bin");
        std::fs::write(&real_b, &b).unwrap();
        let mut c: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        c.insert(a.len() as u64, vec![have_a.clone()]);
        c.insert(b.len() as u64, vec![fake_b, real_b.clone()]);
        let m = find(&files, PL, &hashes, &c);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].index, &m[0].existing), (1, &have_a));
        assert_eq!((m[1].index, &m[1].existing), (2, &real_b));
        // Neither starts on a piece boundary.
        assert!(!m[0].aligned && !m[1].aligned);
    }

    #[test]
    fn alignment_follows_piece_boundaries_and_padding() {
        let f = |offset, len, padding| NewFile {
            index: 0,
            offset,
            len,
            padding,
        };
        let files = vec![
            f(0, PL * 3, false),
            f(PL * 3, 100, true),
            f(PL * 3 + 100, PL - 100, false),
            f(PL * 4, 5, false),
        ];
        assert!(aligned(&files, 0, PL, PL * 4 + 5));
        // Starts after padding, ends on a boundary.
        assert!(aligned(&files, 2, PL, PL * 4 + 5));
        assert!(aligned(&files, 3, PL, PL * 4 + 5));
        let files = vec![f(0, 10, false), f(10, PL * 2, false)];
        assert!(!aligned(&files, 1, PL, PL * 2 + 10));
    }

    #[test]
    fn placing_links_or_copies_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        std::fs::write(&src, b"payload").unwrap();
        let m_aligned = Match {
            index: 0,
            existing: src.clone(),
            aligned: true,
        };
        let m_loose = Match {
            index: 1,
            existing: src.clone(),
            aligned: false,
        };
        let t1 = dir.path().join("new/a/linked.bin");
        let t2 = dir.path().join("new/b/copied.bin");
        let t3 = dir.path().join("new/kept.bin");
        std::fs::create_dir_all(t3.parent().unwrap()).unwrap();
        std::fs::write(&t3, b"mine").unwrap();
        let p = place(
            &[
                (t1.clone(), &m_aligned),
                (t2.clone(), &m_loose),
                (t3.clone(), &m_aligned),
            ],
            u64::MAX,
        );
        assert_eq!(
            p,
            Placed {
                linked: 7,
                copied: 7
            }
        );
        assert_eq!(std::fs::read(&t1).unwrap(), b"payload");
        assert_eq!(std::fs::read(&t2).unwrap(), b"payload");
        assert_eq!(std::fs::read(&t3).unwrap(), b"mine");
        // A copy changes nothing in the original; no space left means no copy.
        std::fs::write(&t2, b"changed").unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), b"payload");
        let t4 = dir.path().join("new/c/none.bin");
        assert_eq!(place(&[(t4.clone(), &m_loose)], 3), Placed::default());
        assert!(!t4.exists());
    }

    #[test]
    fn small_and_unverifiable_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let tiny = dir.path().join("tiny");
        std::fs::write(&tiny, b"x").unwrap();
        let files = vec![NewFile {
            index: 0,
            offset: 0,
            len: 1,
            padding: false,
        }];
        let mut c = HashMap::new();
        c.insert(1u64, vec![tiny]);
        assert!(find(&files, PL, &hashes_of(b"x"), &c).is_empty());
        // Missing candidates and short hash lists don't panic.
        let big = NewFile {
            index: 0,
            offset: 0,
            len: MIN_LEN * 2,
            padding: false,
        };
        let mut c = HashMap::new();
        c.insert(MIN_LEN * 2, vec![dir.path().join("gone")]);
        assert!(find(&[big], PL, &[0u8; 20], &c).is_empty());
    }
}
