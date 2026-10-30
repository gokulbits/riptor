//! librqbit glue. Factories never see a torrent's output folder, so the engine
//! registers each root by info hash first; unregistered torrents are refused.

use std::collections::HashMap;
use std::io::IoSlice;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{ensure, Context};
use librqbit::storage::{BoxStorageFactory, StorageFactory, StorageFactoryExt, TorrentStorage};
use librqbit::{ManagedTorrentShared, TorrentMetadata};
use librqbit_core::hash_id::Id20;
use librqbit_core::lengths::ValidPieceIndex;
use parking_lot::RwLock;

use crate::layout::{Layout, RawFile};
use crate::store::{Shared, Store};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TorrentRoot {
    pub path: PathBuf,
    /// We created it, so it may be removed when empty.
    pub owned: bool,
    /// File index -> relative path (`/`-separated) chosen by the user; sanitized like any other.
    pub renames: std::collections::BTreeMap<usize, String>,
}

#[derive(Clone)]
pub struct CachedStorageFactory {
    shared: Arc<Shared>,
    roots: Arc<RwLock<HashMap<Id20, TorrentRoot>>>,
}

impl CachedStorageFactory {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            roots: Default::default(),
        }
    }

    pub fn register(&self, info_hash: Id20, root: TorrentRoot) {
        self.roots.write().insert(info_hash, root);
    }

    pub fn unregister(&self, info_hash: &Id20) {
        self.roots.write().remove(info_hash);
    }

    pub fn root_of(&self, info_hash: &Id20) -> Option<TorrentRoot> {
        self.roots.read().get(info_hash).cloned()
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn into_boxed(self) -> BoxStorageFactory {
        self.boxed()
    }
}

pub fn layout_for(root: &TorrentRoot, metadata: &TorrentMetadata) -> anyhow::Result<Layout> {
    let raw = metadata
        .info
        .iter_file_details()
        .enumerate()
        .map(|(i, fd)| RawFile {
            components: match root.renames.get(&i) {
                Some(r) => r
                    .split('/')
                    .map(|c| std::borrow::Cow::Owned(c.to_string()))
                    .collect(),
                None => fd.filename.iter_components().collect(),
            },
            len: fd.len,
            padding: fd.attrs().padding,
        });
    let layout = Layout::new(
        &root.path,
        root.owned,
        metadata.lengths().default_piece_length(),
        raw,
    )?;

    ensure!(
        layout.total_len == metadata.lengths().total_length(),
        "length mismatch with engine"
    );
    ensure!(
        layout.files.len() == metadata.file_infos.len(),
        "file count mismatch with engine"
    );
    for (ours, theirs) in layout.files.iter().zip(metadata.file_infos.iter()) {
        ensure!(
            ours.offset == theirs.offset_in_torrent && ours.len == theirs.len,
            "file offset mismatch with engine"
        );
    }
    Ok(layout)
}

impl StorageFactory for CachedStorageFactory {
    type Storage = CachedStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<CachedStorage> {
        let root = self.root_of(&shared.info_hash).with_context(|| {
            format!(
                "no save folder registered for {}",
                shared.info_hash.as_string()
            )
        })?;
        let layout = layout_for(&root, metadata)?;
        Ok(CachedStorage {
            store: Arc::new(Store::new(layout, self.shared.clone())),
            hash: shared.info_hash.as_string(),
        })
    }

    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

pub struct CachedStorage {
    store: Arc<Store>,
    hash: String,
}

impl CachedStorage {
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }
}

impl TorrentStorage for CachedStorage {
    fn init(
        &mut self,
        _shared: &ManagedTorrentShared,
        _metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        // Created on first write, so deselected files never appear.
        Ok(())
    }

    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.store.read(file_id, offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        self.store.write(file_id, offset, &[buf]).map(|_| ())
    }

    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        self.store.write(file_id, offset, &[&bufs[0], &bufs[1]])
    }

    fn remove_file(&self, file_id: usize, _untrusted_name: &Path) -> anyhow::Result<()> {
        self.store.remove_file(file_id)
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.store.remove_empty_dirs(path.as_os_str().is_empty())
    }

    fn ensure_file_length(&self, file_id: usize, length: u64) -> anyhow::Result<()> {
        self.store.ensure_len(file_id, length)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        self.store.release()?;
        Ok(Box::new(CachedStorage {
            store: self.store.clone(),
            hash: self.hash.clone(),
        }))
    }

    fn data_unchanged(&self) -> bool {
        self.store.unchanged(&self.hash)
    }

    fn on_piece_completed(&self, piece_index: ValidPieceIndex) -> anyhow::Result<()> {
        self.store.piece_completed(piece_index.get())
    }
}
