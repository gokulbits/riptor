//! Writes state files off the async runtime, newest contents only, skipping unchanged ones.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::warn;

use crate::state_file;

#[derive(Clone, Default)]
pub struct Saver {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    pending: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    // What's on disk, by path.
    written: Mutex<HashMap<PathBuf, Vec<u8>>>,
    // One writer at a time; pending is taken only while holding it, so older bytes never land last.
    turn: Mutex<()>,
    background: AtomicBool,
    writes: AtomicU64,
}

impl Saver {
    /// Latest wins; bytes already on disk are dropped.
    pub fn queue(&self, path: PathBuf, bytes: Vec<u8>) {
        if self.inner.written.lock().get(&path) == Some(&bytes) {
            self.inner.pending.lock().remove(&path);
            return;
        }
        self.inner.pending.lock().insert(path, bytes);
    }

    /// Writes everything queued on this thread. Blocks on disk I/O.
    pub fn flush_blocking(&self) -> anyhow::Result<()> {
        self.inner.flush()
    }

    /// Writes everything queued without blocking the runtime, and waits for it.
    pub async fn flush(&self) -> anyhow::Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.flush()).await?
    }

    /// Starts a write in the background unless one is already running; failures stay queued.
    pub fn flush_soon(&self) {
        if self.inner.pending.lock().is_empty()
            || self.inner.background.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            loop {
                if let Err(e) = inner.flush() {
                    warn!("saving state failed: {e:#}");
                    inner.background.store(false, Ordering::Release);
                    return;
                }
                inner.background.store(false, Ordering::Release);
                // Something queued after the last write and saw us still running.
                if inner.pending.lock().is_empty() || inner.background.swap(true, Ordering::AcqRel)
                {
                    return;
                }
            }
        });
    }

    pub fn writes(&self) -> u64 {
        self.inner.writes.load(Ordering::Relaxed)
    }
}

impl Inner {
    fn flush(&self) -> anyhow::Result<()> {
        let _turn = self.turn.lock();
        let batch = std::mem::take(&mut *self.pending.lock());
        let mut failed = None;
        let mut rest = batch.into_iter();
        for (path, bytes) in rest.by_ref() {
            if self.written.lock().get(&path) == Some(&bytes) {
                continue;
            }
            match state_file::save_bytes(&path, &bytes) {
                Ok(()) => {
                    self.writes.fetch_add(1, Ordering::Relaxed);
                    self.written.lock().insert(path, bytes);
                }
                Err(e) => {
                    failed = Some((e, path, bytes));
                    break;
                }
            }
        }
        let Some((e, path, bytes)) = failed else {
            return Ok(());
        };
        // Put back what wasn't written, unless something newer arrived meanwhile.
        let mut pending = self.pending.lock();
        for (p, b) in std::iter::once((path, bytes)).chain(rest) {
            pending.entry(p).or_insert(b);
        }
        Err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_files_are_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = Saver::default();
        s.queue(p.clone(), b"one".to_vec());
        s.flush_blocking().unwrap();
        s.queue(p.clone(), b"one".to_vec());
        s.flush_blocking().unwrap();
        assert_eq!(s.writes(), 1);
        s.queue(p.clone(), b"two".to_vec());
        s.flush_blocking().unwrap();
        assert_eq!(s.writes(), 2);
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
    }

    #[test]
    fn only_the_newest_contents_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = Saver::default();
        for i in 0..10 {
            s.queue(p.clone(), format!("{i}").into_bytes());
        }
        s.flush_blocking().unwrap();
        assert_eq!(s.writes(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"9");
    }

    #[test]
    fn queueing_what_is_on_disk_cancels_a_pending_change() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = Saver::default();
        s.queue(p.clone(), b"a".to_vec());
        s.flush_blocking().unwrap();
        s.queue(p.clone(), b"b".to_vec());
        s.queue(p.clone(), b"a".to_vec());
        s.flush_blocking().unwrap();
        assert_eq!(s.writes(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"a");
    }

    #[test]
    fn failed_writes_stay_queued_and_retry() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the folder should be makes the write fail.
        let blocker = dir.path().join("sub");
        std::fs::write(&blocker, b"").unwrap();
        let p = blocker.join("state.json");
        let other = dir.path().join("other.json");
        let s = Saver::default();
        s.queue(p.clone(), b"x".to_vec());
        s.queue(other.clone(), b"y".to_vec());
        assert!(s.flush_blocking().is_err());
        std::fs::remove_file(&blocker).unwrap();
        s.flush_blocking().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        assert_eq!(std::fs::read(&other).unwrap(), b"y");
    }

    #[test]
    fn newer_contents_win_over_a_failed_write() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("sub");
        std::fs::write(&blocker, b"").unwrap();
        let p = blocker.join("state.json");
        let s = Saver::default();
        s.queue(p.clone(), b"old".to_vec());
        assert!(s.flush_blocking().is_err());
        s.queue(p.clone(), b"new".to_vec());
        std::fs::remove_file(&blocker).unwrap();
        s.flush_blocking().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert_eq!(s.writes(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn background_writes_land_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = Saver::default();
        let mut tasks = Vec::new();
        for i in 0..200u32 {
            let s = s.clone();
            let p = p.clone();
            tasks.push(tokio::spawn(async move {
                s.queue(p, i.to_string().into_bytes());
                s.flush_soon();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        s.queue(p.clone(), b"last".to_vec());
        s.flush().await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"last");
        assert!(!p.with_extension("tmp").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn flush_soon_returns_before_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = Saver::default();
        let turn = s.inner.turn.lock();
        s.queue(p.clone(), b"x".to_vec());
        // Would deadlock this single-threaded runtime if it wrote inline.
        s.flush_soon();
        assert!(!p.exists());
        drop(turn);
        s.flush().await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
    }
}
