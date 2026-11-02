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
pub mod stamps;
pub mod store;

use serde::{Deserialize, Serialize};

pub use os::{free_space, total_memory};
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

const MIB: usize = 1024 * 1024;

/// `percent` of physical memory, kept inside `min..=max`. Without a reading of
/// the machine's memory it stays at `max`, which is where these used to sit.
fn share_of_memory(percent: u64, min: usize, max: usize) -> usize {
    total_memory()
        .and_then(|total| usize::try_from(total / 100 * percent).ok())
        .unwrap_or(max)
        .clamp(min, max)
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            read_cache_bytes: share_of_memory(2, 16 * MIB, 128 * MIB),
            write_buffer_bytes: share_of_memory(1, 8 * MIB, 64 * MIB),
            max_open_files: 512,
            preallocation: Preallocation::Sparse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_defaults_stay_inside_their_bounds() {
        let c = StorageConfig::default();
        assert!((16 * MIB..=128 * MIB).contains(&c.read_cache_bytes));
        assert!((8 * MIB..=64 * MIB).contains(&c.write_buffer_bytes));
        // A machine we can measure gets less than the old fixed sizes unless it
        // has the memory for them.
        if let Some(total) = total_memory() {
            assert!(c.read_cache_bytes as u64 <= (total / 100 * 2).max(16 * MIB as u64));
        }
    }

    #[test]
    fn memory_reading_is_plausible() {
        if let Some(total) = total_memory() {
            assert!(total > 256 * MIB as u64, "{total} bytes of RAM?");
            assert!(total < 64 * 1024 * MIB as u64);
        }
    }
}
