//! Re-reads finished data in the background and checks every piece's hash, at a gentle pace.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use riptor_storage::layout::Layout;
use sha1::{Digest, Sha1};

const MAX_BAD: usize = 1000;

pub struct Plan {
    pub layout: Layout,
    /// 20 bytes per piece.
    pub hashes: Vec<u8>,
    pub have: Vec<bool>,
    /// Per file: downloaded at all (unselected files may be missing).
    pub wanted: Vec<bool>,
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Plan {
    /// Pieces we have and whose every byte belongs to a wanted file (or padding).
    fn checkable(&self, piece: u32) -> bool {
        if !self.have.get(piece as usize).copied().unwrap_or(false) {
            return false;
        }
        let (start, len) = self.layout.piece_bounds(piece);
        self.layout.spans(start, len).all(|s| {
            self.layout.files[s.file_id].padding
                || self.wanted.get(s.file_id).copied().unwrap_or(false)
        })
    }
}

/// Opens one file at a time; pieces go in order, so this is nearly always a hit.
struct Reader {
    open: Option<(usize, File)>,
}

impl Reader {
    fn read(
        &mut self,
        layout: &Layout,
        file_id: usize,
        offset: u64,
        out: &mut [u8],
    ) -> std::io::Result<()> {
        if self.open.as_ref().is_none_or(|(id, _)| *id != file_id) {
            let path: &PathBuf = &layout.files[file_id].path;
            self.open = Some((file_id, File::open(path)?));
        }
        let (_, f) = self.open.as_mut().expect("just opened");
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(out)
    }
}

/// The pieces that no longer match. A missing or short file counts as bad pieces, not an error.
pub fn run(plan: &Plan, progress: &Progress, bytes_per_sec: u64) -> anyhow::Result<Vec<u32>> {
    let pieces = plan.hashes.len() / 20;
    anyhow::ensure!(
        pieces as u64 == plan.layout.total_len.div_ceil(plan.layout.piece_len),
        "piece count doesn't match the torrent"
    );
    let todo: Vec<u32> = (0..pieces as u32).filter(|p| plan.checkable(*p)).collect();
    progress.total.store(
        todo.iter().map(|p| plan.layout.piece_bounds(*p).1).sum(),
        Ordering::Relaxed,
    );
    let started = Instant::now();
    let mut reader = Reader { open: None };
    let mut buf = Vec::new();
    let mut bad = Vec::new();
    let mut read_total = 0u64;
    for p in todo {
        if progress.cancel.load(Ordering::Relaxed) {
            anyhow::bail!("stopped");
        }
        let (start, len) = plan.layout.piece_bounds(p);
        buf.clear();
        buf.resize(len as usize, 0);
        let mut ok = true;
        for s in plan.layout.spans(start, len) {
            if plan.layout.files[s.file_id].padding {
                continue;
            }
            let dst = &mut buf[s.rel as usize..(s.rel + s.len) as usize];
            if reader
                .read(&plan.layout, s.file_id, s.file_offset, dst)
                .is_err()
            {
                ok = false;
                break;
            }
        }
        let expected = &plan.hashes[p as usize * 20..p as usize * 20 + 20];
        if (!ok || Sha1::digest(&buf).as_slice() != expected) && bad.len() < MAX_BAD {
            bad.push(p);
        }
        read_total += len;
        progress.done.fetch_add(len, Ordering::Relaxed);
        // Stay under the rate so seeding and other programs keep the disk.
        if bytes_per_sec > 0 {
            let due = Duration::from_secs_f64(read_total as f64 / bytes_per_sec as f64);
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
        }
    }
    Ok(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use riptor_storage::layout::RawFile;

    fn plan(dir: &std::path::Path, files: &[(&str, &[u8], bool)], piece: u32) -> Plan {
        let raw: Vec<RawFile> = files
            .iter()
            .map(|(n, b, pad)| RawFile {
                components: vec![std::borrow::Cow::Owned(n.to_string())],
                len: b.len() as u64,
                padding: *pad,
            })
            .collect();
        let layout = Layout::new(dir, false, piece, raw).unwrap();
        let mut all = Vec::new();
        for (n, b, pad) in files {
            if !pad {
                std::fs::write(dir.join(n), b).unwrap();
            }
            all.extend_from_slice(b);
        }
        let hashes: Vec<u8> = all
            .chunks(piece as usize)
            .flat_map(|c| Sha1::digest(c).to_vec())
            .collect();
        let pieces = hashes.len() / 20;
        Plan {
            layout,
            hashes,
            have: vec![true; pieces],
            wanted: vec![true; files.len()],
        }
    }

    #[test]
    fn rotted_and_missing_data_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let a = vec![1u8; 100];
        let b = vec![2u8; 150];
        let p = plan(
            dir.path(),
            &[("a", &a, false), ("pad", &[0u8; 6], true), ("b", &b, false)],
            64,
        );
        let progress = Progress::default();
        assert_eq!(run(&p, &progress, 0).unwrap(), Vec::<u32>::new());
        assert_eq!(progress.done.load(Ordering::Relaxed), 256);

        // One flipped byte in b (offset 106 + 10 = 116, piece 1).
        let mut rotten = b.clone();
        rotten[10] ^= 0xff;
        std::fs::write(dir.path().join("b"), &rotten).unwrap();
        assert_eq!(run(&p, &Progress::default(), 0).unwrap(), vec![1]);

        // A missing file: every piece it touches is bad.
        std::fs::remove_file(dir.path().join("a")).unwrap();
        assert_eq!(run(&p, &Progress::default(), 0).unwrap(), vec![0, 1]);
    }

    #[test]
    fn only_wanted_pieces_we_have_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = plan(
            dir.path(),
            &[("a", &[1u8; 64], false), ("b", &[2u8; 64], false)],
            64,
        );
        std::fs::remove_file(dir.path().join("b")).unwrap();
        p.wanted[1] = false;
        assert!(run(&p, &Progress::default(), 0).unwrap().is_empty());
        p.wanted[1] = true;
        p.have[1] = false;
        assert!(run(&p, &Progress::default(), 0).unwrap().is_empty());
    }

    #[test]
    fn it_can_be_stopped_and_rate_limited() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(dir.path(), &[("a", &[1u8; 4096], false)], 1024);
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        assert!(run(&p, &progress, 0).is_err());
        let t = Instant::now();
        run(&p, &Progress::default(), 16 * 1024).unwrap();
        assert!(t.elapsed() >= Duration::from_millis(200));
    }

    #[test]
    fn mismatched_piece_counts_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = plan(dir.path(), &[("a", &[1u8; 100], false)], 64);
        p.hashes.truncate(20);
        assert!(run(&p, &Progress::default(), 0).is_err());
    }
}
