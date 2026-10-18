use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use proptest::prelude::*;
use riptor_storage::layout::{Layout, RawFile};
use riptor_storage::{Preallocation, Shared, StorageConfig, Store};

fn layout(root: &Path, owns_root: bool, piece_len: u32, files: &[(&str, u64)]) -> Layout {
    let raw = files.iter().map(|(n, len)| RawFile {
        components: n.split('/').map(|s| Cow::Owned(s.to_string())).collect(),
        len: *len,
        padding: n.contains(".pad/"),
    });
    Layout::new(root, owns_root, piece_len, raw).unwrap()
}

fn cfg(read: usize, write: usize) -> StorageConfig {
    StorageConfig {
        read_cache_bytes: read,
        write_buffer_bytes: write,
        max_open_files: 2,
        preallocation: Preallocation::Sparse,
    }
}

/// Write a flat range the way librqbit's `write_chunk` does: split per file.
fn write_abs(s: &Store, abs: u64, data: &[u8]) {
    let spans: Vec<_> = s.layout().spans(abs, data.len() as u64).collect();
    for sp in spans {
        let chunk = &data[sp.rel as usize..(sp.rel + sp.len) as usize];
        s.write(sp.file_id, sp.file_offset, &[chunk]).unwrap();
    }
}

fn read_abs(s: &Store, abs: u64, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    let spans: Vec<_> = s.layout().spans(abs, len as u64).collect();
    for sp in spans {
        s.read(
            sp.file_id,
            sp.file_offset,
            &mut out[sp.rel as usize..(sp.rel + sp.len) as usize],
        )
        .unwrap();
    }
    out
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn disk_bytes(l: &Layout) -> Vec<u8> {
    let mut out = Vec::new();
    for f in &l.files {
        if f.padding {
            out.extend(std::iter::repeat_n(0, f.len as usize));
            continue;
        }
        let mut d = std::fs::read(&f.path).unwrap_or_default();
        d.resize(f.len as usize, 0);
        out.extend(d);
    }
    out
}

const FILES: &[(&str, u64)] = &[("dir/a.bin", 10), ("dir/sub/b.bin", 25), ("c.bin", 13)];

#[test]
fn roundtrip_through_buffer_and_flush() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path(), true, 16, FILES);
    let shared = Shared::new(cfg(1 << 20, 1 << 20));
    let s = Store::new(l.clone(), shared.clone());
    let data = pattern(48, 7);

    // Write in 4-byte "blocks", out of order.
    for i in (0..12).rev() {
        write_abs(&s, i * 4, &data[i as usize * 4..i as usize * 4 + 4]);
    }
    // Hash check reads come straight from memory; nothing on disk yet.
    assert_eq!(read_abs(&s, 0, 48), data);
    assert!(!l.files[0].path.exists());
    assert_eq!(s.buffered_pieces(), 3);

    for p in 0..3 {
        s.piece_completed(p).unwrap();
    }
    assert_eq!(s.buffered_pieces(), 0);
    assert_eq!(shared.write_budget.used(), 0);
    assert_eq!(disk_bytes(&l), data);
    assert_eq!(read_abs(&s, 0, 48), data);
}

#[test]
fn spills_under_memory_pressure_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path(), true, 16, FILES);
    let shared = Shared::new(cfg(0, 20));
    let s = Store::new(l.clone(), shared.clone());
    let data = pattern(48, 3);
    for i in 0..12u64 {
        write_abs(&s, i * 4, &data[i as usize * 4..i as usize * 4 + 4]);
        // Budget is enforced by spilling everything but the piece being written.
        assert!(
            shared.write_budget.used() <= 20 + 16,
            "used {}",
            shared.write_budget.used()
        );
        assert_eq!(
            read_abs(&s, 0, (i as usize + 1) * 4),
            data[..(i as usize + 1) * 4]
        );
    }
    s.flush_all().unwrap();
    assert_eq!(disk_bytes(&l), data);
}

#[test]
fn write_through_when_buffering_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path(), true, 16, FILES);
    let s = Store::new(l.clone(), Shared::new(cfg(0, 0)));
    let data = pattern(48, 9);
    write_abs(&s, 0, &data);
    assert_eq!(s.buffered_pieces(), 0);
    assert_eq!(disk_bytes(&l), data);
}

#[test]
fn bounds_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::new(
        layout(dir.path(), true, 16, FILES),
        Shared::new(cfg(1024, 1024)),
    );
    assert!(s.write(0, 8, &[&[1, 2, 3]]).is_err());
    assert!(s.write(9, 0, &[&[1]]).is_err());
    assert!(s.write(0, u64::MAX, &[&[1]]).is_err());
    let mut b = [0u8; 4];
    assert!(s.read(0, 8, &mut b).is_err());
}

#[test]
fn padding_files_are_never_created() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path(), true, 8, &[("a", 5), (".pad/3", 3), ("b", 8)]);
    let s = Store::new(l.clone(), Shared::new(cfg(1024, 1024)));
    write_abs(&s, 0, &pattern(16, 1));
    s.flush_all().unwrap();
    s.ensure_len(1, 3).unwrap();
    assert!(!l.files[1].path.exists());
    let mut b = [9u8; 3];
    s.read(1, 0, &mut b).unwrap();
    assert_eq!(b, [0, 0, 0]);
}

#[test]
fn read_cache_serves_seeding_and_invalidates_on_write() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path(), true, 64 * 1024, &[("seed.bin", 64 * 1024 * 4)]);
    let shared = Shared::new(cfg(1 << 20, 1 << 20));
    let s = Store::new(l.clone(), shared.clone());
    let data = pattern(64 * 1024 * 4, 5);
    std::fs::write(&l.files[0].path, &data).unwrap();

    // A peer reads the first extent as 16 KiB blocks: one miss, then hits.
    for i in 0..4u64 {
        let mut b = vec![0u8; 16 * 1024];
        s.read(0, i * 16 * 1024, &mut b).unwrap();
        assert_eq!(
            b,
            data[(i * 16 * 1024) as usize..((i + 1) * 16 * 1024) as usize]
        );
    }
    let st = shared.read_cache.stats();
    assert_eq!((st.misses, st.hits), (1, 3));

    // A write (e.g. re-download after a failed recheck) must not leave stale cache.
    write_abs(&s, 100, &[0xAA; 4]);
    s.piece_completed(s.layout().piece_of(100)).unwrap();
    let mut b = [0u8; 4];
    s.read(0, 100, &mut b).unwrap();
    assert_eq!(b, [0xAA; 4]);
}

#[test]
fn ensure_len_preallocates_sparse_and_full() {
    let dir = tempfile::tempdir().unwrap();
    for mode in [Preallocation::Sparse, Preallocation::Full] {
        let root = dir.path().join(format!("{mode:?}"));
        let l = layout(&root, true, 16, &[("x/big.bin", 10 * 1024 * 1024)]);
        let shared = Shared::new(StorageConfig {
            preallocation: mode,
            ..cfg(0, 0)
        });
        let s = Store::new(l.clone(), shared);
        s.ensure_len(0, 10 * 1024 * 1024).unwrap();
        assert_eq!(
            std::fs::metadata(&l.files[0].path).unwrap().len(),
            10 * 1024 * 1024
        );
        // The engine may only ask for the length the metadata declares.
        assert!(s.ensure_len(0, 1).is_err());
    }
}

#[test]
fn folder_removal_never_touches_user_folders() {
    let dir = tempfile::tempdir().unwrap();
    // Single-file torrent saved straight into a user folder: root not owned.
    let user = dir.path().join("Movies");
    std::fs::create_dir_all(&user).unwrap();
    let s = Store::new(
        layout(&user, false, 16, &[("film.mkv", 4)]),
        Shared::new(cfg(0, 0)),
    );
    write_abs(&s, 0, &[1, 2, 3, 4]);
    s.remove_file(0).unwrap();
    s.remove_empty_dirs(true).unwrap();
    assert!(user.exists(), "user's folder must survive");

    // Multi-file torrent in its own folder: subfolders and root go.
    let root = dir.path().join("Some.Torrent");
    let l = layout(&root, true, 16, &[("a/b/c.txt", 2), ("d.txt", 2)]);
    let s = Store::new(l.clone(), Shared::new(cfg(0, 0)));
    write_abs(&s, 0, &[1, 2, 3, 4]);
    s.remove_file(0).unwrap();
    s.remove_file(1).unwrap();
    // The engine passes its own (untrusted) names; a non-empty path only clears subfolders.
    s.remove_empty_dirs(false).unwrap();
    assert!(root.exists() && !root.join("a").exists());
    s.remove_empty_dirs(true).unwrap();
    assert!(!root.exists());
}

#[test]
fn concurrent_writers_and_readers() {
    let dir = tempfile::tempdir().unwrap();
    let piece = 256u64;
    let total = piece * 64;
    let l = layout(
        dir.path(),
        true,
        piece as u32,
        &[
            ("x/1", total / 3),
            ("x/2", total / 3),
            ("x/3", total - 2 * (total / 3)),
        ],
    );
    let shared = Shared::new(cfg(4096, 2048));
    let s = Arc::new(Store::new(l.clone(), shared));
    let data = Arc::new(pattern(total as usize, 11));

    std::thread::scope(|sc| {
        for t in 0..8u64 {
            let s = s.clone();
            let data = data.clone();
            sc.spawn(move || {
                for p in (t..64).step_by(8) {
                    for blk in 0..4 {
                        let a = p * piece + blk * 64;
                        write_abs(&s, a, &data[a as usize..a as usize + 64]);
                    }
                    let got = read_abs(&s, p * piece, piece as usize);
                    assert_eq!(
                        got,
                        data[(p * piece) as usize..((p + 1) * piece) as usize],
                        "piece {p}"
                    );
                    s.piece_completed(p as u32).unwrap();
                }
            });
        }
    });
    assert_eq!(read_abs(&s, 0, total as usize), *data);
    s.flush_all().unwrap();
    assert_eq!(disk_bytes(&l), *data);
}

#[derive(Debug, Clone)]
enum Op {
    Write { abs: u64, len: usize, seed: u8 },
    Read { abs: u64, len: usize },
    Complete { piece: u32 },
    FlushAll,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0u64..48, 1usize..12, any::<u8>()).prop_map(|(abs, len, seed)| Op::Write { abs, len, seed }),
        3 => (0u64..48, 1usize..20).prop_map(|(abs, len)| Op::Read { abs, len }),
        1 => (0u32..3).prop_map(|piece| Op::Complete { piece }),
        1 => Just(Op::FlushAll),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any interleaving of writes, reads, completions and flushes behaves like
    /// a flat byte array, whatever the cache and buffer sizes.
    #[test]
    fn behaves_like_flat_array(ops in proptest::collection::vec(op(), 1..80), read_cap in 0usize..200, write_cap in 0usize..40) {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path(), true, 16, FILES);
        let s = Store::new(l.clone(), Shared::new(cfg(read_cap, write_cap)));
        let mut model = vec![0u8; 48];
        // Files are created on first write, so give reads something to hit.
        write_abs(&s, 0, &model);
        s.flush_all().unwrap();
        for op in ops {
            match op {
                Op::Write { abs, len, seed } => {
                    // Like the engine, never cross a piece boundary in one write.
                    let len = len.min((16 - abs % 16) as usize).min(48 - abs as usize);
                    let d = pattern(len, seed);
                    write_abs(&s, abs, &d);
                    model[abs as usize..abs as usize + len].copy_from_slice(&d);
                }
                Op::Read { abs, len } => {
                    let len = len.min(48 - abs as usize);
                    prop_assert_eq!(read_abs(&s, abs, len), &model[abs as usize..abs as usize + len]);
                }
                Op::Complete { piece } => s.piece_completed(piece).unwrap(),
                Op::FlushAll => s.flush_all().unwrap(),
            }
        }
        s.flush_all().unwrap();
        prop_assert_eq!(disk_bytes(&l), model);
    }
}
