# ADR 0027. Per-request bump-arena recycling uses a per-shard pool, not a shared pool

> **Date:** 2026-06-06
> **Status:** accepted
> **Options doc:** [005-allocator](../05-options/005-allocator.md)
> **Deciders:** Sriram Popuri

## Context

[ADR 0006](0006-bump-arena-plus-system-malloc.md) committed Riftgate to a per-request `BumpArena` on the hot path with completed-request arenas reset and returned to a pool rather than freed to the OS. It deliberately left one shape question open: is that pool **per-shard** (one pool per worker / core) or **shared** (a single global pool across cores)? [`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md) recorded the same open question and recommended per-shard, pending a measured decision. This ADR closes that question. Full candidate exploration lives in [Options 005](../05-options/005-allocator.md).

The forces summarized: the recycle path runs on every request completion, so it is hot; a shared pool would require cross-core synchronization (a lock or a concurrent stack) on that hot path; the `BumpArena` is already `!Send` and the execution model is shared-nothing per-shard ([ADR 0004](0004-per-shard-default-stealing-opt-in.md)), so a request and its arena never leave their shard. The only thing a shared pool would buy is letting a busy shard borrow an idle shard's spare arenas — at the cost of putting synchronization on the most frequently travelled path in the system.

## Decision

**The per-request arena pool is per-shard: each worker owns a private free-list of reset `BumpArena`s with no cross-core synchronization; a shared/global arena pool is rejected.**

The discipline:

- Each shard's worker owns a private `Vec<BumpArena>` free-list. On request completion the arena is `reset()` and pushed back; on request entry the worker pops one or constructs a fresh arena if the list is empty. No lock, no atomic, no cross-core handoff on this path.
- The free-list is bounded: a shard retains up to `arena_pool_max` reset arenas (default 32, configurable via the `[allocator]` block per [Options 015](../05-options/015-config-model.md)); excess arenas are freed to the global allocator so the idle pool cannot grow without bound.
- An arena whose high-water capacity exceeded `arena_pool_retain_cap_bytes` (default 64 KiB) during the request is **freed rather than pooled**, so a single large request does not pin a multi-MiB chunk in the steady-state pool. This is the recycle-path companion to the per-request 1 MiB allocation cap from [ADR 0006](0006-bump-arena-plus-system-malloc.md).
- Cross-shard arena borrowing is explicitly not supported. A request that migrates shards (rare; see [ADR 0005](0005-sharded-mpmc-queue.md)) gets a fresh arena from its destination shard's pool.

## Consequences

- **Positive:**
  - Zero synchronization on the request recycle path — the hottest allocation path in the system stays lock-free and atomic-free.
  - Each shard's pool stays cache-warm for that core, which is the whole point of recycling; a shared pool would shuttle arenas between cores and cold-trash their cache lines.
  - Aligns cleanly with the shared-nothing per-shard model ([ADR 0004](0004-per-shard-default-stealing-opt-in.md)) and the `!Send` arena; no new thread-safety surface is introduced.
  - The two bounds (`arena_pool_max`, `arena_pool_retain_cap_bytes`) keep idle RSS flat and prevent a large request from pinning memory.
- **Negative / accepted tradeoffs:**
  - Slightly higher aggregate idle RSS than a shared pool: each shard holds its own spare arenas, so the worst-case idle pool memory is `num_shards × arena_pool_max × typical_arena_size`. The bounds keep this small and predictable; we accept it as the cost of a sync-free hot path.
  - A momentarily busy shard cannot borrow an idle shard's spare arenas; it constructs fresh ones instead. This is a deliberate trade of a small allocation-rate cost for the removal of all cross-core coordination.
- **Future work this enables:**
  - NUMA-aware per-shard pools (pin each shard's arenas to its local NUMA node) become a clean future addition, since the pool is already core-local.
  - Per-shard arena-pressure metrics feed naturally into the v0.4 observability plane.
- **Future work this forecloses (until superseded):**
  - We will not ship a shared/global arena pool with cross-core synchronization in `v0.x`.
  - We will not add cross-shard arena stealing; arena locality is a property we keep.

## Compliance

- `crates/riftgate-core` owns the per-shard free-list type; the `riftgate` binary constructs one per worker. No arena type gains a `Send`/`Sync` bound.
- The 1M-request soak benchmark required by [ADR 0006](0006-bump-arena-plus-system-malloc.md) is extended to assert flat RSS with the pool bounds applied; CI gates regressions.
- `riftgate_arena_pool_size_bytes` is emitted **per shard** (labelled by `shard`) via the observability bus per [ADR 0011](0011-otel-default-sink-multisink-fanout.md), so an oversized pool is operator-visible.
- Changing the pool to a shared design, or removing the retain-cap bound, requires a new ADR superseding this one plus a comparative soak benchmark.

## Notes

- "Per-shard" and "per-worker" are the same thing in Riftgate's execution model: one worker per shard per core ([ADR 0004](0004-per-shard-default-stealing-opt-in.md)). The decision is per-shard *as opposed to a single shared pool*, which was the actual open question.
- The retain-cap (free rather than pool an oversized arena) is the load-bearing detail. Without it, a per-shard pool can slowly accumulate large grown arenas — one per shard that ever saw a big request — and hold that memory for the life of the process. Freeing oversized arenas back to the global allocator keeps steady-state RSS tied to the *typical* request, not the *largest* request.
- The arena remains `!Send`, which is what makes the lock-free free-list sound: the borrow checker prevents an arena from escaping its shard, so no other core can ever touch it.
