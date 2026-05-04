# 04.b LLD — Scheduling

> The accept loop, worker shards, request queue, and (optional) work-stealing scheduler.
>
> Status: **outline-stage**. Filled out as `v0.1` (per-core) and `v0.2` (work-stealing) land.

## Purpose

Distribute incoming requests across worker threads with predictable, low-tail-latency behavior. Avoid lock contention. Avoid cache-line bouncing. Surface backpressure cleanly when workers cannot keep up.

## Trait surface

```rust
// Sketch
pub trait Scheduler: Send + Sync {
    fn submit(&self, task: Task);
    fn run(&self);  // Worker entry point
}

pub trait Queue<T>: Send + Sync {
    fn push(&self, item: T) -> Result<(), T>;  // Returns item back on full
    fn pop(&self) -> Option<T>;
    fn len(&self) -> usize;  // Approximate; lock-free
}
```

## Implementations

| Impl | Type | Status | Notes |
|------|------|--------|-------|
| `PerCoreScheduler` | thread-per-core | `v0.1` | Default. One worker per core, sharded queues. |
| `WorkStealingScheduler` | work-stealing | `v0.2` (opt-in) | Chase-Lev deque per worker; FIFO steal. |
| `MpmcQueue<T>` | lock-free MPMC | `v0.2` | Vyukov-style sequence numbers. |
| `ShardedMpmcQueue<T>` | sharded MPMC | `v0.2` | One MPMC per worker shard for accept→worker handoff. |

Decision rationale: [Options 003 (concurrency)](../05-options/003-concurrency-model.md), [Options 004 (queue)](../05-options/004-request-queue.md).

## Component context

### Architecture and dependencies

The scheduler depends on the [`io-runtime`](lld-io-runtime.md) (events drive task submission) and the [`timers`](lld-timers.md) (per-request deadlines). Workers consume from the `Queue<Task>` and execute through the rest of the data plane (parser → filter → router → response).

### Patterns and conventions

- Per-core ownership: each worker owns a CPU core (pinned via `sched_setaffinity` on Linux).
- One MPMC queue per worker for the accept→worker handoff (sharded to avoid contention).
- Work-stealing is opt-in. Default is no stealing — predictable tail latency over peak throughput.
- Cache-line padding (`#[repr(align(64))]`) on shared atomics to avoid false sharing.

### Pitfalls

- **False sharing** on shared atomics is the most common performance regression in this code. Padding is mandatory.
- **CPU affinity drift** if the OS rebalances. Pinning helps but is not absolute.
- **Steal cost**: stealing a task moves data across cores; the cache hit cost can dominate the work being stolen for tiny tasks. We deliberately avoid stealing very small tasks.
- **Memory ordering**: MPMC queue uses `Acquire` / `Release` ordering; misuse produces real bugs. Tests with `loom` cover the critical paths.

### Standards and review gates

- All scheduler changes require a benchmark run on the `accept→echo` workload.
- Lock-free queue changes require `loom` tests.
- New `Scheduler` impls need a conformance pass against the integration test suite.

## Testing strategy

- `loom` tests for the lock-free queue.
- Throughput vs core count benchmark — verify near-linear scaling.
- Tail latency under heterogeneous workload mix — verifies work-stealing claim.
- Long-running soak with intermittent OS load to surface affinity bugs.

## Open questions

- Should the scheduler interleave timer ticks with task execution, or run timers on a dedicated thread? Recommend per-worker timer queues with cross-worker dispatch via the same MPMC.
- Should we support yielding tasks (cooperative multitasking)? Recommend no for `v0.x`; tasks complete or yield via async I/O.
