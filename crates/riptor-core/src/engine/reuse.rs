//! Reusing files other torrents already downloaded (see `crate::dedupe`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use librqbit_core::hash_id::Id20;
use librqbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info;
use riptor_storage::layout::{Layout, RawFile};
use riptor_storage::layout_for;
use tracing::{info, warn};

use super::Engine;
use crate::dedupe::{self, Match, NewFile};

const MAX_CANDIDATES: usize = 20_000;

/// What `add` needs to put found files in place: the matches and the file list they index.
pub(super) struct ReusePlan {
    pub matches: Vec<Match>,
    pub piece_len: u32,
    /// (path components, length, padding) per file.
    pub files: Vec<(Vec<String>, u64, bool)>,
}

impl ReusePlan {
    pub(super) fn new<B: AsRef<[u8]>>(
        matches: Vec<Match>,
        info: &ValidatedTorrentMetaV1Info<B>,
    ) -> Self {
        Self {
            matches,
            piece_len: info.lengths().default_piece_length(),
            files: info
                .iter_file_details()
                .map(|fd| {
                    (
                        fd.filename
                            .iter_components()
                            .map(|c| c.into_owned())
                            .collect(),
                        fd.len,
                        fd.attrs().padding,
                    )
                })
                .collect(),
        }
    }
}

fn new_files<B: AsRef<[u8]>>(info: &ValidatedTorrentMetaV1Info<B>) -> Vec<NewFile> {
    let mut offset = 0;
    info.iter_file_details()
        .enumerate()
        .map(|(index, fd)| {
            let f = NewFile {
                index,
                offset,
                len: fd.len,
                padding: fd.attrs().padding,
            };
            offset += fd.len;
            f
        })
        .collect()
}

impl Engine {
    /// Finished, wanted files of every other torrent, by length.
    fn existing_files(&self, except: &Id20) -> HashMap<u64, Vec<PathBuf>> {
        let hashes: Vec<Id20> = self
            .records
            .read()
            .iter()
            .filter(|(h, r)| {
                *h != except && r.completed_on.is_some() && r.move_on_complete.is_none()
            })
            .map(|(h, _)| *h)
            .collect();
        let mut out: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        let mut n = 0;
        for h in hashes {
            let (Some(root), Ok(t)) = (self.factory.root_of(&h), self.handle(&h)) else {
                continue;
            };
            let only = t.only_files();
            let Ok(Ok(layout)) = t.with_metadata(|m| layout_for(&root, m)) else {
                continue;
            };
            for (i, f) in layout.files.iter().enumerate() {
                if f.padding
                    || f.len < dedupe::MIN_LEN
                    || only.as_ref().is_some_and(|o| !o.contains(&i))
                {
                    continue;
                }
                out.entry(f.len).or_default().push(f.path.clone());
                n += 1;
                if n >= MAX_CANDIDATES {
                    return out;
                }
            }
        }
        out
    }

    /// Files of a torrent being added that are already on disk.
    pub(super) async fn find_duplicates<B: AsRef<[u8]>>(
        &self,
        hash: &Id20,
        info: &ValidatedTorrentMetaV1Info<B>,
    ) -> Vec<Match> {
        let candidates = self.existing_files(hash);
        if candidates.is_empty() {
            return Vec::new();
        }
        let files = new_files(info);
        let piece_len = u64::from(info.lengths().default_piece_length());
        let hashes = info.info().pieces.as_ref().to_vec();
        tokio::task::spawn_blocking(move || dedupe::find(&files, piece_len, &hashes, &candidates))
            .await
            .unwrap_or_default()
    }

    /// Links or copies the matches into the new torrent's folder before it's added.
    pub(super) async fn place_duplicates(
        &self,
        plan: &ReusePlan,
        root: &Path,
        owned: bool,
        only_files: Option<&[usize]>,
    ) {
        let raw = plan.files.iter().map(|(parts, len, padding)| RawFile {
            components: parts
                .iter()
                .map(|p| std::borrow::Cow::Borrowed(p.as_str()))
                .collect(),
            len: *len,
            padding: *padding,
        });
        let matches = &plan.matches;
        let layout = match Layout::new(root, owned, plan.piece_len, raw) {
            Ok(l) => l,
            Err(e) => {
                warn!("reusing files: {e:#}");
                return;
            }
        };
        let targets: Vec<(PathBuf, Match)> = matches
            .iter()
            .filter(|m| only_files.is_none_or(|o| o.contains(&m.index)))
            .filter_map(|m| Some((layout.files.get(m.index)?.path.clone(), m.clone())))
            .collect();
        // Copies leave the free-space margin alone.
        let margin = self.settings.read().min_free_mib.unwrap_or(0) * 1024 * 1024;
        let budget = self
            .free_space(root)
            .map_or(0, |free| free.saturating_sub(margin));
        let placed = tokio::task::spawn_blocking(move || {
            let refs: Vec<(PathBuf, &Match)> =
                targets.iter().map(|(t, m)| (t.clone(), m)).collect();
            dedupe::place(&refs, budget)
        })
        .await
        .unwrap_or_default();
        if placed.linked + placed.copied > 0 {
            info!(
                "reused {} MiB already on disk ({} MiB shared, {} MiB copied)",
                (placed.linked + placed.copied) >> 20,
                placed.linked >> 20,
                placed.copied >> 20
            );
        }
    }
}
