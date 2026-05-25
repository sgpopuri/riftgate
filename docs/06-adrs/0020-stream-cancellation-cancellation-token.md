# ADR 0020. Stream cancellation via `tokio_util::sync::CancellationToken` wrapped in a typed `Cancellation` newtype

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [024-stream-cancellation](../05-options/024-stream-cancellation.md)
> **Deciders:** Sriram Popuri

## Context

`v0.1` and `v0.2` have no cancellation story; in-flight upstream requests run to completion or to a connection-level error. `v0.3` breaks this in two places — hedged routing ([ADR `0023`](0023-hedged-requests-p99-triggered.md)) needs to cancel the slower backend; filter `Terminate` ([ADR `0019`](0019-wasm-extension-mechanism.md)) needs to stop the upstream stream — and a third consumer (client-disconnect) becomes natural at the same time. Five primitives were evaluated in [Options `024`](../05-options/024-stream-cancellation.md): implicit future-drop, `tokio_util::sync::CancellationToken`, `tokio::task::AbortHandle`, a hand-rolled `Arc<AtomicBool>`, and `oneshot<CancelCause>`. The Rust async community converged on `CancellationToken` for streaming proxies (axum, tonic, hyper v1, reqwest); deviating without strong cause adds maintenance burden and external-contributor friction.

## Decision

**`v0.3` adopts `tokio_util::sync::CancellationToken` as the cancellation primitive, wrapped in a Riftgate-owned `Cancellation` newtype that pairs the token with a typed `CancelCause`, and threads the token through the request lifecycle as a child-of-request-root token.**

- New module `riftgate-core::cancel` exposes `Cancellation` (read-only handle, freely clonable) and `CancellationDriver` (writable half, owned by exactly one party — typically the per-request task in `crates/riftgate/src/proxy.rs`).
- `CancelCause` is a typed enum: `HedgedLoser { winner: BackendId }`, `FilterTerminate { filter: &'static str, status: StatusCode }`, `ClientDisconnect`, `UpstreamTimeout { backend: BackendId }`, `Shutdown`.
- The `Request` struct gains a `cancel: Cancellation` field. Filters, routers, and the upstream client receive read-only access.
- The SSE framer ([Options `008`](../05-options/008-stream-framing.md)) gains a terminal `Cancelled { bytes_seen, cause }` state alongside `Done` and `Error`; the FSM's transition table is extended, not bolted on.
- The upstream client wraps blocking awaits in `select! { _ = cancel.cancelled() => ..., x = upstream_op => ... }`. A helper `Cancellation::race(self, fut)` is provided for the common pattern.
- The `Filter` trait surface in `riftgate-core` is unchanged; `Router::on_response` is unchanged; backward compatibility for v0.1/v0.2 callers is preserved.

## Consequences

- **Positive:**
  - First-class Tokio integration; battle-tested in axum, tonic, hyper v1, reqwest.
  - Parent/child cascade matches the request lifecycle: cancelling the request root cancels every child (filter, upstream attempt, parser) atomically.
  - Async cleanup remains in normal control flow (drain socket, flush telemetry, return connection to pool) — unlike `Drop`-based or `AbortHandle`-based primitives.
  - Typed `CancelCause` satisfies `NFR-OBS05`: every cancellation event carries `cause`, `initiator`, and `bytes_seen_before_cancel`.
  - HTTP/1.1 termination is `connection: close` on the upstream socket; HTTP/2 termination (v0.4+) is `RST_STREAM CANCEL` — both triggered by the SSE framer's terminal `Cancelled` state.
- **Negative / accepted tradeoffs:**
  - Cancellation is **cooperative**, not preemptive: a native filter that loops without yielding never observes the token. Mitigated by wasmtime fuel/wallclock limits for WASM filters and a documented review gate for native filters (every data-path `.await` must be inside a `select!` that races `cancel.cancelled()`).
  - Forgetting to wire the token is a runtime correctness bug, not a compile error. Mitigated by a clippy lint (or, until written, by CODEOWNERS review on `crates/riftgate/` and `crates/riftgate-router/`).
  - The `Cancellation`-`CancellationDriver` split adds a small ergonomic cost vs raw `CancellationToken`; the cause-carrying property justifies it.
- **Future work this enables:**
  - Hedged routing ([ADR `0023`](0023-hedged-requests-p99-triggered.md)) cancels the loser via `Cancellation::cancel(CancelCause::HedgedLoser { winner })`.
  - Filter-side `Terminate` ([ADR `0019`](0019-wasm-extension-mechanism.md)) emits `CancelCause::FilterTerminate { filter, status }`.
  - Client-disconnect detection lands as a follow-up (still v0.3): the inbound socket's `read = 0` flips the cancel with `CancelCause::ClientDisconnect`.
  - WAL-recorded cancellations let `riftgate-replay eval` ([Options `019`](../05-options/019-replay-eval.md)) report per-cause distribution and `bytes_wasted_total`.
- **Future work this forecloses (until superseded):**
  - Riftgate will not adopt preemptive runtime cancellation tricks (`tokio_unstable` features; custom executor patches).
  - Riftgate will not treat cancellation as a flavour of error; the SSE framer's `Cancelled` state is distinct from `Error`.
  - Riftgate will not ship a runtime-neutral cancellation trait in v0.3; if a future non-Tokio runtime lands (per [Options `002`](../05-options/002-async-runtime.md)), the `Cancellation` newtype absorbs the change.

## Compliance

- `crates/riftgate-core/src/cancel.rs` is the only place `tokio_util::sync::CancellationToken` is imported; everything else uses `Cancellation` / `CancellationDriver`.
- `crates/riftgate-core/tests/cancel_cascade.rs` asserts parent→child cancellation cascade and cause propagation.
- `crates/riftgate/tests/sse_cancellation.rs` asserts the SSE framer reaches the `Cancelled` terminal state on a triggered cancel and that the upstream connection observes `connection: close`.
- `crates/riftgate/tests/cancellation_latency.rs` asserts median cancel-to-observed latency is < 200µs and p99 < 1ms on the CI reference host (`NFR-P10`).
- A clippy custom lint (or, until written, a documented review gate) flags `.await` on a `Pin<Box<dyn Stream>>` in `crates/riftgate/src/` that is not gated by a `select!` with `cancel.cancelled()`.
- New filter code (native or WASM) is reviewed for cancellation-observation coverage by CODEOWNERS on `crates/riftgate-core/src/cancel.rs`.

## Notes

- The `Cancellation` newtype is closer to Go's `context.Context` than to raw `CancellationToken`: the cause-on-cancellation pattern is well-trodden in Go and is what makes post-incident attribution tractable.
- We deliberately avoid `tokio::task::AbortHandle` because the cancelled task loses the opportunity to run async cleanup. AbortHandle remains useful as a hard-deadline fallback if cooperative cancellation misses prove a real production problem (see Conditions to revisit in Options `024`).
- The POSIX `pthread_cancel` precedent — preemptive cancellation that is famously unsound around resource cleanup — is the cautionary tale we are declining to repeat.
