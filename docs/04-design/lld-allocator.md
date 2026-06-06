# 04.e LLD — Allocator

> Per-request arena allocator. Eliminates `malloc` from the hot path. Returns all per-request memory in O(1) on completion.
>
> Status: **shipped (v0.1)**. `BumpArena` (bumpalo-backed) is the v0.1 default; `SystemAllocator` is retained as the safe baseline.

## Purpose

Make memory allocation cost predictable in the request lifecycle. The general-purpose `malloc` is fine; per-request, scoped allocation is faster and tail-latency-cleaner.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/allocator.rs`](../../crates/riftgate-core/src/allocator.rs):

```rust
pub trait Allocator {
    fn alloc(&self, layout: Layout) -> *mut u8;
    fn reset(&mut self);
    fn capacity(&self) -> usize;
    fn allocated(&self) -> usize;
}
```

The two design adjustments from the v0.0 outline:

- **`reset` takes `&mut self`** because bumpalo's `Bump::reset` requires unique access. Per-request ownership ([ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md)) — each in-flight request owns its arena exclusively — makes this trivial; sharing arenas is a misuse.
- **`capacity()` and `allocated()`** observability methods are part of the trait so the per-request observability span can record arena pressure without reaching into impl-specific state. Both are O(1).

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `BumpArena` | shipped (v0.1, default) | `riftgate-core` | Bumpalo-backed bump allocator. O(1) `alloc` / O(1) `reset`. Initial 4 KB chunk; chunks double up to a 1 MB cap, configurable via `BumpArena::with_initial_capacity`. |
| `SystemAllocator` | shipped (v0.1) | `riftgate-core` | Wraps `std::alloc::System`. Retained for non-hot-path data structures and for differential testing. Used wherever the lifetime of the allocation outlives the request scope. |
| `MimallocGlobal` | v0.2 (opt-in) | (link in the `mimalloc` crate) | Replaces the global allocator for `malloc` calls outside the arena. Behind a Cargo feature flag, never on by default. |

Decision rationale: [Options 005 (allocator)](../05-options/005-allocator.md).

Foundational principles: bump-pointer arena allocators (Postgres memory contexts, LLVM/rustc per-pass arenas) and modern multi-thread `malloc` implementations (`jemalloc`, `mimalloc`, `tcmalloc`; Berger Hoard paper for the underlying multithread-allocator design).

## Component context

### Architecture and dependencies

The allocator is owned by the per-request handler in `crates/riftgate/src/proxy.rs`: a fresh `BumpArena` is constructed at request entry and dropped (or `reset()`-and-returned-to-pool, future work) on completion. All per-request data structures (parser scratch buffers, request-body buffers, filter state) should allocate from the arena.

The arena does not depend on any other Riftgate subsystem. It depends transitively on `bumpalo` for the bump implementation and on `std::alloc::System` for the fallback path.

### Patterns and conventions

- **Arena scope = request scope.** Nothing in the arena outlives the request unless explicitly copied to `SystemAllocator` first.
- **Free is a no-op.** Individual deallocation is a programming error; the arena is reset wholesale.
- **Pre-sized arenas.** Each arena starts at 4 KB and grows by doubling up to a 1 MB cap (configurable). Over-cap requests fall back to system allocation; this is logged but does not fail the request.
- **Per-shard arena pool (decided).** Completed-request arenas are `reset()` and returned to a **per-shard** free-list, not a shared/global pool, per [ADR 0027](../06-adrs/0027-per-shard-bump-arena-pool.md). The pool is bounded: a shard retains up to `arena_pool_max` reset arenas (default 32), and an arena whose high-water capacity exceeded `arena_pool_retain_cap_bytes` (default 64 KiB) is freed rather than pooled so a single large request does not pin a multi-MiB chunk. The recycle path takes no lock and no atomic — the arena is `!Send` and never leaves its shard ([ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md)).
- **`SystemAllocator` for off-path data.** The router's backend table, the config tree, and the observability bus all live in `SystemAllocator` because their lifetime is the gateway, not the request.

### Pitfalls

- **Lifetime confusion.** Borrows from arena memory must not escape the request scope. Compile-time enforced by Rust's borrow checker plus the `&mut self` discipline on `reset`.
- **Unbounded growth.** Without a cap, a single bad request can grow the arena forever. The 1 MB cap is enforced; over-cap allocations spill to `SystemAllocator` so a runaway request does not pin a multi-MB chunk in the pool.
- **Alignment mistakes.** The bump allocator must respect each `Layout`'s alignment; covered by the bumpalo unit tests and by [`crates/riftgate-core/benches/allocator.rs`](../../crates/riftgate-core/benches/allocator.rs).
- **`*mut u8` is unsafe.** Callers must construct slices with the correct length and not alias arena pointers. The Riftgate crates wrap arena-allocated buffers in safe types (`Vec`, `Bytes`) before exposing them.

### Standards and review gates

- Allocator changes must keep [`crates/riftgate-core/benches/allocator.rs`](../../crates/riftgate-core/benches/allocator.rs) green: a fresh `BumpArena` plus several allocations should fit in tens of nanoseconds; full request-arena teardown should remain free.
- The trait surface is part of the v0.1 frozen surface — changes require a new ADR superseding the v0.0 ADR.
- Memory usage after a 1M-request soak must be flat (within noise); the arena pool, when added, must not unbound RSS.

## Testing strategy

- Unit tests in `riftgate-core/src/allocator.rs` cover alignment, capacity, and reset semantics.
- The microbenchmark in [`crates/riftgate-core/benches/allocator.rs`](../../crates/riftgate-core/benches/allocator.rs) measures alloc / reset throughput.
- The end-to-end test in [`crates/riftgate/tests/e2e.rs`](../../crates/riftgate/tests/e2e.rs) exercises the per-request path; v0.2 will add a soak test once arena pooling lands.

## Open questions

- Should the arena pool be per-shard or shared? **Resolved: per-shard**, per [ADR 0027](../06-adrs/0027-per-shard-bump-arena-pool.md) — no cross-core synchronization on the recycle path, bounded by `arena_pool_max` and `arena_pool_retain_cap_bytes`; the accepted cost is slightly higher idle RSS.
- Should we support per-request arena tracing (which call sites allocated how much)? Useful for debugging; opt-in.
- jemalloc as an alternative to mimalloc for the global allocator? Both are fine; mimalloc is simpler to integrate. See [Options 005](../05-options/005-allocator.md).
