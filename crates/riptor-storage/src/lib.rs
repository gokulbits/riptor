//! Riptor's storage layer for librqbit: sanitized paths, a write-back piece
//! buffer, a seeding read cache and a handle pool.
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

pub use os::free_space;
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
    /// 0 disables it.
    pub read_cache_bytes: usize,
    /// 0 writes straight through.
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
