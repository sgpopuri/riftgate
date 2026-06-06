# ADR 0028. HierarchicalWheel is benchmark-gated, not milestone-scheduled; BinaryHeapTimers stays the default

> **Date:** 2026-06-06
> **Status:** accepted (supersedes the cutover-schedule clause of [ADR 0010](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md))
> **Options doc:** [006-timer-subsystem](../05-options/006-timer-subsystem.md)
> **Deciders:** Sriram Popuri

## Context

[ADR 0010](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md) shipped `BinaryHeapTimers` in `v0.1` and stated that `HierarchicalWheel` would "land in `v0.2`" as a peer impl and "become the default in `v0.3`" once a benchmark justified it. Two things are now true that the original schedule did not anticipate:

1. `HierarchicalWheel` was **not** built in `v0.2`. The shipped timer impls are still `BinaryHeapTimers` and the `DeterministicTimers` test wrapper (see [`crates/riftgate-core/src/timers.rs`](../../crates/riftgate-core/src/timers.rs)). No production workload has shown the heap to be the bottleneck.
2. The benchmark ADR 0010 named (`benchmarks/timers/heap_at_100k_1m.rs`) was never created at that path; the real timer benchmark lives at [`crates/riftgate-core/benches/timers.rs`](../../crates/riftgate-core/benches/timers.rs).

The original "v0.2 land / v0.3 default" calendar created a standing expectation that a ~1000-line hand-rolled cascading data structure (plus its conformance and fuzz surface) must be built on a milestone clock, regardless of whether the heap is actually hurting. That is speculative performance work, which cuts against the project's posture: pluggability over performance, honest numbers only, and an explicit decision *not* to compete with TensorZero on raw P99. This ADR resolves the carried-forward "HierarchicalWheel cutover threshold" open question by replacing the milestone schedule with a concrete, measured gate. Full candidate exploration remains in [Options 006](../05-options/006-timer-subsystem.md).

## Decision

**`BinaryHeapTimers` is the default timer subsystem indefinitely; `HierarchicalWheel` is not built on a milestone schedule and is implemented only when a documented benchmark gate trips — at which point its promotion to default is pre-authorized by this ADR and needs no further ADR.**

The discipline:

- **The gate.** `HierarchicalWheel` work is triggered when [`crates/riftgate-core/benches/timers.rs`](../../crates/riftgate-core/benches/timers.rs), run with the production churn mix (schedule + cancel + fire, idle-stream re-arm included), shows the heap's **per-tick p99 exceeding the 100 µs tick budget** from [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md) at **≥100k sustained live timers**, or schedule/cancel p99 regressing past the same budget. Until that gate trips, the heap stays default and no wheel is built.
- **No speculative build.** We do not land `HierarchicalWheel` "to have it." Absent gate evidence, the heap is the only production timer impl.
- **When the gate trips**, `HierarchicalWheel` is implemented as a peer impl behind the unchanged `TimerSubsystem` trait, must pass the shared conformance suite (`crates/riftgate-core/tests/timers_conformance.rs`), ships opt-in first (`[timer] kind = "hierarchical_wheel"`), and is then promoted to default. That promotion is **pre-authorized by this ADR**: it is recorded by the benchmark artifact in the promoting PR, not by a new ADR. A new ADR is required only to change the gate itself.
- The `TimerSubsystem` trait surface is unchanged and remains frozen per [ADR 0010](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md); this ADR changes *when/whether* the wheel lands, not the contract it lands behind.

## Consequences

- **Positive:**
  - No speculative perf work: engineering effort goes to the wheel only when measurement says the heap hurts, which honors "honest numbers only."
  - At current scale the heap is comfortably good enough (`log₂(n)` is 17–20 at 100k–1M live timers on a cache-friendly array), consistent with "pluggability over performance."
  - The trait seam already absorbs the wheel, so if/when it lands there is zero caller churn.
  - Removes the stale "v0.2 land / v0.3 default" expectation that no longer matches reality, closing a documentation drift.
  - Pre-authorizing the default flip on documented evidence avoids ADR ceremony for a decision we have already reasoned through.
- **Negative / accepted tradeoffs:**
  - We knowingly run an asymptotically-suboptimal default (O(log n) insert/cancel vs the wheel's amortized O(1)) until the gate trips. Accepted because the measured constant factors are fine at our scale.
  - If a real workload spikes timer churn, there is a build-and-validate lag before the wheel can become default. Mitigated by keeping the timer benchmark in CI, so the gate approaching is visible before it bites.
- **Future work this enables:**
  - A clean, evidence-triggered path to `HierarchicalWheel` sharing the existing conformance suite.
  - The same gate pattern (benchmark-triggered, pre-authorized promotion) is reusable for other "good enough until measured otherwise" subsystem upgrades.
- **Future work this forecloses (until superseded):**
  - We will not land `HierarchicalWheel` on a milestone calendar.
  - We will not change the default away from the heap without either the documented gate evidence or a new ADR that changes the gate.

## Compliance

- `BinaryHeapTimers` remains the configured default in the `riftgate` binary; `[timer] kind` selects an alternate impl only once one exists.
- [`crates/riftgate-core/benches/timers.rs`](../../crates/riftgate-core/benches/timers.rs) is the gate of record; CI tracks per-tick and schedule/cancel p99 at 100k and 1M live timers and surfaces regressions.
- Any `HierarchicalWheel` impl must pass `crates/riftgate-core/tests/timers_conformance.rs` before it can be selected, opt-in or default.
- Changing the gate threshold (the 100 µs budget or the 100k-timer floor) requires a new ADR superseding this one.

## Notes

- This ADR supersedes only the **cutover-schedule clause** of [ADR 0010](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md) (the "lands in v0.2 / default in v0.3" language). ADR 0010's substantive decisions — heap as the v0.1 impl, lazy-deletion cancellation, the frozen trait surface, per-shard ownership — all stand unchanged.
- The 100 µs per-tick budget is not new; it is the figure already named in [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md). This ADR promotes it from a design note to the literal trigger condition.
- The Varghese–Lauck hierarchical wheel (SOSP 1987) remains the correct *production-grade* answer at sufficient scale. Nothing here disputes that; the decision is to let measurement, not a calendar, decide when "sufficient scale" has arrived.
