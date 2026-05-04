# ADR 0006. Per-request bump arena on the hot path; system malloc globally in v0.1; mimalloc opt-in in v0.2

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [005-allocator](../05-options/005-allocator.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs an allocator strategy that keeps hot-path allocation cost predictable while leaving the global allocator surface familiar. Full exploration of candidates (system `malloc`, jemalloc, mimalloc, bump arena alone, bump arena + mimalloc) and the tradeoff matrix live in [Options 005](../05-options/005-allocator.md).

The forces summarized: the request hot path benefits enormously from a bump-pointer arena (allocation drops from ~50-200 ns to ~5-15 ns; bulk reset replaces per-allocation `free`); the global allocator quality matters for non-hot-path lifetimes (connection state, WAL, timer wheel, config); operators value familiarity, so the global allocator default should be the platform default.

## Decision

**`v0.1` ships:**

- A per-request `BumpArena` (`crates/riftgate-core::allocator::BumpArena`) used for every per-request allocation: parser scratch, filter state, response framing, intermediate JSON values.
- A per-worker arena pool: completed-request arenas are reset and returned to the worker's local pool, not freed back to the OS, so subsequent requests reuse the memory.
- Per-request memory cap (default 1 MB, configurable via `RIFTGATE_REQUEST_ARENA_CAP_BYTES`). Over-cap allocations fall back to the global allocator with a `riftgate_arena_overflow_total` metric increment and a `tracing::warn!` event.
- The system `malloc` (whatever the platform provides — `ptmalloc2` on glibc Linux, `mallocng` on musl, the macOS allocator on Tier-2) as the `#[global_allocator]` default.
- A `SystemAllocator` impl of the `Allocator` trait that wraps the global allocator, used for non-hot-path components.

**`v0.2` adds:**

- A `MimallocGlobal` opt-in via `--features mimalloc`. When enabled, `mimalloc` replaces the system `malloc` as the `#[global_allocator]`. The `BumpArena` is unchanged.
- Per-arena pool size and growth metrics (`riftgate_arena_pool_size_bytes`, `riftgate_arena_growth_total{cause="capacity"}`).

## Consequences

- **Positive:**
  - Hot-path allocation cost drops by an order of magnitude vs system `malloc`. Per-request memory release is O(1).
  - Per-worker arena pools keep the working set cache-warm, so the steady-state allocator footprint is small and predictable ([NFR-C01](../01-requirements/non-functional.md), [NFR-P04](../01-requirements/non-functional.md)).
  - Global allocator default is the platform's most-familiar shape, which keeps debugging tools (`malloc_stats`, `valgrind`, `mtrace`) working out of the box.
  - `mimalloc` opt-in in `v0.2` gives users a tunable for the non-hot-path that does not break the default deployment.
  - Lifetime safety is enforced by Rust's borrow checker; the trait surface uses lifetime annotations to prevent arena memory from escaping request scope.
- **Negative / accepted tradeoffs:**
  - Hand-rolled `BumpArena` is real correctness work: alignment must be respected for every `Layout`; tests cover the surface; fuzz coverage is non-optional.
  - Two allocator surfaces (arena + global) means memory regressions need triage to attribute. We invest in `riftgate_arena_*` metrics to make this tractable.
  - `mimalloc` as a feature increases binary size by ~150 KB when enabled; small enough that we accept it.
  - Per-request arena cap can shed legitimate large requests if the cap is too tight. Default is conservative; operators tune as needed.
- **Future work this enables:**
  - Per-request arena tracing (which call sites allocated how much) as a debugging feature behind `--features arena-tracing`.
  - Possible NUMA-aware arena allocation in a future thread-per-core deployment.
  - Cross-allocator A/B benchmarks in `v0.2` to inform whether `mimalloc` should ever become the default.
- **Future work this forecloses (until superseded):**
  - We will not ship a `tcmalloc` or `snmalloc` global allocator option in `v0.x`.
  - We will not ship a custom global allocator written in-tree.
  - We will not allow per-request allocations without a cap.

## Compliance

- `crates/riftgate-core::allocator::Allocator` trait is the single trait that all allocator impls implement.
- `crates/riftgate-core::allocator::BumpArena` is the hot-path impl. `SystemAllocator` is the global-allocator wrapper. `MimallocGlobal` (gated behind `--features mimalloc`) ships in `v0.2`.
- `crates/riftgate-core/tests/arena_alignment.rs` covers alignment requirements for every `Layout` we exercise; CI runs this with sanitizers (`MIRIFLAGS="-Zmiri-strict-provenance"` under `cargo miri`).
- `crates/riftgate-core/tests/arena_overflow.rs` verifies the cap is enforced and the overflow path is observable.
- A long-running soak benchmark (10M requests, default config) verifies that RSS is flat within noise after warm-up. CI gates regressions.
- Public APIs that return arena-borrowed data carry an `'arena` lifetime; review enforces this.
- Adding a new global allocator requires a new ADR superseding (or amending) this one, plus a comparative benchmark.

## Notes

- The choice of `mimalloc` over `jemalloc` for the `v0.2` opt-in is deliberate. As of 2026, jemalloc's upstream maintenance picture is unclear after Facebook archived the upstream repo in 2024; mimalloc has active Microsoft Research stewardship and a simpler integration story. Operators who specifically need jemalloc can write their own `#[global_allocator]` block; the project does not actively block them.
- The default request-arena cap of 1 MB is a starting point, not a target. Real workloads may vary by orders of magnitude. The metric `riftgate_arena_overflow_total` is the operator-visible signal that the cap may need tuning.
- The arena is non-`Send`. This is intentional and aligns with the per-shard execution model from [ADR 0004](0004-per-shard-default-stealing-opt-in.md): a request stays on its shard, so its arena does not need to cross thread boundaries.
- Per-request arena state must not be retained across the request boundary unless explicitly copied to the global allocator. This is the most common bug class for arena-based code in any language; Rust's borrow checker catches it cleanly.
