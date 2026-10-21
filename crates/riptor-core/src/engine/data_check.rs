//! Schedules the background data check and acts on what it finds.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use librqbit_core::hash_id::Id20;
use riptor_storage::layout_for;
use tracing::{info, warn};

use super::{parse_hash, unix_now, Engine, Job};
use crate::model::NoticeLevel;
use crate::scrub::{self, Plan, Progress};

const DAY: u64 = 86_400;

impl Engine {
    /// Checks the data of these torrents now, while they keep seeding.
    pub fn check_data(&self, ids: &[String]) -> anyhow::Result<()> {
        for id in ids {
            self.queue_job(Job::Scrub(parse_hash(id)?));
        }
        Ok(())
    }

    /// Queues the finished torrent that's waited longest, if one is due.
    pub(super) fn schedule_data_check(&self) {
        let Some(days) = self.settings.read().scrub_days else {
            return;
        };
        let now = unix_now();
        let due = self
            .records
            .read()
            .iter()
            .filter(|(_, r)| r.move_on_complete.is_none())
            .filter_map(|(h, r)| {
                // A new download was just verified, so it counts from completion.
                let last = r.last_scrub.or(r.completed_on)?;
                (now.saturating_sub(last) >= u64::from(days) * DAY).then_some((last, *h))
            })
            .min();
        if let Some((_, h)) = due {
            self.queue_job(Job::Scrub(h));
        }
    }

    pub(super) fn data_check_progress(&self, hash: &Id20) -> Option<f32> {
        let p = self.scrubbing.lock().get(hash)?.clone();
        let total = p.total.load(Ordering::Relaxed).max(1);
        Some((p.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0) as f32)
    }

    /// Moves, rechecks and removals call this: the files are about to change under it.
    pub(super) fn stop_data_check(&self, hash: &Id20) {
        if let Some(p) = self.scrubbing.lock().get(hash) {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) async fn scrub(&self, hash: Id20) {
        let Some(root) = self.factory.root_of(&hash) else {
            return;
        };
        let complete = self
            .records
            .read()
            .get(&hash)
            .is_some_and(|r| r.completed_on.is_some() && r.move_on_complete.is_none());
        let Ok(t) = self.handle(&hash) else {
            return;
        };
        if !complete {
            return;
        }
        let only = t.only_files();
        let have = t.have_pieces().unwrap_or_default();
        let plan = t.with_metadata(|m| -> anyhow::Result<Plan> {
            let layout = layout_for(&root, m)?;
            let wanted = (0..layout.files.len())
                .map(|i| only.as_ref().is_none_or(|o| o.contains(&i)))
                .collect();
            Ok(Plan {
                layout,
                hashes: m.info.info().pieces.as_ref().to_vec(),
                have: have.clone(),
                wanted,
            })
        });
        drop(t);
        let plan = match plan {
            Ok(Ok(p)) => p,
            Ok(Err(e)) | Err(e) => {
                warn!("data check: {e:#}");
                return;
            }
        };
        let progress = Arc::new(Progress::default());
        self.scrubbing.lock().insert(hash, progress.clone());
        let rate = u64::from(self.settings.read().scrub_mib_per_sec) * 1024 * 1024;
        let p = progress.clone();
        let result = tokio::task::spawn_blocking(move || scrub::run(&plan, &p, rate)).await;
        self.scrubbing.lock().remove(&hash);
        let bad = match result {
            Ok(Ok(bad)) => bad,
            Ok(Err(e)) => {
                info!("data check stopped: {e:#}");
                return;
            }
            Err(e) => {
                warn!("data check failed: {e}");
                return;
            }
        };
        let name = {
            let mut records = self.records.write();
            let Some(r) = records.get_mut(&hash) else {
                return;
            };
            r.last_scrub = Some(unix_now());
            r.name.clone()
        };
        self.dirty.store(true, Ordering::Relaxed);
        if bad.is_empty() {
            info!("data check: {name} is intact");
            return;
        }
        warn!("data check: {} damaged pieces in {name}", bad.len());
        self.notice(
            NoticeLevel::Warning,
            format!(
                "{name}: {} piece(s) on disk are damaged. Riptor is checking the whole torrent again and will download them again.",
                bad.len()
            ),
        );
        if let Err(e) = self.recheck(&[hash.as_string()]).await {
            warn!("recheck after the data check failed: {e:#}");
        }
    }
}
