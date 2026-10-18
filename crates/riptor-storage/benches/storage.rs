//! Compares Riptor's cached store against plain positional file I/O (what
//! librqbit's FilesystemStorage does) for the two hot paths:
//!
//! * download: 16 KiB blocks arrive, the piece is hash-read back, then committed
//! * seeding: peers read pieces back as 16 KiB blocks
//!
//! Run with `cargo bench -p riptor-storage`.

use std::borrow::Cow;
use std::fs::OpenOptions;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use rand::seq::SliceRandom;
use riptor_storage::files::{read_exact_at, write_all_at};
use riptor_storage::layout::{Layout, RawFile};
use riptor_storage::{Shared, StorageConfig, Store};

const PIECE: u64 = 1024 * 1024;
const PIECES: u64 = 64;
const BLOCK: u64 = 16 * 1024;

fn layout(root: &std::path::Path) -> Layout {
    Layout::new(
        root,
        true,
        PIECE as u32,
        [RawFile {
            components: vec![Cow::Borrowed("data.bin")],
            len: PIECE * PIECES,
            padding: false,
        }],
    )
    .unwrap()
}

/// Blocks arrive shuffled within a window of in-flight pieces, roughly one
/// piece per active peer, which is how librqbit requests them.
const IN_FLIGHT: u64 = 16;

fn block_order() -> Vec<(u64, u64)> {
    let mut rng = rand::thread_rng();
    let mut out = Vec::new();
    for w in (0..PIECES).step_by(IN_FLIGHT as usize) {
        let mut v: Vec<(u64, u64)> = (w..(w + IN_FLIGHT).min(PIECES))
            .flat_map(|p| (0..PIECE / BLOCK).map(move |b| (p, b)))
            .collect();
        v.shuffle(&mut rng);
        out.extend(v);
    }
    out
}

fn download(c: &mut Criterion) {
    let block = vec![7u8; BLOCK as usize];
    let mut g = c.benchmark_group("download_64MiB");
    g.throughput(Throughput::Bytes(PIECE * PIECES));
    g.sample_size(10);

    g.bench_function("plain_file", |b| {
        b.iter_batched(
            || (tempfile::tempdir().unwrap(), block_order()),
            |(dir, order)| {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(dir.path().join("d"))
                    .unwrap();
                let mut left = vec![PIECE / BLOCK; PIECES as usize];
                let mut buf = vec![0u8; 64 * 1024];
                for (p, blk) in order {
                    write_all_at(&f, &block, p * PIECE + blk * BLOCK).unwrap();
                    left[p as usize] -= 1;
                    if left[p as usize] == 0 {
                        // hash check reads the piece back from disk
                        for off in (0..PIECE).step_by(buf.len()) {
                            read_exact_at(&f, &mut buf, p * PIECE + off).unwrap();
                        }
                    }
                }
                (dir, f) // dropped outside the timed region
            },
            BatchSize::PerIteration,
        )
    });

    for (name, write_buffer) in [
        ("riptor_store", 64usize << 20),
        ("riptor_store_write_through", 0),
    ] {
        g.bench_function(name, |b| {
            b.iter_batched(
                || (tempfile::tempdir().unwrap(), block_order()),
                |(dir, order)| {
                    let cfg = StorageConfig {
                        write_buffer_bytes: write_buffer,
                        ..StorageConfig::default()
                    };
                    let s = Store::new(layout(dir.path()), Shared::new(cfg));
                    let mut left = vec![PIECE / BLOCK; PIECES as usize];
                    let mut buf = vec![0u8; 64 * 1024];
                    for (p, blk) in order {
                        s.write(0, p * PIECE + blk * BLOCK, &[&block]).unwrap();
                        left[p as usize] -= 1;
                        if left[p as usize] == 0 {
                            for off in (0..PIECE).step_by(buf.len()) {
                                s.read(0, p * PIECE + off, &mut buf).unwrap();
                            }
                            s.piece_completed(p as u32).unwrap();
                        }
                    }
                    // Returned so the final flush + fsync on drop is not timed; plain I/O never fsyncs here either.
                    (dir, s)
                },
                BatchSize::PerIteration,
            )
        });
    }
    g.finish();
}

fn seeding(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let l = layout(dir.path());
    std::fs::write(&l.files[0].path, vec![3u8; (PIECE * PIECES) as usize]).unwrap();
    // Several peers interleave whole-piece requests.
    let mut pieces: Vec<u64> = (0..PIECES).collect();
    pieces.shuffle(&mut rand::thread_rng());

    let mut g = c.benchmark_group("seed_64MiB");
    g.throughput(Throughput::Bytes(PIECE * PIECES));
    g.sample_size(10);

    g.bench_function("plain_file", |b| {
        let f = std::fs::File::open(&l.files[0].path).unwrap();
        let mut buf = vec![0u8; BLOCK as usize];
        b.iter(|| {
            for p in &pieces {
                for blk in 0..PIECE / BLOCK {
                    read_exact_at(&f, &mut buf, p * PIECE + blk * BLOCK).unwrap();
                }
            }
        })
    });

    g.bench_function("riptor_store", |b| {
        let s = Store::new(l.clone(), Shared::new(StorageConfig::default()));
        let mut buf = vec![0u8; BLOCK as usize];
        b.iter(|| {
            for p in &pieces {
                for blk in 0..PIECE / BLOCK {
                    s.read(0, p * PIECE + blk * BLOCK, &mut buf).unwrap();
                }
            }
        })
    });
    g.finish();
}

criterion_group!(benches, download, seeding);
criterion_main!(benches);
