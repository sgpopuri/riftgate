# 03.a Data Plane

> The per-request hot path. Detailed lifecycle, concurrency, memory model, and contracts.
>
> Status: **outline-stage**. Filled out as `v0.1` lands.

## What lives here

- The `accept` loop and the IO subsystem (`AsyncIO` impls)
- The streaming parser (`StreamParser`)
- The request queue (`Queue<T>`)
- The scheduler and worker shards (`Scheduler`)
- The per-request allocator (`Allocator`)
- The timer subsystem (`TimerSubsystem`)
- The request log (`WAL`)

## Per-request lifecycle (detailed)

For now, see the sequence diagram in [`hld.md`](hld.md#3-per-request-lifecycle). A detailed phase-by-phase narrative will land here as `v0.1` implementation reaches each phase.

## Concurrency invariants

- The accept loop is single-threaded by design.
- Workers are per-core; no shared mutable state between workers in the hot path.
- The MPMC queue between accept and workers is the only cross-core synchronization point in the steady state.
- Timer ticks are per-core; cross-core timer events are routed via the same MPMC queue.

## Memory model

Per-request arena for everything allocated during the request lifecycle. Long-lived state (the WAL writer, the metrics aggregator) uses normal allocation. The arena is returned to a free pool on request completion. See [`../04-design/lld-allocator.md`](../04-design/lld-allocator.md).

## Failure model

- Parser errors → 400 with a structured body, no impact on other requests.
- Backend connection failures → trip the circuit breaker, return 502 or attempt fallback per routing strategy.
- Timer-triggered request timeouts → 504, log to WAL with timeout marker.
- Internal panics → process abort with a structured log line. We do not catch panics in the hot path; correctness over availability for unrecoverable bugs.

## What is NOT in the data plane

- Configuration mutation (lives in the [control plane](control-plane.md))
- Filter compilation (lives in the [extension plane](extension-plane.md))
- Metric aggregation (lives in the [observability plane](observability-plane.md))

This separation matters. A change in the data plane must not require changes in two other planes; if it does, the plane boundaries are wrong.

## Open questions for `v0.0` design phase

- Should the request body be parsed lazily or eagerly? (Affects when filters can intercept.)
- Should the WAL append be synchronous (blocking the response) or asynchronous (best-effort)? Recommend asynchronous with a configurable durability mode.
- How do we handle very large prompts (>1 MB)? Streaming parser should support this without buffering the whole body.
