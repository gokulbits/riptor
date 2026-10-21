//! Checks on what a torrent contains: checksum files and disguised files.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use librqbit_core::hash_id::Id20;
use tracing::{info, warn};

use super::{parse_hash, unix_now, Engine, Job};
use crate::checksums::{self, FileRef};
use crate::model::{NoticeLevel, Safety};
use crate::suspicion::{self, Level};

impl Engine {
    pub(super) fn queue_job(&self, job: Job) {
        let mut jobs = self.jobs.lock();
        if jobs.1.insert(job) {
            jobs.0.push(job);
        }
    }

    pub(super) fn take_jobs(&self) -> Vec<Job> {
        std::mem::take(&mut self.jobs.lock().0)
    }

    pub(super) async fn run_job(self: Arc<Self>, job: Job) {
        match job {
            Job::VpnAddress => self.look_up_vpn_address().await,
            Job::NetInfo => self.refresh_net_info().await,
            Job::Scrub(h) => {
                let _one_at_a_time = self.disk_job.lock().await;
                self.scrub(h).await
            }
            Job::Checksums(h) => {
                let _one_at_a_time = self.disk_job.lock().await;
                self.check_sums(h).await
            }
            Job::ContentScan(h) => {
                let _one_at_a_time = self.disk_job.lock().await;
                self.scan_content(h).await
            }
        }
        self.jobs.lock().1.remove(&job);
    }

    /// Checks the torrents against the checksum files they contain.
    pub fn verify_checksums(&self, ids: &[String]) -> anyhow::Result<()> {
        for id in ids {
            self.queue_job(Job::Checksums(parse_hash(id)?));
        }
        Ok(())
    }

    pub fn safety(&self, id: &str) -> anyhow::Result<Safety> {
        let hash = parse_hash(id)?;
        let (checksums, findings) = self
            .records
            .read()
            .get(&hash)
            .map(|r| {
                let findings = r
                    .findings
                    .iter()
                    .chain(&r.content_findings)
                    .flatten()
                    .cloned()
                    .collect();
                (r.checksums.clone(), findings)
            })
            .unwrap_or_default();
        let has_manifest = self
            .files(id)
            .map(|f| f.iter().any(|f| checksums::is_manifest(&f.path)))
            .unwrap_or(false);
        let secret_trackers = self
            .trackers(id)
            .unwrap_or_default()
            .iter()
            .filter(|t| crate::passkey::has_secret(t))
            .count();
        let private = self
            .handle(&hash)
            .ok()
            .and_then(|t| t.with_metadata(|m| m.info.info().private).ok())
            .unwrap_or(false);
        Ok(Safety {
            has_manifest,
            checksums,
            secret_trackers,
            private,
            findings,
        })
    }

    /// An image or text file shown inside Riptor; only once it's fully downloaded.
    pub async fn preview_file(
        &self,
        id: &str,
        index: usize,
    ) -> anyhow::Result<crate::preview::FilePreview> {
        let (path, len, ready) = self.file_path(id, index)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        anyhow::ensure!(
            ready >= len && path.is_file(),
            "this file isn't fully downloaded yet"
        );
        anyhow::ensure!(
            !self.flagged_dangerous(id, index),
            "this file isn't what its name says, so it isn't shown"
        );
        tokio::task::spawn_blocking(move || crate::preview::read(&path, &name)).await?
    }

    /// A file found to be a disguised program is never opened from Riptor.
    pub fn flagged_dangerous(&self, id: &str, index: usize) -> bool {
        let Ok(hash) = parse_hash(id) else {
            return true;
        };
        let Some(rel) = self
            .files(id)
            .ok()
            .and_then(|f| f.into_iter().find(|f| f.index == index))
            .map(|f| f.path.replace('\\', "/"))
        else {
            return true;
        };
        self.records.read().get(&hash).is_some_and(|r| {
            r.findings
                .iter()
                .chain(&r.content_findings)
                .flatten()
                .any(|f| f.level == Level::Danger && f.file.as_deref().is_some_and(|p| p == rel))
        })
    }

    fn file_refs(&self, hash: &Id20) -> Option<Vec<FileRef>> {
        let files = self.files(&hash.as_string()).ok()?;
        let root = self.factory.root_of(hash)?;
        Some(
            files
                .iter()
                .map(|f| FileRef {
                    rel: f.path.replace('\\', "/"),
                    path: root.path.join(&f.path),
                    complete: f.selected && f.done >= f.size,
                })
                .collect(),
        )
    }

    async fn check_sums(&self, hash: Id20) {
        let Some(refs) = self.file_refs(&hash) else {
            return;
        };
        if !refs.iter().any(|f| checksums::is_manifest(&f.rel)) {
            return;
        }
        let now = unix_now();
        let report = match tokio::task::spawn_blocking(move || checksums::verify(&refs, now)).await
        {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                warn!("checksum check stopped: {e}");
                return;
            }
        };
        let name = {
            let mut records = self.records.write();
            let Some(r) = records.get_mut(&hash) else {
                return;
            };
            r.checksums = Some(report.clone());
            r.name.clone()
        };
        self.dirty.store(true, Ordering::Relaxed);
        let listed = report.manifests.join(", ");
        if report.mismatched_count > 0 {
            warn!(
                "{name}: {} files don't match {listed}",
                report.mismatched_count
            );
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "{name}: {} file(s) don't match {listed}, starting with {}. Don't trust them; try Force recheck or get them from elsewhere.",
                    report.mismatched_count,
                    report.mismatched.first().map(String::as_str).unwrap_or("?")
                ),
            );
        } else if report.verified > 0 {
            info!("{name}: {} files match {listed}", report.verified);
            self.notice(
                NoticeLevel::Info,
                format!("{name}: {} file(s) match {listed}.", report.verified),
            );
        }
    }

    /// Looks at the first bytes of each finished file.
    async fn scan_content(&self, hash: Id20) {
        let Some(refs) = self.file_refs(&hash) else {
            return;
        };
        let found = tokio::task::spawn_blocking(move || {
            refs.iter()
                .filter(|f| f.complete)
                .filter_map(|f| suspicion::check_content(&f.rel, &f.path))
                .take(20)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let name = {
            let mut records = self.records.write();
            let Some(r) = records.get_mut(&hash) else {
                return;
            };
            r.content_findings = Some(found.clone());
            r.name.clone()
        };
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(f) = found.iter().find(|f| f.level == Level::Danger) {
            warn!("{name}: {}", f.message);
            self.notice(NoticeLevel::Warning, format!("{name}: {}", f.message));
        }
    }
}
