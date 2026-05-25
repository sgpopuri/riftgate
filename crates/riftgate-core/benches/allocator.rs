//! Microbenchmarks for [`riftgate_core::allocator`].
//!
//! Backs the per-request hot-path claim that a `BumpArena` is
//! ~5–15 ns per allocation versus 50–200 ns for `std::alloc::System`,
//! and that `BumpArena::reset` returns the bump pointer in O(1)
//! regardless of how much was allocated.
//!
//! See [Options 005](../../../docs/05-options/005-allocator.md) for
//! the design rationale and [`docs/04-design/lld-allocator.md`](../../../docs/04-design/lld-allocator.md)
//! for the per-request memory cap policy that the per-shard pool
//! enforces above this layer.

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate_core::allocator::{Allocator, BumpArena, SystemAllocator};
use std::alloc::Layout;
use std::hint::black_box;

fn bench_arena_alloc_value(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocator/arena_alloc_value");
    for n in [16usize, 256, 4096] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let arena = BumpArena::with_capacity(64 * 1024);
            b.iter(|| {
                let layout = Layout::array::<u8>(n).expect("valid layout");
                let p = arena.alloc(layout);
                black_box(p);
            });
        });
    }
    group.finish();
}

fn bench_system_alloc_value(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocator/system_alloc_value");
    for n in [16usize, 256, 4096] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let alloc = SystemAllocator::new();
            b.iter(|| {
                let layout = Layout::array::<u8>(n).expect("valid layout");
                let p = alloc.alloc(layout);
                // SAFETY: alloc returned a non-null pointer for `layout`;
                // we deallocate immediately so the bench stays leak-free.
                #[allow(unsafe_code)]
                unsafe {
                    std::alloc::dealloc(p, layout);
                }
                black_box(());
            });
        });
    }
    group.finish();
}

fn bench_arena_reset(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocator/arena_reset");
    for n in [128usize, 4096] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut arena = BumpArena::with_capacity(64 * 1024);
            b.iter(|| {
                for _ in 0..n {
                    let layout = Layout::array::<u8>(64).expect("valid layout");
                    let p = arena.alloc(layout);
                    black_box(p);
                }
                Allocator::reset(&mut arena);
            });
        });
    }
    group.finish();
}

mod harness {
    use super::{bench_arena_alloc_value, bench_arena_reset, bench_system_alloc_value};
    use criterion::criterion_group;
    criterion_group!(
        allocator,
        bench_arena_alloc_value,
        bench_system_alloc_value,
        bench_arena_reset,
    );
}
criterion_main!(harness::allocator);
