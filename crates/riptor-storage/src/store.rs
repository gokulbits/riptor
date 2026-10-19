//! The storage engine proper, independent of librqbit's trait so it can be
//! tested directly.
//!
//! Write path: blocks land in per-piece [`Segments`] in memory. The engine's
//! hash check reads them back from memory. When the piece verifies,
//! [`Store::piece_completed`] writes it to disk as a few large writes *before*
//! the engine announces the piece to peers, so anything a peer can request is
//! already on disk. If memory runs short, the least recently touched pieces
//! are spilled to disk early.
//!
//! Read path: buffered bytes are overlaid on disk data; otherwise reads go
//! through an extent-sized LRU cache, so a peer fetching a piece as sixteen
//! 16 KiB blocks costs one disk read instead of sixteen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{ensure, Context};
use parking_lot::Mutex;
use tracing::{debug, warn};

use crate::cache::{Key, ReadCache, WriteBudget};
use crate::files::{read_exact_at, write_all_at, FilePool};
use crate::layout::Layout;
use crate::segments::Segments;
use crate::{os, Preallocation, StorageConfig};

static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);

/// Resources shared by every torrent.
pub struct Shared {
    pub read_cache: ReadCache,
    pub write_budget: WriteBudget,
    pub config: Mutex<StorageConfig>,
    pub metrics: Metrics,
}

/// Cumulative disk activity across all torrents since the session started.
#[derive(Default)]
pub struct Metrics {
    pub pieces_flushed: AtomicU64,
    /// Unfinished pieces written early because the write buffer was full
    /// (also known as "write cache overload").
    pub spills: AtomicU64,
    pub bytes_written: AtomicU64,
    pub bytes_read: AtomicU64,
    pub disk_writes: AtomicU64,
    pub disk_reads: AtomicU64,
    pub flush_nanos: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct MetricsSnapshot {
    pub pieces_flushed: u64,
    pub spills: u64,
    pub bytes_written: u64,
    pub bytes_read: u64,
    pub disk_writes: u64,
    pub disk_reads: u64,
    /// Average time to commit one verified piece, in microseconds.
    pub avg_flush_us: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        let flushed = self.pieces_flushed.load(Ordering::Relaxed);
        MetricsSnapshot {
            pieces_flushed: flushed,
            spills: self.spills.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            disk_writes: self.disk_writes.load(Ordering::Relaxed),
            disk_reads: self.disk_reads.load(Ordering::Relaxed),
            avg_flush_us: self.flush_nanos.load(Ordering::Relaxed) / flushed.max(1) / 1000,
        }
    }
}

impl Shared {
    pub fn new(config: StorageConfig) -> Arc<Self> {
        Arc::new(Self {
            read_cache: ReadCache::new(config.read_cache_bytes),
            write_budget: WriteBudget::new(config.write_buffer_bytes),
            config: Mutex::new(config),
            metrics: Metrics::default(),
        })
    }

    pub fn reconfigure(&self, config: StorageConfig) {
        self.read_cache.set_capacity(config.read_cache_bytes);
        self.write_budget.set_capacity(config.write_buffer_bytes);
        *self.config.lock() = config;
    }
}

struct PieceBuf {
    segs: Segments,
    /// Set once flushed and removed from the map; holders must look it up again.
    dead: bool,
}

struct PieceEntry {
    touched: AtomicU64,
    buf: Mutex<PieceBuf>,
}

pub struct Store {
    id: u64,
    layout: Layout,
    files: FilePool,
    pieces: Mutex<HashMap<u32, Arc<PieceEntry>>>,
    shared: Arc<Shared>,
    clock: AtomicU64,
    /// Bumped on every invalidation; a cache fill that raced with a write is dropped.
    generation: AtomicU64,
    extent: u64,
}

impl Store {
    pub fn new(layout: Layout, shared: Arc<Shared>) -> Self {
        let cfg = shared.config.lock().clone();
        let files = FilePool::new(
            layout.files.iter().map(|f| f.path.clone()).collect(),
            cfg.max_open_files,
        );
        // One extent ~ one piece, bounded so small reads stay cheap and big pieces
        // do not blow the cache.
        let extent = layout.piece_len.clamp(64 * 1024, 1024 * 1024);
        Self {
            id: NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed),
            layout,
            files,
            pieces: Mutex::new(HashMap::new()),
            shared,
            clock: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            extent,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn check_bounds(
        &self,
        file_id: usize,
        offset: u64,
        len: usize,
    ) -> anyhow::Result<&crate::layout::FileSlot> {
        let slot = self.layout.files.get(file_id).context("no such file")?;
        let end = offset.checked_add(len as u64).context("offset overflow")?;
        ensure!(
            end <= slot.len,
            "access past end of file {file_id}: {offset}+{len} > {}",
            slot.len
        );
        Ok(slot)
    }

    // ---------- write path ----------

    pub fn write(&self, file_id: usize, offset: u64, bufs: &[&[u8]]) -> anyhow::Result<usize> {
        let len: usize = bufs.iter().map(|b| b.len()).sum();
        let slot = self.check_bounds(file_id, offset, len)?;
        if slot.padding || len == 0 {
            return Ok(len);
        }
        let abs = slot.offset + offset;
        let piece = self.layout.piece_of(abs);
        let (pstart, plen) = self.layout.piece_bounds(piece);

        if !self.shared.write_budget.enabled() || abs + len as u64 > pstart + plen {
            // Buffering off, or a write spanning pieces (the engine never does this).
            let mut pos = offset;
            for b in bufs {
                self.disk_write(file_id, pos, b)?;
                pos += b.len() as u64;
            }
            return Ok(len);
        }

        let mut data = Vec::with_capacity(len);
        for b in bufs {
            data.extend_from_slice(b);
        }
        let rel = abs - pstart;
        loop {
            let entry = self.entry(piece, true).expect("created");
            let mut g = entry.buf.lock();
            if g.dead {
                continue;
            }
            entry.touched.store(
                self.clock.fetch_add(1, Ordering::Relaxed),
                Ordering::Relaxed,
            );
            let delta = g.segs.insert(rel, std::mem::take(&mut data));
            self.shared.write_budget.add(delta);
            break;
        }
        self.invalidate(file_id, offset, len as u64);
        if self.shared.write_budget.over() {
            self.spill(piece)?;
        }
        Ok(len)
    }

    fn entry(&self, piece: u32, create: bool) -> Option<Arc<PieceEntry>> {
        let mut map = self.pieces.lock();
        if let Some(e) = map.get(&piece) {
            return Some(e.clone());
        }
        if !create {
            return None;
        }
        let e = Arc::new(PieceEntry {
            touched: AtomicU64::new(0),
            buf: Mutex::new(PieceBuf {
                segs: Segments::default(),
                dead: false,
            }),
        });
        map.insert(piece, e.clone());
        Some(e)
    }

    /// Write a piece's buffered bytes to disk and drop the buffer.
    fn flush_piece(&self, piece: u32, entry: &Arc<PieceEntry>) -> anyhow::Result<()> {
        let mut g = entry.buf.lock();
        if g.dead {
            return Ok(());
        }
        let (pstart, _) = self.layout.piece_bounds(piece);
        for (rel, data) in g.segs.runs() {
            for span in self.layout.spans(pstart + rel, data.len() as u64) {
                if self.layout.files[span.file_id].padding {
                    continue;
                }
                let chunk = &data[span.rel as usize..(span.rel + span.len) as usize];
                self.disk_write(span.file_id, span.file_offset, chunk)?;
            }
        }
        self.shared.write_budget.add(-(g.segs.bytes() as isize));
        g.segs.clear();
        g.dead = true;
        let mut map = self.pieces.lock();
        if map.get(&piece).is_some_and(|e| Arc::ptr_eq(e, entry)) {
            map.remove(&piece);
        }
        Ok(())
    }

    /// Called once a piece has passed its hash check.
    pub fn piece_completed(&self, piece: u32) -> anyhow::Result<()> {
        let Some(e) = self.entry(piece, false) else {
            return Ok(());
        };
        let t = std::time::Instant::now();
        self.flush_piece(piece, &e)?;
        let m = &self.shared.metrics;
        m.pieces_flushed.fetch_add(1, Ordering::Relaxed);
        m.flush_nanos
            .fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Flush least recently touched pieces until we are back under budget.
    fn spill(&self, keep: u32) -> anyhow::Result<()> {
        while self.shared.write_budget.over() {
            let victim = {
                let map = self.pieces.lock();
                map.iter()
                    .filter(|(p, _)| **p != keep)
                    .min_by_key(|(_, e)| e.touched.load(Ordering::Relaxed))
                    .map(|(p, e)| (*p, e.clone()))
            };
            let Some((piece, entry)) = victim else { break };
            debug!(store = self.id, piece, "spilling unfinished piece to disk");
            self.shared.metrics.spills.fetch_add(1, Ordering::Relaxed);
            self.flush_piece(piece, &entry)?;
        }
        Ok(())
    }

    /// Flush everything buffered and fsync (pause, shutdown, before a move).
    pub fn flush_all(&self) -> anyhow::Result<()> {
        let entries: Vec<(u32, Arc<PieceEntry>)> = self
            .pieces
            .lock()
            .iter()
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        for (p, e) in entries {
            self.flush_piece(p, &e)?;
        }
        self.files.sync_all()?;
        Ok(())
    }

    fn disk_write(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let f = self
            .files
            .get(file_id, true)
            .with_context(|| format!("opening {:?}", self.files.path(file_id)))?;
        write_all_at(&f, buf, offset)
            .with_context(|| format!("writing {:?}", self.files.path(file_id)))?;
        self.shared
            .metrics
            .disk_writes
            .fetch_add(1, Ordering::Relaxed);
        self.shared
            .metrics
            .bytes_written
            .fetch_add(buf.len() as u64, Ordering::Relaxed);
        self.invalidate(file_id, offset, buf.len() as u64);
        Ok(())
    }

    // ---------- read path ----------

    pub fn read(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        let slot = self.check_bounds(file_id, offset, buf.len())?;
        if slot.padding {
            buf.fill(0);
            return Ok(());
        }
        // Seeding fast path: nothing buffered, so no per-piece lookups needed.
        if self.pieces.lock().is_empty() {
            return self.read_cached(file_id, offset, buf);
        }
        let abs = slot.offset + offset;
        let mut done = 0usize;
        while done < buf.len() {
            let a = abs + done as u64;
            let piece = self.layout.piece_of(a);
            let (pstart, plen) = self.layout.piece_bounds(piece);
            let n = ((pstart + plen - a) as usize).min(buf.len() - done);
            self.read_in_piece(
                file_id,
                offset + done as u64,
                piece,
                a - pstart,
                &mut buf[done..done + n],
            )?;
            done += n;
        }
        Ok(())
    }

    fn read_in_piece(
        &self,
        file_id: usize,
        offset: u64,
        piece: u32,
        rel: u64,
        buf: &mut [u8],
    ) -> anyhow::Result<()> {
        loop {
            let Some(entry) = self.entry(piece, false) else {
                return self.read_cached(file_id, offset, buf);
            };
            let g = entry.buf.lock();
            if g.dead {
                continue; // flushed meanwhile: data is now on disk
            }
            if !g.segs.covers(rel, buf.len() as u64) {
                // Partly on disk (spilled earlier or pre-existing data). Hold the
                // lock so a concurrent spill cannot slip between the two reads.
                self.disk_read(file_id, offset, buf)?;
            }
            g.segs.overlay(rel, buf);
            return Ok(());
        }
    }

    fn read_cached(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        if !self.shared.read_cache.enabled() {
            return self.disk_read(file_id, offset, buf);
        }
        let file_len = self.layout.files[file_id].len;
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let extent = pos / self.extent;
            let ext_start = extent * self.extent;
            let ext_len = self.extent.min(file_len - ext_start);
            let in_ext = (pos - ext_start) as usize;
            let n = (ext_len as usize - in_ext).min(buf.len() - done);
            let key = Key {
                store: self.id,
                file: file_id as u32,
                extent,
            };
            let data = match self.shared.read_cache.get(&key) {
                Some(d) => d,
                None => {
                    let generation = self.generation.load(Ordering::Acquire);
                    let mut v = vec![0u8; ext_len as usize];
                    if let Err(e) = self.disk_read(file_id, ext_start, &mut v) {
                        // e.g. a short file still being preallocated: read only what was asked.
                        debug!("extent read failed ({e:#}), falling back to a direct read");
                        return self.disk_read(file_id, pos, &mut buf[done..]);
                    }
                    let d: Arc<[u8]> = v.into();
                    if self.generation.load(Ordering::Acquire) == generation {
                        self.shared.read_cache.insert(key, d.clone());
                    }
                    // Peers usually walk forward; let the kernel start on the next extent.
                    let next = ext_start + ext_len;
                    if next < file_len {
                        if let Ok(f) = self.files.get(file_id, false) {
                            os::will_need(&f, next, self.extent.min(file_len - next));
                        }
                    }
                    d
                }
            };
            buf[done..done + n].copy_from_slice(&data[in_ext..in_ext + n]);
            done += n;
        }
        Ok(())
    }

    fn disk_read(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        let f = self
            .files
            .get(file_id, false)
            .with_context(|| format!("opening {:?}", self.files.path(file_id)))?;
        read_exact_at(&f, buf, offset)
            .with_context(|| format!("reading {:?}", self.files.path(file_id)))?;
        self.shared
            .metrics
            .disk_reads
            .fetch_add(1, Ordering::Relaxed);
        self.shared
            .metrics
            .bytes_read
            .fetch_add(buf.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    fn invalidate(&self, file_id: usize, offset: u64, len: u64) {
        if len == 0 {
            return;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let first = offset / self.extent;
        let last = (offset + len - 1) / self.extent;
        self.shared
            .read_cache
            .invalidate(self.id, file_id as u32, first..=last);
    }

    // ---------- file management ----------

    pub fn ensure_len(&self, file_id: usize, len: u64) -> anyhow::Result<()> {
        let slot = self.layout.files.get(file_id).context("no such file")?;
        if slot.padding {
            return Ok(());
        }
        ensure!(
            len == slot.len,
            "engine asked for length {len}, metadata says {}",
            slot.len
        );
        let f = self.files.get(file_id, true)?;
        let prealloc = self.shared.config.lock().preallocation;
        let current = f.metadata()?.len();
        match prealloc {
            Preallocation::Sparse => {
                if let Err(e) = os::mark_sparse(&f) {
                    warn!("could not mark {:?} sparse: {e}", self.files.path(file_id));
                }
                if current != len {
                    f.set_len(len)?;
                }
            }
            Preallocation::Full => {
                if current != len {
                    f.set_len(len)?;
                }
                os::allocate(&f, len)?;
            }
        }
        self.invalidate(file_id, 0, len.max(1));
        Ok(())
    }

    /// Delete one file. The engine passes its own idea of the name; it is
    /// ignored in favour of the sanitized path for this file id.
    pub fn remove_file(&self, file_id: usize) -> anyhow::Result<()> {
        let slot = self.layout.files.get(file_id).context("no such file")?;
        self.files.close(file_id);
        self.shared.read_cache.purge_store(self.id);
        if slot.padding {
            return Ok(());
        }
        match std::fs::remove_file(&slot.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Remove empty folders this torrent created. The engine's `path` is not
    /// trusted: an empty path means "the torrent root", which is only removed
    /// when Riptor created it; anything else removes our own empty subfolders.
    pub fn remove_empty_dirs(&self, root_requested: bool) -> anyhow::Result<()> {
        for d in self.layout.subdirs_deepest_first() {
            remove_dir_if_empty(&d);
        }
        if root_requested && self.layout.owns_root {
            remove_dir_if_empty(&self.layout.root);
        }
        Ok(())
    }

    /// Flush and release file handles (torrent paused).
    pub fn release(&self) -> anyhow::Result<()> {
        self.flush_all()?;
        self.files.close_all();
        self.shared.read_cache.purge_store(self.id);
        Ok(())
    }

    pub fn buffered_pieces(&self) -> usize {
        self.pieces.lock().len()
    }
}

fn remove_dir_if_empty(d: &std::path::Path) {
    if let Ok(mut it) = std::fs::read_dir(d) {
        if it.next().is_none() {
            if let Err(e) = std::fs::remove_dir(d) {
                warn!("could not remove empty folder {d:?}: {e}");
            }
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Err(e) = self.flush_all() {
            warn!("flushing torrent data on drop failed: {e:#}");
        }
        self.shared.read_cache.purge_store(self.id);
    }
}
