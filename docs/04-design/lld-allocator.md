# 04.e LLD — Allocator

> Per-request arena allocator. Eliminates `malloc` from the hot path. Returns all per-request memory in O(1) on completion.
>
> Status: **outline-stage**. Filled out as `v0.1` lands.

## Purpose

Make memory allocation cost predictable in the request lifecycle. The general-purpose `malloc` is fine; per-request, scoped allocation is faster and tail-latency-cleaner.

## Trait surface

```rust
// Sketch
pub trait Allocator: Send + Sync {
    fn alloc(&self, layout: Layout) -> *mut u8;
    fn reset(&self);  // Free everything in O(1)
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `SystemAllocator` | `v0.1` | `riftgate-core` | Wraps the global allocator. Default for non-hot-path. |
| `BumpArena` | `v0.1` | `riftgate-core` | Per-request arena. Bumps a pointer; resets to zero. |
| `MimallocGlobal` | `v0.2` (opt-in) | (link in `mimalloc` crate) | Replaces the global allocator for `malloc` calls outside the arena. |

Decision rationale: [Options 005 (allocator)](../05-options/005-allocator.md).

Foundational principles: bump-pointer arena allocators (Postgres memory contexts, LLVM/rustc per-pass arenas) and modern multi-thread `malloc` implementations (`jemalloc`, `mimalloc`, `tcmalloc`; Berger Hoard paper for the underlying multithread-allocator design).

## Component context

### Architecture and dependencies

The allocator is owned by the per-request context. The context is created in the [`scheduling`](lld-scheduling.md) layer when a request enters and destroyed (i.e. the arena is reset) on completion. All per-request data structures (parser scratch buffers, filter state, response buffers) allocate from the arena.

### Patterns and conventions

- **Arena scope = request scope.** Nothing outlives the request unless explicitly copied.
- **Free is a no-op.** Individual deallocation is a programming error; the arena is reset wholesale.
- **Pre-sized arenas.** Each arena starts at 4 KB and grows by doubling up to a configured cap (default 1 MB). Over-cap requests fall back to system allocation with a warning.
- **Arena pool.** Completed-request arenas are returned to a per-worker pool, not freed back to the OS, so subsequent requests reuse the memory.

### Pitfalls

- **Lifetime confusion.** Borrows from arena memory must not escape the request scope. Compile-time enforced by Rust's borrow checker; defense in depth via lifetime annotations on the public API.
- **Unbounded growth.** Without a cap, a single bad request can grow the arena forever. The cap is enforced.
- **Alignment mistakes.** The bump allocator must respect each `Layout`'s alignment; tests cover misaligned allocations.

### Standards and review gates

- Allocator changes require microbenchmark on the `accept→arena_alloc→reset` path.
- Memory usage after a million-request soak must be flat (within noise).
- Fuzz tests on `BumpArena` for alignment and overflow.

## Testing strategy

- Alignment fuzz.
- Long-running soak — RSS should be flat.
- Concurrent stress — many workers allocating concurrently from per-worker arenas.

## Open questions

- Should the arena pool be per-worker or shared? Recommend per-worker to avoid cross-core synchronization. Cost: slightly higher RSS.
- Should we support per-request arena tracing (which call sites allocated how much)? Useful for debugging; opt-in.
- jemalloc as an alternative to mimalloc for the global allocator? Both are fine; mimalloc is simpler to integrate. See [Options 005](../05-options/005-allocator.md).
