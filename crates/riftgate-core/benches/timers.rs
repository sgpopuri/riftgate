//! Microbenchmarks for [`riftgate_core::timers`].
//!
//! These benchmarks back the [`FR-008`](../../../docs/01-requirements/functional.md)
//! acceptance bar:
//!
//! > 100k concurrent timers cost less than `O(n)` per tick.
//!
//! On a `BinaryHeapTimers` the per-tick cost is `O(k log n)` where `k`
//! is the number of expired timers. With every deadline pushed into the
//! distant future, `k = 0` and the tick cost collapses to a single
//! `peek` plus a comparison — single-digit microseconds at 100k.
//!
//! The bench groups also profile `schedule` and `cancel` so a future
//! hierarchical-wheel implementation has a baseline to regress against.

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate_core::timers::{BinaryHeapTimers, TimerHandle, TimerSubsystem};
use std::time::{Duration, Instant};

fn bench_tick_no_expirations(c: &mut Criterion) {
    let mut group = c.benchmark_group("timers/tick_no_expirations");
    for size in [1_000usize, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let mut timers = BinaryHeapTimers::new();
            let now = Instant::now();
            for i in 0..size {
                let _ =
                    timers.schedule(now + Duration::from_secs(3600 + i as u64), Box::new(|| {}));
            }
            b.iter(|| {
                timers.tick(now);
            });
        });
    }
    group.finish();
}

fn bench_schedule(c: &mut Criterion) {
    let mut group = c.benchmark_group("timers/schedule");
    for size in [1_000usize, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            // Pre-populate; measure the cost of inserting one more.
            let mut timers = BinaryHeapTimers::new();
            let now = Instant::now();
            for i in 0..size {
                let _ =
                    timers.schedule(now + Duration::from_secs(3600 + i as u64), Box::new(|| {}));
            }
            let mut next: u64 = size as u64;
            b.iter(|| {
                let _ = timers.schedule(now + Duration::from_secs(7200 + next), Box::new(|| {}));
                next += 1;
            });
        });
    }
    group.finish();
}

fn bench_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("timers/cancel");
    for size in [1_000usize, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            // Schedule `size` timers and stash their handles; cancel
            // them one by one.
            let mut timers = BinaryHeapTimers::new();
            let now = Instant::now();
            let mut handles: Vec<TimerHandle> = Vec::with_capacity(size);
            for i in 0..size {
                handles.push(
                    timers.schedule(now + Duration::from_secs(3600 + i as u64), Box::new(|| {})),
                );
            }
            let mut idx = 0usize;
            b.iter(|| {
                if idx < handles.len() {
                    let _ = timers.cancel(handles[idx]);
                    idx += 1;
                }
            });
        });
    }
    group.finish();
}

mod harness {
    use super::{bench_cancel, bench_schedule, bench_tick_no_expirations};
    use criterion::criterion_group;
    criterion_group!(
        timers,
        bench_tick_no_expirations,
        bench_schedule,
        bench_cancel
    );
}
criterion_main!(harness::timers);
