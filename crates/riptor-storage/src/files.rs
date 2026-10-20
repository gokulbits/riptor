//! Lazily opened file handles with an LRU cap (Linux defaults to 1024 fds).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use lru::LruCache;
use parking_lot::Mutex;

struct Handle {
    file: Arc<File>,
    writable: bool,
}

pub struct FilePool {
    paths: Vec<PathBuf>,
    slots: Vec<Mutex<Option<Handle>>>,
    order: Mutex<LruCache<usize, ()>>,
    max_open: usize,
}

impl FilePool {
    pub fn new(paths: Vec<PathBuf>, max_open: usize) -> Self {
        let slots = paths.iter().map(|_| Mutex::new(None)).collect();
        Self {
            paths,
            slots,
            order: Mutex::new(LruCache::unbounded()),
            max_open: max_open.max(1),
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
                    self.order.lock().promote(&id);
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
        let victims: Vec<usize> = {
            let mut order = self.order.lock();
            order.put(id, ());
            let mut v = Vec::new();
            while order.len() > self.max_open {
                if let Some((victim, _)) = order.pop_lru() {
                    v.push(victim);
                }
            }
            v
        };
        for v in victims {
            *self.slots[v].lock() = None;
        }
        Ok(file)
    }

    pub fn close(&self, id: usize) {
        *self.slots[id].lock() = None;
        self.order.lock().pop(&id);
    }

    pub fn close_all(&self) {
        for s in &self.slots {
            *s.lock() = None;
        }
        self.order.lock().clear();
    }

    pub fn sync_all(&self) -> io::Result<()> {
        for s in &self.slots {
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
        self.order.lock().len()
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
        let pool = FilePool::new(paths, 2);
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
        let pool = FilePool::new(vec![dir.path().join("missing")], 2);
        assert!(pool.get(0, false).is_err());
        assert!(!dir.path().join("missing").exists());
    }
}
