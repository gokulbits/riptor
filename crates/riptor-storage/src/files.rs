//! Lazily opened file handles. The OS limit is per process, so all torrents share one LRU cap.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use lru::LruCache;
use parking_lot::Mutex;

struct Handle {
    file: Arc<File>,
    writable: bool,
}

type Slots = Vec<Mutex<Option<Handle>>>;

/// The handles open across every torrent, oldest first.
pub struct OpenFiles {
    order: Mutex<LruCache<(u64, usize), Weak<Slots>>>,
    max_open: AtomicUsize,
    next_pool: AtomicU64,
}

impl OpenFiles {
    pub fn new(max_open: usize) -> Arc<Self> {
        Arc::new(Self {
            order: Mutex::new(LruCache::unbounded()),
            max_open: AtomicUsize::new(max_open.max(1)),
            next_pool: AtomicU64::new(0),
        })
    }

    pub fn set_max_open(&self, max_open: usize) {
        self.max_open.store(max_open.max(1), Ordering::Relaxed);
    }

    pub fn open_count(&self) -> usize {
        self.order.lock().len()
    }
}

pub struct FilePool {
    id: u64,
    paths: Vec<PathBuf>,
    slots: Arc<Slots>,
    open: Arc<OpenFiles>,
}

impl FilePool {
    pub fn new(paths: Vec<PathBuf>, open: Arc<OpenFiles>) -> Self {
        let slots = Arc::new(paths.iter().map(|_| Mutex::new(None)).collect());
        Self {
            id: open.next_pool.fetch_add(1, Ordering::Relaxed),
            paths,
            slots,
            open,
        }
    }

    pub fn path(&self, id: usize) -> &PathBuf {
        &self.paths[id]
    }

    /// `write` creates the file and its folders.
    pub fn get(&self, id: usize, write: bool) -> io::Result<Arc<File>> {
        {
            let slot = self.slots[id].lock();
            if let Some(h) = &*slot {
                if h.writable || !write {
                    let f = h.file.clone();
                    drop(slot);
                    self.open.order.lock().promote(&(self.id, id));
                    return Ok(f);
                }
            }
        }
        let path = &self.paths[id];
        let (file, writable) = if write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            (f, true)
        } else {
            match OpenOptions::new().read(true).write(true).open(path) {
                Ok(f) => (f, true),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => (File::open(path)?, false),
                Err(e) => return Err(e),
            }
        };
        let file = Arc::new(file);
        *self.slots[id].lock() = Some(Handle {
            file: file.clone(),
            writable,
        });
        let victims = {
            let mut order = self.open.order.lock();
            order.put((self.id, id), Arc::downgrade(&self.slots));
            let max_open = self.open.max_open.load(Ordering::Relaxed);
            let mut v = Vec::new();
            while order.len() > max_open {
                if let Some(victim) = order.pop_lru() {
                    v.push(victim);
                }
            }
            v
        };
        for ((_, file_id), slots) in victims {
            if let Some(slots) = slots.upgrade() {
                *slots[file_id].lock() = None;
            }
        }
        Ok(file)
    }

    pub fn close(&self, id: usize) {
        *self.slots[id].lock() = None;
        self.open.order.lock().pop(&(self.id, id));
    }

    pub fn close_all(&self) {
        for s in self.slots.iter() {
            *s.lock() = None;
        }
        let mut order = self.open.order.lock();
        for id in 0..self.paths.len() {
            order.pop(&(self.id, id));
        }
    }

    pub fn sync_all(&self) -> io::Result<()> {
        for s in self.slots.iter() {
            let f = s
                .lock()
                .as_ref()
                .filter(|h| h.writable)
                .map(|h| h.file.clone());
            if let Some(f) = f {
                f.sync_data()?;
            }
        }
        Ok(())
    }

    pub fn open_count(&self) -> usize {
        self.slots.iter().filter(|s| s.lock().is_some()).count()
    }
}

impl Drop for FilePool {
    fn drop(&mut self) {
        self.close_all();
    }
}

#[cfg(unix)]
pub fn read_exact_at(f: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, offset)
}

#[cfg(unix)]
pub fn write_all_at(f: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, offset)
}

#[cfg(windows)]
pub fn read_exact_at(f: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_read(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                offset += n as u64;
                buf = &mut buf[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn write_all_at(f: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_write(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                offset += n as u64;
                buf = &buf[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_cap_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..5)
            .map(|i| dir.path().join("sub").join(format!("{i}")))
            .collect();
        let pool = FilePool::new(paths, OpenFiles::new(2));
        for i in 0..5 {
            let f = pool.get(i, true).unwrap();
            write_all_at(&f, &[i as u8; 3], 10).unwrap();
        }
        assert_eq!(pool.open_count(), 2);
        let f = pool.get(0, false).unwrap();
        let mut b = [0u8; 3];
        read_exact_at(&f, &mut b, 10).unwrap();
        assert_eq!(b, [0, 0, 0]);
        let f = pool.get(4, false).unwrap();
        read_exact_at(&f, &mut b, 10).unwrap();
        assert_eq!(b, [4, 4, 4]);
        let pool = FilePool::new(vec![dir.path().join("missing")], OpenFiles::new(2));
        assert!(pool.get(0, false).is_err());
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn the_cap_covers_every_torrent_together() {
        let dir = tempfile::tempdir().unwrap();
        let open = OpenFiles::new(3);
        let pool = |name: &str| {
            let paths = (0..4)
                .map(|i| dir.path().join(format!("{name}{i}")))
                .collect();
            FilePool::new(paths, open.clone())
        };
        let (a, b) = (pool("a"), pool("b"));
        for i in 0..4 {
            a.get(i, true).unwrap();
        }
        assert_eq!(a.open_count(), 3);
        // Opening files for one torrent closes the longest-unused ones of another.
        for i in 0..2 {
            b.get(i, true).unwrap();
        }
        assert_eq!(
            (a.open_count(), b.open_count(), open.open_count()),
            (1, 2, 3)
        );
        // Reopening still works, and a removed torrent gives its share back.
        a.get(0, false).unwrap();
        assert_eq!(open.open_count(), 3);
        drop(a);
        assert_eq!((b.open_count(), open.open_count()), (2, 2));
        open.set_max_open(1);
        b.get(2, true).unwrap();
        assert_eq!(open.open_count(), 1);
    }
}
