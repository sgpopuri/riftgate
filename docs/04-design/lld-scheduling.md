# 04.b LLD — Scheduling

> The accept loop, worker shards, request queue, and (optional) work-stealing scheduler.
>
> Status: **shipped (v0.1, tokio multi-thread runtime + accept loop in `crates/riftgate/src/server.rs`)**. Custom `PerCoreScheduler` and work-stealing impls land in v0.2 behind the same trait.

## Purpose

Distribute incoming requests across worker threads with predictable, low-tail-latency behavior. Avoid lock contention. Avoid cache-line bouncing. Surface backpressure cleanly when workers cannot keep up.

## v0.1 reality

The v0.1 binary uses **tokio's multi-thread runtime** as the scheduler per [ADR 0003](../06-adrs/0003-tokio-multithread-default.md). This is a deliberate choice for the walking skeleton: tokio gives us a battle-tested executor while we build out the rest of the gateway, and the trait surface in `riftgate-core` is shaped so that a custom `PerCoreScheduler` can replace tokio in v0.2 without rippling through the request-handling code.

Concretely, v0.1 has:

- One tokio multi-thread runtime, `worker_threads` = number of cores by default.
- A single `accept` task on the runtime that loops on `TcpListener::accept` and `tokio::spawn`s a per-connection task.
- Each per-connection task runs the hyper 1.x connection driver, which calls into the proxy handler in `crates/riftgate/src/proxy.rs` per request.

The `Scheduler` and `Queue` traits in `riftgate-core` are declared but have no production impl in v0.1; they are the contract `PerCoreScheduler` will satisfy in v0.2.

## Trait surface

The trait surface — see [`crates/riftgate-core/src/scheduler.rs`](../../crates/riftgate-core/src/scheduler.rs) and [`src/queue.rs`](../../crates/riftgate-core/src/queue.rs):

```rust
pub trait Scheduler: Send + Sync {
    fn submit(&self, task: Task);
    fn run(&self);
}

pub trait Queue<T>: Send + Sync {
    fn push(&self, item: T) -> Result<(), T>;
    fn pop(&self) -> Option<T>;
    fn len(&self) -> usize;
}
```

Both are `Send + Sync` because the v0.2 plan is to share an `Arc<dyn Scheduler>` for cross-shard task submission. v0.1 does not exercise this path; tokio's runtime handle covers the same role.

## Implementations

| Impl | Type | Status | Notes |
|------|------|--------|-------|
| tokio multi-thread runtime | work-stealing (tokio) | shipped (v0.1, default) | Per [ADR 0003](../06-adrs/0003-tokio-multithread-default.md). The walking-skeleton scheduler. |
| `PerCoreScheduler` | thread-per-core | v0.2 | Custom `Scheduler` impl. One worker per core, sharded queues, no work-stealing by default. |
| `WorkStealingScheduler` | work-stealing | v0.2 (opt-in) | Chase-Lev deque per worker; FIFO steal. Behind a Cargo feature. |
| `MpmcQueue<T>` | lock-free MPMC | v0.2 | Vyukov-style sequence numbers. |
| `ShardedMpmcQueue<T>` | sharded MPMC | v0.2 | One MPMC per worker shard for accept→worker handoff. |

Decision rationale: [Options 003 (concurrency)](../05-options/003-concurrency-model.md), [Options 004 (queue)](../05-options/004-request-queue.md).

## Component context

### Architecture and dependencies

In v0.1 the scheduler is the tokio multi-thread runtime. It depends on tokio (which depends on mio internally, the same primitive [`riftgate-io-epoll`](lld-io-runtime.md) wraps). The accept loop and per-connection tasks live in `crates/riftgate/src/server.rs`; per-request handling lives in `crates/riftgate/src/proxy.rs`. Per-request deadlines today come from `tokio::time::timeout`; the per-shard `BinaryHeapTimers` from [`timers`](lld-timers.md) become the deadline source when v0.2 lands the custom `PerCoreScheduler`.

When v0.2 lands, the scheduler will depend on the [`io-runtime`](lld-io-runtime.md) (events drive task submission) and the [`timers`](lld-timers.md) (per-request deadlines). Workers will consume from the `Queue<Task>` and execute through the rest of the data plane (parser → filter → router → response).

### Patterns and conventions

v0.1 (tokio):

- One multi-thread tokio runtime; `worker_threads` defaults to the number of cores.
- One accept task per listening socket; per-connection tasks spawned on the same runtime.
- Backpressure is supplied by tokio's task budget; in-flight count is bounded by the connection budget configured in `riftgate-config`.
- Graceful shutdown: a `tokio::sync::watch<bool>` flips the gateway into draining state on SIGTERM/SIGINT, the accept task stops accepting new connections, and the runtime is given a `--drain-grace-ms` budget to finish in-flight work before forced shutdown.

v0.2 (custom scheduler) — guidance for the upcoming work:

- Per-core ownership: each worker owns a CPU core (pinned via `sched_setaffinity` on Linux).
- One MPMC queue per worker for the accept→worker handoff (sharded to avoid contention).
- Work-stealing is opt-in. Default is no stealing — predictable tail latency over peak throughput.
- Cache-line padding (`#[repr(align(64))]`) on shared atomics to avoid false sharing.

### Pitfalls

v0.1:

- **The tokio runtime is shared.** All connections, all timers, the observability bus worker — everything runs on the same multi-thread runtime. A blocking call (`std::fs::read`, a sync mutex held too long) starves the entire gateway. Use `tokio::task::spawn_blocking` for any blocking work.
- **`tokio::time::timeout` is per-task.** When the v0.2 `PerCoreScheduler` lands and replaces tokio, request deadlines move to the per-shard `BinaryHeapTimers`. Code that hard-codes `tokio::time::timeout` will need to migrate behind the `TimerSubsystem` trait.

v0.2 guidance:

- **False sharing** on shared atomics is the most common performance regression in this code. Padding is mandatory.
- **CPU affinity drift** if the OS rebalances. Pinning helps but is not absolute.
- **Steal cost.** Stealing a task moves data across cores; the cache hit cost can dominate the work being stolen for tiny tasks. Avoid stealing very small tasks.
- **Memory ordering.** MPMC queue uses `Acquire` / `Release` ordering; misuse produces real bugs. Tests with `loom` will cover the critical paths.

### Standards and review gates

- All scheduler changes require a benchmark run on the `accept→echo` workload (see [`crates/riftgate/benches/end_to_end.rs`](../../crates/riftgate/benches/end_to_end.rs)).
- Lock-free queue changes (v0.2) require `loom` tests.
- New `Scheduler` impls (v0.2) need a conformance pass against the end-to-end test suite in [`crates/riftgate/tests/e2e.rs`](../../crates/riftgate/tests/e2e.rs).

## Testing strategy

v0.1:

- The end-to-end test suite in [`crates/riftgate/tests/e2e.rs`](../../crates/riftgate/tests/e2e.rs) exercises the full accept-loop → proxy handler → upstream → response path against a mock backend, covering FR-001 through FR-008.
- The microbenchmark in [`crates/riftgate/benches/end_to_end.rs`](../../crates/riftgate/benches/end_to_end.rs) measures the steady-state request latency of the v0.1 binary.

v0.2:

- `loom` tests for the lock-free queue.
- Throughput vs core count benchmark — verify near-linear scaling.
- Tail latency under heterogeneous workload mix — verifies work-stealing claim.
- Long-running soak with intermittent OS load to surface affinity bugs.

## Open questions

- Should the scheduler interleave timer ticks with task execution, or run timers on a dedicated thread? Recommend per-worker timer queues with cross-worker dispatch via the same MPMC.
- Should we support yielding tasks (cooperative multitasking)? Recommend no for v0.x; tasks complete or yield via async I/O.
- When does the custom `PerCoreScheduler` actually replace tokio? Likely v0.2 once we have a benchmarked baseline; the ADR will spell out the migration window.
