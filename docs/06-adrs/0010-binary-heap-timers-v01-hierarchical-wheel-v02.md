# ADR 0010. Binary-heap timer subsystem in v0.1; hierarchical wheel in v0.2 behind the same trait

> **Date:** 2026-05-10
> **Status:** accepted — the cutover-schedule clause (`HierarchicalWheel` lands in v0.2, becomes default in v0.3) is **superseded by [ADR 0028](0028-timer-cutover-benchmark-gated.md)**, which makes the cutover benchmark-gated; all other clauses stand.
> **Options doc:** [006-timer-subsystem](../05-options/006-timer-subsystem.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs a per-shard timer subsystem to enforce per-request deadlines (request-overall, upstream-call, idle-stream) at the scale described by [`NFR-S01`](../01-requirements/non-functional.md) (≥50k concurrent connections) and [`NFR-S02`](../01-requirements/non-functional.md) (≥10k concurrent in-flight streaming requests). Full exploration of candidates (binary heap, single-level hashed wheel, hierarchical wheel, OS `timerfd`-per-timer, and direct delegation to Tokio) lives in [Options `006`](../05-options/006-timer-subsystem.md). The decision is recorded here.

The forces summarized: cancellation is the dominant operation (streaming-token re-arm cancels and re-inserts on every token); tick processing must not be O(n) over live timers ([`FR-008`](../01-requirements/functional.md)); the trait surface in [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md) is the kernel contract and must accommodate a future wheel impl without breaking changes; a 10 ms tick resolution is plenty for request-deadline use.

## Decision

**`v0.1` ships `BinaryHeapTimers` — a `std::collections::BinaryHeap<Reverse<(Instant, TimerId)>>` per shard, with cancellation implemented as lazy deletion via a per-shard `HashSet<TimerId>` and a configurable compaction trigger. `v0.2` adds `HierarchicalWheel` as a peer impl of the `TimerSubsystem` trait, becoming the default in `v0.3` only if the `v0.2` benchmark gate justifies the constant-factor win.**

The discipline:

- The `TimerSubsystem` trait is defined in `crates/riftgate-core::timers` per the sketch in [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md). The signature accepts both heap and wheel impls without modification: `schedule(deadline, callback) -> TimerHandle`, `cancel(handle) -> bool`, `tick(now)`.
- `BinaryHeapTimers` is the only shipped impl in `v0.1`. `DeterministicTimers` (a `#[cfg(test)]` impl that advances a controlled clock for unit tests) ships alongside it as the FR-X02 second impl.
- Tick resolution defaults to 10 ms; configurable to 5 ms / 100 ms via the `[timer]` block in the config (per Options [`015`](../05-options/015-config-model.md)).
- Each shard owns a private `BinaryHeapTimers`. No locking on the hot path. Cross-shard timer dispatch (rare) borrows the per-shard MPMC queue ([ADR `0005`](0005-sharded-mpmc-queue.md)).
- The per-shard worker loop alternates `AsyncIO::poll(timeout = next_deadline - now)` and `TimerSubsystem::tick(now)`. There is no separate timer thread.
- Cancelled-set growth is observable via `riftgate_timers_cancelled_pending` (gauge). A configurable compaction trigger (default: when the cancelled set exceeds 25% of the heap size) drains stale entries.
- `HierarchicalWheel` lands in `v0.2`. It is opt-in (selected via `[timer] kind = "hierarchical_wheel"` in config). It becomes default in `v0.3` only after `benchmarks/timers/heap_at_100k_1m.rs` shows the heap exceeding the LLD's <100 µs tick budget under realistic load.

## Consequences

- **Positive:**
  - The `v0.1` impl is ~150 lines of code (heap + cancelled set + tick loop + compaction). The wheel would be ~1000+ lines including cascade-correctness tests.
  - The heap satisfies [`FR-008`](../01-requirements/functional.md) acceptance ("100k concurrent timers cost less than O(n) per tick"): tick processes only expired entries (O(k log n)), not all live entries.
  - At our `v0.x` scale (100k–1M timers), `log_2(n)` is 17–20 — practical O(1) on cache-friendly array layout. The heap is good enough.
  - Zero `unsafe` code; zero hand-rolled data structures; small conformance-test surface.
  - `DeterministicTimers` makes deadline-sensitive code unit-testable without sleeping.
  - The trait is the contract; `HierarchicalWheel` lands in `v0.2` without callers changing.
- **Negative / accepted tradeoffs:**
  - Insert and pop are O(log n), not O(1). Wheels are asymptotically faster; we accept the constant-factor cost in exchange for the implementation simplicity.
  - The cancelled-set requires periodic compaction; a pathological cancel-heavy pattern with no pops can grow the set if compaction is misconfigured. The `riftgate_timers_cancelled_pending` metric is the operator-visible signal.
  - Far-future timers occupy their heap slot the entire time. Practically irrelevant for our workload (request deadlines are bounded in seconds), but worth naming.
  - Each shard's heap is private; cross-shard timer dispatch (a request migrated to another shard) costs one MPMC enqueue, which is the right tradeoff over a global locked structure.
- **Future work this enables:**
  - `HierarchicalWheel` in `v0.2`, sharing the conformance suite (`crates/riftgate-core/tests/timers_conformance.rs`).
  - Deterministic deadline-test harnesses for filter chains, hedged-request cancellation, MCP audit timeouts.
  - A future `TimerfdTimers` impl for the (hypothetical) sub-millisecond-precision use case.
- **Future work this forecloses (until superseded):**
  - We will not promote a hierarchical wheel to `v0.1` even if a contributor wants to land one — the `v0.1` impl is the heap, full stop.
  - We will not delegate timers to Tokio's runtime-internal timer driver as the hidden default; the `TimerSubsystem` trait is the kernel surface.
  - We will not ship a `timerfd`-per-timer implementation; the fd-table cost is prohibitive at our scale.

## Compliance

- `crates/riftgate-core::timers::TimerSubsystem` is the single trait; `BinaryHeapTimers` is the `v0.1` shipped impl; `DeterministicTimers` is the `#[cfg(test)]` second impl.
- `crates/riftgate-core/tests/timers_conformance.rs` runs every `TimerSubsystem` impl through the same conformance suite (in-order firing, cancellation correctness, idempotent cancel, monotonic-clock invariants under simulated jumps).
- `benchmarks/timers/heap_at_100k_1m.rs` benchmarks `schedule + cancel + fire` at 100k and 1M live timers and gates regressions in CI.
- `riftgate_timers_cancelled_pending` (gauge) and `riftgate_timers_total` (counter, labelled by `event = scheduled|cancelled|fired|compacted`) are emitted via the observability bus per [ADR `0011`](0011-otel-default-sink-multisink-fanout.md).
- Adding a new `TimerSubsystem` impl requires passing the conformance suite. Promoting `HierarchicalWheel` to default in `v0.3` requires a new ADR superseding the relevant clause of this one.

## Notes

- The choice of a heap over a wheel for `v0.1` is conservative on purpose. The Varghese–Lauck (SOSP 1987) hierarchical wheel is the right *production-grade* answer; it is also a real implementation effort and a real conformance-test effort. We ship the conservative option in `v0.1` and let `v0.2` benchmark data, not speculation, drive the upgrade.
- "Less than O(n) per tick" in [`FR-008`](../01-requirements/functional.md) is satisfied by the heap because tick pops only expired entries from the top. The wording in FR-008 names the *direction* (a hierarchical wheel) rather than the literal asymptotic class; the heap is the conservative `v0.1` step on the way there. The Options doc records this nuance to prevent a reader from perceiving a contradiction.
- The lazy-deletion pattern for cancellation is an old standard (Linux's `epoll` ready list, Java's `DelayQueue`, the Go runtime's old timer impl). The compaction trigger is the only knob that needs tuning in production; default is conservative (25% threshold).
- Per-shard ownership is non-`Send`. This aligns with [ADR `0004`](0004-per-shard-default-stealing-opt-in.md): a request stays on its shard, so its timers stay on the same shard's heap.
