//! Microbenchmarks for [`riftgate_replay::FileWal`] append paths.

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate_core::wal::{Durability, WAL};
use riftgate_replay::{FileWal, FileWalConfig};
use std::hint::black_box;
use std::time::Duration;

fn bench_file_wal_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("wal/append");
    for payload_len in [128_usize, 1024_usize, 8192_usize] {
        group.bench_with_input(
            BenchmarkId::new("async", payload_len),
            &payload_len,
            |b, &len| {
                let dir = tempfile::tempdir().expect("tempdir");
                let wal = FileWal::open(FileWalConfig {
                    root: dir.path().to_path_buf(),
                    shards: 2,
                    segment_size_max: 16 * 1024 * 1024,
                    flush_interval: Duration::from_millis(5),
                    flush_buffer_bytes: 256 * 1024,
                })
                .expect("open wal");
                let payload = vec![0xA5_u8; len];
                b.iter(|| {
                    black_box(
                        wal.append(&payload, Durability::Async)
                            .expect("append async"),
                    );
                });
                wal.shutdown();
            },
        );

        group.bench_with_input(
            BenchmarkId::new("fdatasync", payload_len),
            &payload_len,
            |b, &len| {
                let dir = tempfile::tempdir().expect("tempdir");
                let wal = FileWal::open(FileWalConfig {
                    root: dir.path().to_path_buf(),
                    shards: 2,
                    segment_size_max: 16 * 1024 * 1024,
                    flush_interval: Duration::from_millis(5),
                    flush_buffer_bytes: 256 * 1024,
                })
                .expect("open wal");
                let payload = vec![0x5A_u8; len];
                b.iter(|| {
                    black_box(
                        wal.append(&payload, Durability::FdataSync)
                            .expect("append durable"),
                    );
                });
                wal.shutdown();
            },
        );
    }
    group.finish();
}

mod harness {
    use super::bench_file_wal_append;
    use criterion::criterion_group;
    criterion_group!(wal, bench_file_wal_append);
}
criterion_main!(harness::wal);
