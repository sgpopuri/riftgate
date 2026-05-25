# 04.f LLD — Timers

> Hierarchical timing wheel for per-request deadlines. O(1) insert, O(1) cancel, O(1) tick processing for the common case.
>
> Status: **shipped (v0.1)**. The v0.1 default is `BinaryHeapTimers` per [ADR 0010](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md); `HierarchicalWheel` lands in v0.2 behind the same trait.

## Purpose

Track per-request deadlines (request-overall, upstream-call, idle-stream) at a scale of 100k+ concurrent requests without paying O(n) per tick that a naive scan would charge.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/timers.rs`](../../crates/riftgate-core/src/timers.rs):

```rust
pub struct TimerHandle(pub u64);

pub trait TimerSubsystem {
    fn schedule(
        &mut self,
        deadline: Instant,
        on_fire: Box<dyn FnOnce() + Send>,
    ) -> TimerHandle;
    fn cancel(&mut self, handle: TimerHandle) -> bool;
    fn tick(&mut self, now: Instant);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn next_deadline(&self) -> Option<Instant>;
}
```

Two design adjustments from the v0.0 outline:

- **Methods take `&mut self`** because `BinaryHeapTimers` mutates its heap and cancelled set on every call. Per-shard ownership ([ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md)) means a shard's `TimerSubsystem` instance is private to that shard's worker, so `Send + Sync` is not required either.
- **`len()` and `next_deadline()`** are part of the trait so the per-shard worker loop can compute the next IO poll timeout (`AsyncIO::poll(timeout = next_deadline - now)`) without poking impl-private state.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `BinaryHeapTimers` | shipped (v0.1) | `riftgate-core` | `BinaryHeap<Reverse<(Instant, u64)>>` + `HashMap<u64, callback>` + lazy `HashSet<u64>` of cancelled ids. O(log n) schedule / O(1) average cancel / O(k log n) tick. Compaction rebuilds the heap when the cancelled set exceeds 25% of the heap. |
| `DeterministicTimers` | shipped (v0.1) | `riftgate-core` | Wraps `BinaryHeapTimers` with an externally-driven `Instant`; used by tests that need to control time without sleeping. |
| `HierarchicalWheel` | v0.2 | `riftgate-core` | O(1) insert/cancel; O(slot-list) per tick. Replaces `BinaryHeapTimers` as the default once tick scaling becomes the bottleneck. |

Decision rationale: [Options 006 (timer subsystem)](../05-options/006-timer-subsystem.md).

Foundational principles: hashed and hierarchical timing wheels (Varghese and Lauck, *Hashed and Hierarchical Timing Wheels: Data Structures for the Efficient Implementation of a Timer Facility*, SOSP 1987); low-level synchronization primitives (`futex`, mutex, condvar) per Drepper's *Futexes Are Tricky* and the Linux `futex(2)` man page.

## Component context

### Architecture and dependencies

Timers are per-shard, owned exclusively by the shard's worker (no `Send + Sync` bound on the trait — sharing is a misuse). The worker loop drives the timer subsystem in three steps each pass:

1. `tick(now)` to drain expired timers and execute their callbacks inline.
2. `next_deadline()` to compute how long to block in `AsyncIO::poll(timeout)`.
3. `schedule(...)` / `cancel(...)` from request-handling code as deadlines are taken on or released.

Timers depend on a monotonic clock source (`std::time::Instant`, which on Linux maps to `clock_gettime(CLOCK_MONOTONIC)`). The `DeterministicTimers` wrapper substitutes an externally-driven `Instant` for tests.

### Patterns and conventions

- **Per-shard, no sharing.** Each worker's heap is private; `&mut self` enforces this at the type level.
- **Lazy cancellation.** `cancel()` records the handle id in a `HashSet` rather than searching the heap. The cancelled entry is skipped at `tick()` time.
- **Compaction threshold.** When the cancelled set exceeds 25% of the heap size, the heap is rebuilt without the cancelled entries. This bounds memory bloat from cancel-heavy workloads (e.g. a client that opens and immediately closes 10k connections).
- **Stable handles.** `TimerHandle` wraps a monotonically increasing `u64` so handles are unique for the lifetime of the subsystem; reusing handles after `cancel()` would race with lazy compaction.
- **Tick resolution is decided by the worker loop**, not the subsystem. The current default in the `riftgate` binary is 10 ms; the trait does not constrain it.

### Pitfalls

- **Slow callbacks block the worker.** Callbacks run on the shard worker — keep them small. Long work belongs on the scheduler, not the timer.
- **Cancellation races** are impossible because the trait is `&mut self` and the subsystem is single-threaded per shard. Any "looks racy" code is a sign the trait is being used wrong.
- **Heap memory bloat** without compaction. A naive lazy-cancel implementation can hold gigabytes of dead handles; the 25% threshold is a hard requirement, not an optimization.
- **`u64` handle wraparound** is theoretically possible after ~580 years of continuous scheduling at 1 GHz. Documented for completeness; not a real concern.

### Standards and review gates

- Timer changes must keep [`crates/riftgate-core/benches/timers.rs`](../../crates/riftgate-core/benches/timers.rs) green: schedule and cancel must remain in the low-microseconds range at 100k timers.
- Memory overhead per timer in `BinaryHeapTimers` is currently ~80 bytes (Reverse(Instant, u64) tuple in the heap + (u64, Box<dyn FnOnce>) entry in the HashMap); budget is <128 bytes.
- The trait surface is part of the v0.1 frozen surface — changes require a new ADR superseding [ADR 0010](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md).

## Testing strategy

- Unit tests in `riftgate-core/src/timers.rs` cover schedule / fire ordering, `cancel` returning `true` only for live handles, and `next_deadline()` correctness.
- The `DeterministicTimers` wrapper is exercised by the tests in `riftgate-core/src/timers.rs::deterministic_tests` and is the recommended fixture for downstream consumers.
- The end-to-end test in [`crates/riftgate/tests/e2e.rs`](../../crates/riftgate/tests/e2e.rs) implicitly exercises the per-shard timer for request and idle deadlines.
- The microbenchmark in [`crates/riftgate-core/benches/timers.rs`](../../crates/riftgate-core/benches/timers.rs) measures schedule / cancel / fire at the 100k-timer scale.

## Open questions

- Should we support sub-millisecond timers? Recommend no for the wheel; users can do their own tight loops if they need that.
- Should the tick rate be configurable per shard? Recommend yes (5 ms / 10 ms / 100 ms options) but default to 10 ms; track in the v0.2 plan.
- Is cross-shard timer dispatch needed? So far no real workload has required it. If it becomes a need, the recommendation is per-shard wheels with replicated "global" timers, not a shared heap.
