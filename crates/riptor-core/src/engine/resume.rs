//! Coming back from a restart: not re-reading unchanged files, and not losing torrents.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{ensure, Context};
use librqbit::api::TorrentIdOrHash;
use librqbit::{AddTorrent, AddTorrentOptions, Session, TorrentStatsState};
use librqbit_core::hash_id::Id20;
use riptor_storage::{layout_for, TorrentRoot};
use tracing::{info, warn};

use super::Engine;
use crate::input;
use crate::model::Record;
use crate::session_db::{self, Entry};

impl Engine {
    /// Remembers what torrents' files look like, so the next start needn't read them.
    pub(super) async fn stamp_files(&self, session: &Session, finished_only: bool) {
        let handles = session.with_torrents(|it| it.map(|(_, h)| h.clone()).collect::<Vec<_>>());
        let mut layouts = Vec::new();
        for h in handles {
            let hash = h.info_hash();
            let stats = h.stats();
            match stats.state {
                // Its old stamp still describes what it last proved.
                TorrentStatsState::Initializing { .. } => continue,
                TorrentStatsState::Error => {
                    self.storage.stamps.forget(&hash.as_string());
                    continue;
                }
                _ if finished_only && !stats.finished => continue,
                _ => {}
            }
            let Some(root) = self.factory.root_of(&hash) else {
                continue;
            };
            if let Ok(Ok(layout)) = h.with_metadata(|m| layout_for(&root, m)) {
                layouts.push((hash.as_string(), layout));
            }
        }
        let storage = self.storage.clone();
        let saved = tokio::task::spawn_blocking(move || {
            for (hash, layout) in layouts {
                storage.stamps.set(hash, &layout);
            }
            storage.stamps.save()
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!("saving file stamps failed: {e}"),
            Err(e) => warn!("saving file stamps failed: {e}"),
        }
    }

    pub(super) async fn repair_session(&self) {
        // A move or a restart is under way; the next try will see how it ended.
        if let Ok(_g) = self.admin.try_lock() {
            self.repair_locked().await;
        }
    }

    /// Call with `admin` held: nothing else is taking torrents out of the session then.
    pub(super) async fn repair_locked(&self) {
        let session = self.session();
        let (present, used_ids) = session.with_torrents(|it| {
            let mut present = HashSet::new();
            let mut used_ids = HashSet::new();
            for (id, h) in it {
                present.insert(h.info_hash());
                used_ids.insert(id);
            }
            (present, used_ids)
        });
        let gone: Vec<(Id20, Record)> = self
            .records
            .read()
            .iter()
            .filter(|(h, _)| !present.contains(*h))
            .map(|(h, r)| (*h, r.clone()))
            .collect();
        self.missing
            .lock()
            .retain(|h, _| gone.iter().any(|(g, _)| g == h));
        if gone.is_empty() {
            return;
        }
        let dir = self.data_dir.join("session");
        let saved = {
            let dir = dir.clone();
            tokio::task::spawn_blocking(move || session_db::entries(&dir))
                .await
                .unwrap_or_default()
        };
        for (hash, rec) in gone {
            let result = self
                .put_back(&session, hash, &rec, &dir, &saved, &used_ids)
                .await;
            match result {
                Ok(()) => {
                    info!("put {} back in the session", rec.name);
                    self.missing.lock().remove(&hash);
                }
                Err(e) => {
                    let why = format!("couldn't load this torrent: {e:#}");
                    let mut missing = self.missing.lock();
                    if missing.get(&hash) != Some(&why) {
                        warn!("{}: {why}", rec.name);
                        missing.insert(hash, why);
                    }
                }
            }
        }
    }

    async fn put_back(
        &self,
        session: &std::sync::Arc<Session>,
        hash: Id20,
        rec: &Record,
        dir: &Path,
        saved: &HashMap<String, Entry>,
        used_ids: &HashSet<usize>,
    ) -> anyhow::Result<()> {
        let name = hash.as_string();
        let bytes = tokio::fs::read(dir.join(format!("{name}.torrent")))
            .await
            .context("its saved .torrent file can't be read; remove it and add it again")?;
        ensure!(
            bytes.len() <= input::MAX_TORRENT_BYTES,
            "its saved .torrent file is too large"
        );
        let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&bytes)
            .context("its saved .torrent file is damaged")?;
        ensure!(
            parsed.info_hash == hash,
            "its saved .torrent file is for a different torrent"
        );
        drop(parsed);
        let entry = saved.get(&name);
        self.factory.register(
            hash,
            TorrentRoot {
                path: rec.root.clone(),
                owned: rec.owned_root,
                renames: rec.renames.clone(),
            },
        );
        let opts = AddTorrentOptions {
            paused: true, // the queue decides when it runs again
            overwrite: true,
            output_folder: Some(
                rec.root
                    .to_str()
                    .context("folder path is not valid Unicode")?
                    .to_string(),
            ),
            only_files: entry.and_then(|e| e.only_files.clone()),
            trackers: entry.map(|e| e.trackers.clone()).filter(|t| !t.is_empty()),
            // Taking its old id replaces its saved entry instead of adding a second one.
            preferred_id: entry.map(|e| e.id).filter(|id| !used_ids.contains(id)),
            ..Default::default()
        };
        session
            .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
            .await?;
        Ok(())
    }

    /// Takes a torrent out of the session while its record stays.
    #[doc(hidden)]
    pub async fn drop_from_session(&self, id: &str) -> anyhow::Result<()> {
        let hash = super::parse_hash(id)?;
        self.session()
            .delete(TorrentIdOrHash::Hash(hash), false)
            .await
    }

    #[doc(hidden)]
    pub async fn repair_now(&self) {
        let _g = self.admin.lock().await;
        self.repair_locked().await;
    }
}
