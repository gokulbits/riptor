//! Riptor's torrent engine. Wraps a librqbit session with hardened storage,
//! persistent statistics and qBittorrent-style seeding/queueing.
#![forbid(unsafe_code)]

pub mod backup;
mod engine;
pub mod input;
pub mod model;
mod motw;
pub mod network;
pub mod paths;
pub mod settings;
pub mod state_file;
pub mod stats;

pub use engine::{BackupSummary, Engine, EngineConfig, QueueMove, RestoreOptions, RestoreReport};
pub use input::Source;
pub use model::*;
pub use network::{NetworkMode, VpnBinding};
pub use riptor_seeding::{Intent, Limit, ShareLimitAction, ShareLimits};
pub use settings::Settings;
pub use stats::{Statistics, StatsReport};
