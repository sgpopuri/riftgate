# ADR 0005. Sharded MPMC queue strategy; crossbeam-channel in v0.1, hand-rolled Vyukov in v0.2

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [004-request-queue](../05-options/004-request-queue.md)
> **Deciders:** Sriram Popuri

## Context

The `PerShardScheduler` from [ADR 0004](0004-per-shard-default-stealing-opt-in.md) needs a queue per shard. This ADR records the queue strategy and the implementation phasing. Full exploration of candidates (`Mutex<VecDeque>`, single Vyukov MPMC, SPSC + routing, sharded MPMC) and the tradeoff matrix live in [Options 004](../05-options/004-request-queue.md).

The forces summarized: lock-free producer/consumer is required for [NFR-S03](../01-requirements/non-functional.md) (linear scaling); bounded queues are required for [FR-104](../01-requirements/functional.md) and [NFR-R03](../01-requirements/non-functional.md); per-shard observability is required for [NFR-OBS02](../01-requirements/non-functional.md); and engineering capacity is finite, so `v0.1` uses a mature off-the-shelf crate and the hand-rolled instrumentation-friendly version arrives in `v0.2` per [FR-106](../01-requirements/functional.md).

## Decision

The architectural decision is **sharded bounded MPMC**: one MPMC queue per worker shard, with accept-side fan-in.

The implementation phasing:

- **`v0.1`**: `riftgate-core` defines the `Queue<T>` trait. The default impl is `CrossbeamQueue<T>`, a thin wrapper around `crossbeam_channel::bounded::<T>(cap)`. The `PerShardScheduler` instantiates one `CrossbeamQueue<T>` per shard. Default capacity is configurable; default value is `2048` per shard, tunable via `RIFTGATE_SHARD_QUEUE_CAPACITY`.
- **`v0.2`**: `riftgate-core` adds `MpmcQueue<T>` (hand-rolled Vyukov bounded MPMC, `loom`-tested) and `ShardedMpmcQueue<T>` (the per-shard composition). These ship behind the `riftgate-mpmc` cargo feature initially; once the conformance and benchmark suites pass, they become the default and `crossbeam-channel` is retained as an optional alternative impl behind the `crossbeam-queue` feature.

Accept-side fan-in defaults to round-robin. Source-tuple hash is configurable via `accept_strategy = "source-hash"` in TOML config.

## Consequences

- **Positive:**
  - `v0.1` ships a working, mature queue without paying the engineering cost of a hand-rolled Vyukov MPMC up front.
  - `v0.2` owns the queue substrate end-to-end, which lets us add per-slot instrumentation (queue residency time, dequeue latency) that we cannot extract from `crossbeam-channel` cleanly.
  - Per-shard queue depth is a natural metric; one shard's queue filling does not affect other shards. Bulkhead pattern at the queue layer ([NFR-R03](../01-requirements/non-functional.md)).
  - `loom`-tested hand-rolled impl becomes a documentation-and-teaching artifact; this fits the project's documentation-first pillar.
- **Negative / accepted tradeoffs:**
  - The `v0.1` → `v0.2` impl swap means we must run a benchmark side-by-side before promoting the hand-rolled impl to default. We accept this cost in exchange for ownership of the hot-path data structure.
  - Hash-quality issues in production (one shard always hot) require operator-side mitigation (rendezvous hashing or least-loaded routing as a future deliverable). Not covered by this ADR.
  - Two impls of `Queue<T>` to maintain in `v0.2+`. The conformance suite is the discipline that keeps them honest.
- **Future work this enables:**
  - Per-slot residency metrics in `v0.2+`.
  - Priority-tier-aware queue variants (gated on [Options 022](../05-options/README.md) and the `v0.2` retro), per [FR-206](../01-requirements/functional.md).
  - Possible NUMA-aware queue allocation in a future thread-per-core deployment shape.
- **Future work this forecloses (until superseded):**
  - We will not ship a single shared MPMC as the production default.
  - We will not ship unbounded queues.
  - We will not ship `Mutex<VecDeque>`-backed `Queue<T>` impls outside of test scaffolding.

## Compliance

- `crates/riftgate-core::queue::Queue<T>` is the single trait that all queue impls implement.
- `crates/riftgate-core::queue::CrossbeamQueue<T>` is the `v0.1` default.
- `crates/riftgate-core::queue::MpmcQueue<T>` and `ShardedMpmcQueue<T>` are the `v0.2` impls (gated behind the `riftgate-mpmc` feature initially, default in `v0.2+`).
- `crates/riftgate-core/tests/queue_conformance.rs` runs every `Queue<T>` impl through the same conformance suite (FIFO order, bounded backpressure, drop semantics on full, concurrent push/pop).
- Hand-rolled MPMC requires a `loom` test in `crates/riftgate-core/tests/mpmc_loom.rs` covering at minimum: 2-producer/2-consumer with capacity 2; 3-producer/1-consumer; head/tail wrap.
- Cache-line padding (`#[repr(align(64))]`) on producer-side and consumer-side counters is verified by `std::mem::align_of_val` assertions in unit tests.
- Per-shard `riftgate_shard_queue_depth{shard="N"}` and `riftgate_shard_queue_capacity{shard="N"}` metrics are required; integration test verifies both are exported.

## Notes

- The naming distinction between `MpmcQueue<T>` (a single bounded MPMC) and `ShardedMpmcQueue<T>` (the per-shard composition) is intentional. They are separate types because one is a primitive and the other is the composition; consumers can pick.
- The `v0.2` hand-roll is justified primarily by ownership-and-instrumentation, not performance. Benchmarks against `crossbeam-channel` are expected to be within ±10%; if `crossbeam-channel` wins, we keep both impls and document the choice in a follow-up ADR.
- This ADR does not decide *what to put on a queue*. The queue carries `Task` (a per-request handle into the per-shard arena and the connection state). The shape of `Task` is internal to `riftgate-core` and can evolve without touching the queue trait.
