//! Riptor's storage layer for librqbit.
//!
//! * **Hardened paths** ([`sanitize`]): every on-disk path is rebuilt from the
//!   raw torrent metadata and proven to stay under the save folder.
//! * **Write-back piece buffer**: downloaded blocks are held per piece, hash
//!   checks read from memory, and verified pieces are flushed as large writes.
//! * **Seeding read cache**: extent-sized LRU shared by all torrents.
//! * **Handle pool**: lazily opened files with an LRU cap.
//! * **Preallocation**: sparse (default) or full, as in qBittorrent.
#![deny(unsafe_code)]

pub mod cache;
pub mod files;
pub mod layout;
mod os;
pub mod rqbit;
pub mod sanitize;
pub mod segments;
pub mod store;

use serde::{Deserialize, Serialize};

pub use rqbit::{layout_for, CachedStorage, CachedStorageFactory, TorrentRoot};
pub use store::{Shared, Store};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preallocation {
    #[default]
    Sparse,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Seeding read cache, shared by all torrents. 0 disables it.
    pub read_cache_bytes: usize,
    /// Downloaded-but-unflushed data, shared by all torrents. 0 writes straight through.
    pub write_buffer_bytes: usize,
    pub max_open_files: usize,
    pub preallocation: Preallocation,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            read_cache_bytes: 128 * 1024 * 1024,
            write_buffer_bytes: 64 * 1024 * 1024,
            max_open_files: 512,
            preallocation: Preallocation::Sparse,
        }
    }
}
