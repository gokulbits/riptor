//! Shared memory accounting: a byte-weighted LRU read cache for seeding, and
//! a byte budget for not-yet-flushed downloaded data. Both are shared by every
//! torrent so one global "disk cache" setting bounds total memory.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use lru::LruCache;
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Key {
    pub store: u64,
    pub file: u32,
    pub extent: u64,
}

struct Inner {
    lru: LruCache<Key, Arc<[u8]>>,
    bytes: usize,
}

pub struct ReadCache {
    inner: Mutex<Inner>,
    capacity: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl ReadCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                lru: LruCache::unbounded(),
                bytes: 0,
            }),
            capacity: AtomicUsize::new(capacity),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    pub fn enabled(&self) -> bool {
        self.capacity.load(Ordering::Relaxed) > 0
    }

    pub fn set_capacity(&self, cap: usize) {
        self.capacity.store(cap, Ordering::Relaxed);
        let mut g = self.inner.lock();
        Self::shrink(&mut g, cap);
    }

    fn shrink(g: &mut Inner, cap: usize) {
        while g.bytes > cap {
            match g.lru.pop_lru() {
                Some((_, v)) => g.bytes -= v.len(),
                None => break,
            }
        }
    }

    pub fn get(&self, k: &Key) -> Option<Arc<[u8]>> {
        let r = self.inner.lock().lru.get(k).cloned();
        let counter = if r.is_some() {
            &self.hits
        } else {
            &self.misses
        };
        counter.fetch_add(1, Ordering::Relaxed);
        r
    }

    pub fn insert(&self, k: Key, v: Arc<[u8]>) {
        let cap = self.capacity.load(Ordering::Relaxed);
        if v.len() > cap / 4 {
            return; // never let one extent dominate the cache
        }
        let mut g = self.inner.lock();
        g.bytes += v.len();
        if let Some(old) = g.lru.put(k, v) {
            g.bytes -= old.len();
        }
        Self::shrink(&mut g, cap);
    }

    pub fn invalidate(&self, store: u64, file: u32, extents: std::ops::RangeInclusive<u64>) {
        let mut g = self.inner.lock();
        if g.lru.is_empty() {
            return;
        }
        for extent in extents {
            if let Some(v) = g.lru.pop(&Key {
                store,
                file,
                extent,
            }) {
                g.bytes -= v.len();
            }
        }
    }

    pub fn purge_store(&self, store: u64) {
        let mut g = self.inner.lock();
        let keys: Vec<Key> = g
            .lru
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| k.store == store)
            .collect();
        for k in keys {
            if let Some(v) = g.lru.pop(&k) {
                g.bytes -= v.len();
            }
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            bytes: self.inner.lock().bytes,
            capacity: self.capacity.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct CacheStats {
    pub bytes: usize,
    pub capacity: usize,
    pub hits: usize,
    pub misses: usize,
}

/// Bytes of downloaded data held in memory and not yet on disk.
pub struct WriteBudget {
    used: AtomicUsize,
    capacity: AtomicUsize,
}

impl WriteBudget {
    pub fn new(capacity: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            capacity: AtomicUsize::new(capacity),
        }
    }
    pub fn enabled(&self) -> bool {
        self.capacity.load(Ordering::Relaxed) > 0
    }
    pub fn set_capacity(&self, cap: usize) {
        self.capacity.store(cap, Ordering::Relaxed);
    }
    pub fn add(&self, delta: isize) {
        if delta >= 0 {
            self.used.fetch_add(delta as usize, Ordering::Relaxed);
        } else {
            self.used.fetch_sub(delta.unsigned_abs(), Ordering::Relaxed);
        }
    }
    pub fn over(&self) -> bool {
        self.used.load(Ordering::Relaxed) > self.capacity.load(Ordering::Relaxed)
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(e: u64) -> Key {
        Key {
            store: 1,
            file: 0,
            extent: e,
        }
    }

    #[test]
    fn evicts_by_bytes() {
        let c = ReadCache::new(100);
        c.insert(k(0), vec![0; 20].into());
        c.insert(k(1), vec![0; 20].into());
        c.insert(k(2), vec![0; 20].into());
        assert!(c.get(&k(0)).is_some()); // touch 0 so 1 is LRU
        c.insert(k(3), vec![0; 20].into());
        c.insert(k(4), vec![0; 20].into());
        c.insert(k(5), vec![0; 20].into());
        assert_eq!(c.stats().bytes, 100);
        assert!(c.get(&k(1)).is_none());
        assert!(c.get(&k(0)).is_some());
        // Too-large values are ignored.
        c.insert(k(9), vec![0; 60].into());
        assert!(c.get(&k(9)).is_none());
        c.invalidate(1, 0, 0..=3);
        assert!(c.get(&k(0)).is_none());
        c.purge_store(1);
        assert_eq!(c.stats().bytes, 0);
    }
}
