# ADR 0004. Shared-nothing per-shard scheduler in v0.1; work-stealing as v0.2 opt-in

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [003-concurrency-model](../05-options/003-concurrency-model.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs a `Scheduler` impl in `riftgate-core` that distributes incoming requests across worker shards. Full exploration of candidates (shared global queue, shared-nothing per shard, work-stealing, actor model) and the tradeoff matrix live in [Options 003](../05-options/003-concurrency-model.md).

This decision is layered on top of [ADR 0003](0003-tokio-multithread-default.md). Tokio's runtime work-steals at the OS-thread / future level; Riftgate's `Scheduler` decides at the request-distribution level. The two layers do not collapse into one.

## Decision

**`v0.1` ships `PerShardScheduler` as the default `Scheduler` impl.**

- N worker shards, each owning a private bounded MPMC queue (the implementation lands in [Options 004](../05-options/004-request-queue.md)).
- The accept loop hashes incoming connections to a shard. Default hash is round-robin; source-tuple hash is configurable.
- A connection's lifetime is bound to its shard from accept to response. No cross-shard task migration.
- Per-shard metrics (queue depth, latency histogram) are exported via `/metrics`.

**`v0.2` adds `WorkStealingScheduler` behind the `work-stealing` cargo feature and a `scheduler = "work-stealing"` config setting.** Default remains `PerShardScheduler` until the `v0.2` retro produces data justifying a different default.

## Consequences

- **Positive:**
  - Predictable per-shard tail latency. Operators can reason about each shard independently and correlate hot shards with traffic sources.
  - Cache locality on the hot path. Per-request arena ([`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md)), parser scratch buffers, and filter state stay on one shard's CPU caches.
  - Per-shard SLO enforcement is naturally available, unlocking the priority-tier path in `v0.3` ([FR-206](../01-requirements/functional.md), [Options 022](../05-options/README.md)) without committing to it now.
  - The `Scheduler` trait stays the abstraction boundary; adding work-stealing is a new impl, not a trait redesign.
- **Negative / accepted tradeoffs:**
  - Heterogeneous request mixes (one shard hot, others idle) suffer in `v0.1`. We accept this until the `v0.2` work-stealing impl ships.
  - Hash-quality issues in production (one shard always hot) require operator awareness or rendezvous hashing as a future improvement.
  - Two-layer scheduling (Tokio's internal work-stealer + Riftgate's per-shard) is more complex to reason about than a single layer; documentation must be explicit about what runs where.
- **Future work this enables:**
  - `WorkStealingScheduler` (`v0.2`) per [FR-107](../01-requirements/functional.md).
  - `PriorityPerShardScheduler` (gated on [Options 022](../05-options/README.md) and the `v0.2` retro), per [FR-206](../01-requirements/functional.md).
  - True thread-per-core in `v0.2`+ (sharded current-thread Tokio per shard, with `sched_setaffinity` pinning).
- **Future work this forecloses (until superseded):**
  - We will not ship a shared global queue scheduler.
  - We will not ship an actor-model scheduler.
  - We will not auto-detect "best" scheduler at runtime; users opt in explicitly.

## Compliance

- `crates/riftgate-core::scheduler::PerShardScheduler` is the default `Scheduler` impl.
- `crates/riftgate-core/tests/scheduler_conformance.rs` runs every `Scheduler` impl through the same conformance suite.
- Per-shard metrics export is verified by an integration test that asserts `riftgate_shard_queue_depth{shard="N"}` for every shard.
- Adding a new `Scheduler` impl requires a new ADR superseding (or amending) this one, plus passing the conformance suite and updating the metrics integration test.
- Code-review gate: any code that does cross-shard state access in `riftgate-core` requires explicit justification in the PR description.

## Notes

- The decision to ship per-shard before work-stealing is in line with the systems-design literature on bulkhead isolation (Nygard, *Release It*) and shared-nothing architecture (ScyllaDB / Seastar): start with shared-nothing, add coupling only when measurements demand it.
- Many readers may expect Riftgate to ship work-stealing first because Tokio (and Go, Rayon, Cilk, TBB) all default to it. The distinction is that those are *task* schedulers — sub-millisecond units. Riftgate's `Scheduler` is a *request* scheduler — hundreds of microseconds per unit, with a heavier per-unit working set. The cost model is different.
- The naming — `PerShardScheduler` here vs `PerCoreScheduler` in earlier LLD drafts ([`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md)) — is deliberate. "Per-core" implies hardware affinity, which is a `v0.2`+ deployment-shape decision. "Per-shard" is the `v0.1` reality: M logical shards on N Tokio threads, no pinning required.
