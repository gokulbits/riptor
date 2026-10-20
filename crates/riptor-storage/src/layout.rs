//! Flat torrent offsets to pieces and files.

use std::path::{Path, PathBuf};

use anyhow::ensure;

use crate::sanitize::{dedupe, safe_join, sanitize_relative};

#[derive(Debug, Clone)]
pub struct FileSlot {
    pub path: PathBuf,
    pub offset: u64,
    pub len: u64,
    /// BEP 47; never written.
    pub padding: bool,
}

pub struct RawFile<'a> {
    pub components: Vec<std::borrow::Cow<'a, str>>,
    pub len: u64,
    pub padding: bool,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    /// We created `root`, so it may be removed when empty.
    pub owns_root: bool,
    pub files: Vec<FileSlot>,
    pub piece_len: u64,
    pub total_len: u64,
}

/// A contiguous span inside one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file_id: usize,
    pub file_offset: u64,
    pub len: u64,
    /// Relative to the requested range.
    pub rel: u64,
}

impl Layout {
    pub fn new<'a>(
        root: &Path,
        owns_root: bool,
        piece_len: u32,
        raw: impl IntoIterator<Item = RawFile<'a>>,
    ) -> anyhow::Result<Self> {
        ensure!(piece_len > 0, "piece length must be positive");
        let mut rels = Vec::new();
        let mut meta = Vec::new();
        for f in raw {
            rels.push(sanitize_relative(f.components.iter().map(|c| c.as_ref()))?);
            meta.push((f.len, f.padding));
        }
        ensure!(!rels.is_empty(), "torrent has no files");
        dedupe(&mut rels);

        let mut offset = 0u64;
        let mut files = Vec::with_capacity(rels.len());
        for (rel, (len, padding)) in rels.into_iter().zip(meta) {
            files.push(FileSlot {
                path: safe_join(root, &rel)?,
                offset,
                len,
                padding,
            });
            offset = offset
                .checked_add(len)
                .ok_or_else(|| anyhow::anyhow!("torrent too large"))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            owns_root,
            files,
            piece_len: piece_len as u64,
            total_len: offset,
        })
    }

    pub fn piece_of(&self, abs: u64) -> u32 {
        (abs / self.piece_len) as u32
    }

    pub fn piece_bounds(&self, piece: u32) -> (u64, u64) {
        let start = piece as u64 * self.piece_len;
        (
            start,
            self.piece_len.min(self.total_len.saturating_sub(start)),
        )
    }

    pub fn spans(&self, abs: u64, len: u64) -> impl Iterator<Item = Span> + '_ {
        let end = abs + len;
        let first = self.files.partition_point(|f| f.offset + f.len <= abs);
        self.files[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, f)| f.offset < end)
            .filter(|(_, f)| f.len > 0)
            .map(move |(i, f)| {
                let s = abs.max(f.offset);
                let e = end.min(f.offset + f.len);
                Span {
                    file_id: first + i,
                    file_offset: s - f.offset,
                    len: e - s,
                    rel: s - abs,
                }
            })
    }

    /// Deepest first.
    pub fn subdirs_deepest_first(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for f in &self.files {
            let mut p = f.path.parent();
            while let Some(d) = p {
                if d == self.root || !d.starts_with(&self.root) {
                    break;
                }
                dirs.push(d.to_path_buf());
                p = d.parent();
            }
        }
        dirs.sort();
        dirs.dedup();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        dirs
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn raw(names: &[(&str, u64)]) -> Vec<RawFile<'static>> {
        names
            .iter()
            .map(|(n, len)| RawFile {
                components: n
                    .split('/')
                    .map(|s| std::borrow::Cow::Owned(s.to_string()))
                    .collect(),
                len: *len,
                padding: n.starts_with(".pad"),
            })
            .collect()
    }

    pub fn root() -> PathBuf {
        std::env::temp_dir().join("riptor-layout-test")
    }

    #[test]
    fn spans_cross_files() {
        let l = Layout::new(
            &root(),
            true,
            10,
            raw(&[("a", 5), ("empty", 0), ("b", 10), ("c", 7)]),
        )
        .unwrap();
        assert_eq!(l.total_len, 22);
        let spans: Vec<_> = l.spans(3, 15).collect();
        assert_eq!(
            spans,
            vec![
                Span {
                    file_id: 0,
                    file_offset: 3,
                    len: 2,
                    rel: 0
                },
                Span {
                    file_id: 2,
                    file_offset: 0,
                    len: 10,
                    rel: 2
                },
                Span {
                    file_id: 3,
                    file_offset: 0,
                    len: 3,
                    rel: 12
                },
            ]
        );
        assert_eq!(l.piece_bounds(2), (20, 2));
        assert_eq!(l.piece_of(19), 1);
    }

    #[test]
    fn paths_are_sanitized_and_unique() {
        let l = Layout::new(
            &root(),
            true,
            16,
            raw(&[("d/C:x", 1), ("d/C_x", 1), ("D/c_X", 1)]),
        )
        .unwrap();
        let names: Vec<_> = l
            .files
            .iter()
            .map(|f| f.path.strip_prefix(root()).unwrap().to_path_buf())
            .collect();
        assert_eq!(names[0], Path::new("d").join("C_x"));
        assert_eq!(names[1], Path::new("d").join("C_x (1)"));
        assert_eq!(names[2], Path::new("D").join("c_X (2)"));
        assert!(l.files.iter().all(|f| f.path.starts_with(root())));
    }

    #[test]
    fn subdirs_exclude_root() {
        let l = Layout::new(
            &root(),
            false,
            16,
            raw(&[("a/b/c.txt", 1), ("a/d.txt", 1), ("top.txt", 1)]),
        )
        .unwrap();
        assert_eq!(
            l.subdirs_deepest_first(),
            vec![root().join("a").join("b"), root().join("a")]
        );
    }
}
