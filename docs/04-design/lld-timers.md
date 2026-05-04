# 04.f LLD — Timers

> Hierarchical timing wheel for per-request deadlines. O(1) insert, O(1) cancel, O(1) tick processing for the common case.
>
> Status: **outline-stage**. Filled out as `v0.1` lands.

## Purpose

Track per-request deadlines (request-overall, upstream-call, idle-stream) at a scale of 100k+ concurrent requests without paying O(log n) per insert / cancel that a heap would charge.

## Trait surface

```rust
// Sketch
pub struct TimerHandle(/* opaque */);

pub trait TimerSubsystem: Send + Sync {
    fn schedule(&self, deadline: Instant, on_fire: Box<dyn FnOnce() + Send>) -> TimerHandle;
    fn cancel(&self, handle: TimerHandle) -> bool;  // Returns true if cancellation pre-empted the fire.
    fn tick(&self, now: Instant);  // Called once per resolution unit.
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `BinaryHeapTimers` | `v0.1` | `riftgate-core` | O(log n). Default until heap performance becomes a bottleneck. |
| `HierarchicalWheel` | `v0.2` | `riftgate-core` | O(1) insert/cancel; O(slot-list) per tick. |

Decision rationale: [Options 006 (timer subsystem)](../05-options/006-timer-subsystem.md).

Source-systems chapter: `Ch15 (timer wheels and scheduling primitives)`.

## Component context

### Architecture and dependencies

Timers are per-worker. Each worker owns a timing wheel and ticks it as part of the worker loop. Cross-worker timer dispatch (rare) goes through the same MPMC queue as work tasks. Timers depend on a monotonic clock source (`Instant` — typically `clock_gettime(CLOCK_MONOTONIC)`).

### Patterns and conventions

- **Per-worker, no sharing.** Each worker's wheel is private to that worker.
- **Tick resolution = 10 ms by default.** Coarser than `hrtimer` but plenty for request-deadline use.
- **Wheels are doubly-linked-list buckets.** Cancellation is O(1) given the handle.
- **Hierarchical levels** so that scheduling far in the future does not waste any wheel slots — far-future timers cascade down as time passes.

### Pitfalls

- **Tick precision.** With 10 ms ticks, a "5-second timeout" can fire anywhere between 5.000s and 5.010s. This is fine for our use; document the tradeoff.
- **Cancellation races.** A timer can fire concurrently with cancellation; the cancel path must handle "already fired" cleanly.
- **Cascading cost.** When a far-future timer becomes near-future, it cascades to the lower wheel. Cascading is amortized O(1) but can be noisy at wheel-boundary crossings.
- **Slow callbacks block the tick.** Timer callbacks must be small. Long work belongs on the scheduler, not the timer.

### Standards and review gates

- Timer changes require a microbenchmark on `schedule + cancel + fire` at 1M timers.
- Memory overhead per timer should be <64 bytes.
- Tick processing time at peak should be <100 µs.

## Testing strategy

- Microbenchmarks at 100k, 1M, 10M timers.
- Fault injection — slow callbacks, fast cancellations.
- Cross-validation: schedule N timers, verify fire times within tolerance.

## Open questions

- Should we support sub-millisecond timers? Recommend no for the wheel; users can do their own tight loops if they need that.
- Should the tick rate be configurable? Recommend yes (5 ms / 10 ms / 100 ms options) but default to 10 ms.
- Is the cross-worker timer dispatch path performant enough? Recommend benchmarking; if not, per-worker wheels with replicated "global" timers.
