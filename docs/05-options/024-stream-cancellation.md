# 024. Stream cancellation

> **Status:** `recommended` — `v0.3` adopts `tokio_util::sync::CancellationToken` as the cancellation primitive and a per-stream FSM that owns the cancel transition; hedged routing ([Options `025`](025-v03-routing-strategies.md)) and client-disconnect handling are the two primary consumers. See [ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md).
> **Foundational topics:** structured concurrency (Kotlin / Trio `nursery` lineage; Eric Niebler's "Sender/Receiver" model), table-driven FSMs for protocol cancel transitions, `connection: close` semantics for HTTP/1.1 mid-stream termination, cooperative-cancellation contracts in async runtimes (Tokio, smol, async-std), the C++ `std::stop_token` design (`P0660R10`).
> **Related options:** [`007 — protocol parser`](007-protocol-parser.md) (the SSE framer that observes cancellation), [`008 — stream framing`](008-stream-framing.md), [`025 — v0.3 routing strategies`](025-v03-routing-strategies.md) (hedged requests are the primary consumer), [`016 — extension mechanism`](016-extension-mechanism.md) (filters that abort mid-stream)
> **Related ADR:** [ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md)

## 1. The decision in one sentence

> What primitive does Riftgate `v0.3` adopt for *cooperative mid-stream cancellation* of in-flight upstream requests, and how is that primitive plumbed through the request-routing, parsing, and SSE-forwarding paths without invalidating the existing trait surface?

## 2. Context — what forces this decision

`v0.1` and `v0.2` have no cancellation story. A request that has started streaming from an upstream backend runs to completion or to a connection-level error; there is no in-band way for the gateway to say "stop, I do not need the rest of this response." The data path drops the response future and the connection eventually closes, which is correct for the only consumer (`v0.1` + `v0.2` always have exactly one in-flight upstream per client request).

`v0.3` breaks that invariant in two places, and a third one is sitting in plain sight:

1. **Hedged requests** ([Options `025`](025-v03-routing-strategies.md)) fire the same client request to two backends and take whichever returns first. The slower one needs to be cancelled cleanly — not abandoned, not left to time out — because the cost of running both backends to completion on every hedged request is unacceptable (see [Options `010`](010-routing-strategy.md) §3.5).
2. **Filter chain `Terminate`** ([Options `016`](016-extension-mechanism.md)) on the response side: a filter can decide mid-stream that the response should not continue (e.g. a token-budget guard that detects the response is about to exceed quota). The upstream stream must be stopped, not just discarded.
3. **Client disconnect.** The HTTP client closing the connection mid-stream is observable from the inbound socket; today the binary notices only when the next write fails. `v0.3` is the right milestone to formalise this signal because the same primitive — a cooperative cancellation token threaded through the request lifecycle — handles all three.

Three forces frame the choice:

- **The data plane already runs on Tokio.** [ADR `0003`](../06-adrs/0003-tokio-multithread-default.md) committed `v0.1` (and `v0.2`'s binary) to the multi-thread Tokio runtime. Whatever cancellation primitive we adopt must compose with Tokio's task model — drop semantics, `select!`, abort handles — or we pay an enormous integration tax.
- **The cancel transition is a state-machine transition.** The SSE framer ([Options `008`](008-stream-framing.md)) is already an FSM ([ADR `0007`](../06-adrs/0007-handrolled-fsm-parser.md)). Cancellation is a transition into a terminal `Cancelled` state from any non-terminal state. Treating cancellation as just-another-error is incorrect; it has a distinct semantics — *we asked for it* — that should be observable in telemetry and recoverable in replay.
- **Cancellation is asynchronous and concurrent.** Two backends streaming SSE in parallel; the gateway needs to (a) cancel the slower one, (b) drain the inbound socket safely, (c) emit a `connection: close` to the upstream so the backend can free its resources, and (d) record the cancellation cause for telemetry. Doing all four atomically is not free; the choice of primitive determines how hard it is.

Requirements this is load-bearing for:

- **`FR-202`** — hedged-request support; the cancellation contract is FR-202's prerequisite.
- **`FR-204`** — filters that terminate mid-stream must release upstream resources.
- **`NFR-P10`** — the cancellation hot path must complete within 1ms of the trigger at p99.
- **`NFR-OBS05`** — every cancellation event records `cause`, `initiator`, and `bytes_seen_before_cancel`.

## 3. Candidates

### 3.1. Future-drop (the implicit baseline)

**What it is.** No explicit cancellation primitive. The gateway holds a `Future` representing the upstream request; if it drops the future, the socket-read closes, the connection is destroyed, and the upstream backend eventually notices and frees its state.

**Why it's interesting.**
- Zero new code. Today's implicit behaviour.
- Rust's `Drop`-based resource management is sound and well-understood.
- Composes trivially with `select!` and `tokio::spawn`'s `JoinHandle::abort`.

**Where it falls short.**
- **No telemetry.** A dropped future emits no event. We cannot distinguish "client disconnected" from "filter aborted" from "hedged loser." `NFR-OBS05` is unmeetable.
- **No graceful upstream signal.** The upstream backend learns of the cancellation by socket close, which can be slow (TCP keepalives) or absent (HTTP/2 streams require an explicit `RST_STREAM`). The cancellation cost lingers in upstream resources.
- **`Drop` runs synchronously.** Cleanup that needs to await (flush a telemetry event, return a connection to a pool, write a WAL cancel record) cannot be done from a drop impl. We end up with parallel cleanup paths.
- **Hedged-loser cancellation is racy.** Selecting "first to emit a token wins" via `select!` and dropping the loser works for the data, but the loser's tail end (sending `connection: close`, emitting a span) is on the dropped task's heap, with no awaiting reaper. Heap-after-free in the logical sense, not the memory sense.

**Real-world systems that use it.** Most early Tokio code. Reqwest used to be this shape. It is the right v0.1 stance; it is not the right v0.3 stance.

### 3.2. `tokio_util::sync::CancellationToken`

**What it is.** A clonable, lazily-signalable token from `tokio-util`. Calling `token.cancel()` flips a flag observable via `token.cancelled().await`. Tokens form a tree: a parent cancels all children atomically; a child can be cancelled without affecting the parent.

The contract is *cooperative*: the awaitee chooses where to observe the token (typically via `tokio::select! { _ = token.cancelled() => ..., x = upstream_read => ... }`). The runtime does not interrupt; the awaitee chooses to return.

**Why it's interesting.**
- **First-class Tokio integration.** Same crate as the runtime; battle-tested in `axum`, `tonic`, `reqwest`, `hyper`'s server side. The shape is what existing Rust async devs expect.
- **Trees match the request lifecycle.** A parent token per request; child tokens per upstream attempt (hedge), per filter-stage, per inbound-socket-read. Cancelling the parent cleanly cancels all children. This maps the structured-concurrency model (Trio's `nursery`, Kotlin's `coroutineScope`) onto Tokio.
- **Cheap.** A `CancellationToken` is an `Arc<AtomicBool + Mutex<Waker list>>`; cloning is one atomic increment; cancellation is one atomic store plus waker wakeups.
- **Observable.** The token's cancel cause is *not* a built-in field — but the natural pattern (a token paired with a `CancelCause` enum stored in an `Arc<OnceLock<CancelCause>>`) extends it without subclassing.
- **Allows async cleanup.** A cancelled task can `await` cleanup (drain socket, flush telemetry) before returning. Unlike `Drop`, cleanup is in the normal control flow.

**Where it falls short.**
- **Cooperative, not preemptive.** A filter that loops without yielding (or, worse, a WASM filter without fuel) never observes the token. We mitigate via wasmtime's fuel/wallclock limits (Options `016` §6) and document the contract: every native await point on the data path must observe the token.
- **No structural compile-time enforcement.** Forgetting to wire the token into a `select!` is a runtime correctness bug. We mitigate with a clippy lint via review gates and integration tests that assert observable cancellation latency.
- **Token cloning means we hand out write capability.** Anyone holding a clone can cancel the request. We mitigate by wrapping the token in a thin newtype (`Cancellation`) that exposes only `is_cancelled()` and `cancelled().await` to most call sites; only the request driver holds the writable handle.

**Real-world systems that use it.** Axum routers; tonic gRPC streaming; reqwest 0.12+; `hyper` v1 server-side accept loop. The community converged on this primitive between 2022 and 2024.

### 3.3. `tokio::task::AbortHandle` (preemptive task abort)

**What it is.** Spawning a Tokio task returns a `JoinHandle`; calling `handle.abort()` causes the task to panic-abort at its next yield point, after which `handle.await` returns `JoinError::is_cancelled()`.

**Why it's interesting.**
- Built into the runtime; no extra crate.
- Forces cancellation: a task that calls `abort` will, at its next `.await`, terminate without running the rest of its body.
- Models the hedged-loser case directly: spawn both upstream attempts as tasks, abort the loser.

**Where it falls short.**
- **All-or-nothing.** The cancelled task gets no opportunity to run cleanup code synchronously. Telemetry, WAL writes, and connection-return-to-pool all happen via `Drop`, which loses async-await again.
- **Coarse-grained.** AbortHandle abort the whole task, not a sub-step. A filter that wants to cancel just the upstream stream without affecting parent telemetry has nowhere to express that.
- **Composes poorly with `select!`.** The natural pattern with abort is "spawn, hold handle, abort"; the natural pattern with our existing code is "select! over alternatives." Mixing the two is error-prone.
- **Loses the cause.** `JoinError` does not carry an application-defined cancel cause.

**Real-world systems that use it.** Some Tokio user code; rarely as the only cancellation mechanism in a streaming proxy. AbortHandle is correct for short-lived tasks where preemption is wanted, not for the long-running request lifecycle we are designing.

### 3.4. Hand-rolled cancellation flag (`Arc<AtomicBool>`)

**What it is.** A bespoke flag per request. The driver sets it; awaitees poll it from inside loops; structured-concurrency primitives are reimplemented locally.

**Why it's interesting.**
- Minimal dependency surface.
- Full control over the API shape.

**Where it falls short.**
- **Reinvents `CancellationToken` poorly.** No waker integration means awaitees have to poll, which adds latency or wastes CPU.
- **No structural cancellation cascade.** Parent/child trees become a hand-rolled `Vec<Arc<AtomicBool>>` that we must maintain.
- **Loses ecosystem alignment.** Code reviewers and external contributors expect `CancellationToken`; meeting that expectation with a bespoke shape costs them and us.

**Real-world systems that use it.** Some internal Rust services pre-2022; rarely accepted in modern Rust async code.

### 3.5. Explicit `cancel_token: oneshot::Sender<CancelCause>`

**What it is.** A `tokio::sync::oneshot` channel per request; the driver holds the sender; the awaitee selects on `recv()` along with the upstream stream. Cancelling sends a `CancelCause` payload; the awaitee observes both the signal *and* the cause atomically.

**Why it's interesting.**
- Cancellation carries the cause for free.
- One-shot semantics match "this request is cancelled exactly once."
- No external crate.

**Where it falls short.**
- **No tree structure.** Parent/child cascading is hand-rolled.
- **Sender/receiver are not clonable.** Multiple awaitees (filter, parser, socket-read, upstream-write) cannot all listen on the same channel directly; we end up broadcasting, which means a second crate (`tokio::sync::broadcast`) and back to where we started.
- **The cancellation cause is the wrong layer.** The driver knows the cause; the awaitee usually does not need to discriminate (it just needs to stop). Carrying the cause through every cancellation observation is over-coupled.

**Real-world systems that use it.** Some small-scope cancellation paths inside larger systems. Rarely the top-level abstraction.

## 4. Tradeoff matrix

| Property | 3.1 Future-drop | 3.2 CancellationToken | 3.3 AbortHandle | 3.4 Hand-rolled AtomicBool | 3.5 oneshot + cause | Why it matters |
|---|---|---|---|---|---|---|
| Tokio-native | yes (implicit) | **yes (first-class)** | yes | yes | yes | `NFR-P10` 1ms latency. |
| Allows async cleanup | no | **yes** | no | yes | yes | Drain socket + flush telemetry on cancel. |
| Parent/child cascade | no | **yes** | no | manual | no | Hedged + filter + client-disconnect share a parent. |
| Carries cancel cause | no | sidecar pattern | no | sidecar | yes | `NFR-OBS05`. |
| Cooperative vs preemptive | n/a | cooperative | preemptive | cooperative | cooperative | Filters can stage cleanup. |
| Cooperative-miss risk | n/a | exists (mitigated) | n/a | exists | exists | Misbehaving filters. |
| Hot-path cost | zero | one atomic load per `select!` | runtime-internal | one atomic load | channel poll | NFR-P09. |
| Maintenance burden | zero | low (external crate) | zero | high | medium | Reviewer attention. |
| Community alignment | weak | **strong** | medium | weak | weak | External contributors. |
| Ecosystem precedent | thin | axum, tonic, hyper v1 | rare | thin | thin | Where the industry is. |
| Hedged-loser cancellation | racy | clean | clean but cleanup-coarse | manual | clean | FR-202 prerequisite. |
| Filter `Terminate` mid-stream | impossible to plumb | natural | possible (drops state) | possible | possible | FR-204. |

## 5. Foundational principles

**Structured concurrency (Trio's `nursery`; Kotlin's `coroutineScope`; Niebler's *Sender/Receiver*).** A request has a hierarchical lifetime: the parent task supervises children (upstream attempts, filter executions). Children cannot outlive the parent. Cancellation cascades down the tree. The `CancellationToken` shape is the Tokio expression of this principle; `child_token()` is the API that builds the tree.

**Cooperative cancellation (C++ `std::stop_token`, P0660R10).** The cancellation contract is "the awaitee chooses when to observe the signal." This is the same contract as `std::jthread`'s `std::stop_token`: the cancellable code is required to poll at well-defined points, and the runtime does not interrupt. This is deliberately weaker than POSIX thread cancellation (which is preemptive and famously unsound around resource cleanup). We adopt the weaker, sound contract.

**Table-driven FSM transitions (`v0.1` SSE framer).** Cancellation is *not* an error; it is a distinct terminal state. The SSE framer's existing FSM ([ADR `0007`](../06-adrs/0007-handrolled-fsm-parser.md)) gains a `Cancelled { bytes_seen, cause }` terminal state alongside `Done` and `Error`. The cancel transition is observable in the LLD as a row in the transition table.

**`connection: close` semantics for graceful upstream termination.** HTTP/1.1 streaming responses are terminated by socket close. To free upstream resources promptly without abandoning the connection mid-frame, the gateway sends `connection: close` before closing — this signals the backend that the connection is being deliberately retired and lets it drop any half-formed response state on its side. HTTP/2 uses `RST_STREAM` with `CANCEL` (`0x8`). The cancellation contract is layered such that the SSE framer's terminal state triggers the right upstream signal.

**Sidecar cancel cause (the `Cancellation` newtype).** `CancellationToken` itself does not carry a cause. The Riftgate pattern wraps a token plus an `Arc<OnceLock<CancelCause>>`:

```rust
pub struct Cancellation {
    token: CancellationToken,
    cause: Arc<OnceLock<CancelCause>>,
}

pub enum CancelCause {
    HedgedLoser { winner: BackendId },
    FilterTerminate { filter: &'static str, status: StatusCode },
    ClientDisconnect,
    UpstreamTimeout { backend: BackendId },
    Shutdown,
}
```

The `Cancellation::cancel(cause)` method records the cause atomically with the token flip; observers retrieve the cause via `Cancellation::cause()` for telemetry. This pattern is the cancellation-with-cause idiom standard in Go (`context.Context.Err()`) ported to Rust.

## 6. Recommendation

**`v0.3` adopts `tokio_util::sync::CancellationToken` as the cancellation primitive, wrapped in a Riftgate-owned `Cancellation` newtype that adds a typed cancel cause, and threads the token through the request lifecycle as a child-of-request-root token.**

Concretely:

1. **New types in `riftgate-core::cancel`:**

   ```rust
   pub struct Cancellation { /* token + Arc<OnceLock<CancelCause>> */ }

   impl Cancellation {
       pub fn root() -> (CancellationDriver, Cancellation);
       pub fn child(&self) -> Cancellation;
       pub fn is_cancelled(&self) -> bool;
       pub async fn cancelled(&self) -> CancelCause;
       pub fn cause(&self) -> Option<CancelCause>;
   }

   pub struct CancellationDriver { /* private writable half */ }
   impl CancellationDriver {
       pub fn cancel(self, cause: CancelCause);
   }

   pub enum CancelCause { HedgedLoser { winner: BackendId }, FilterTerminate { /* ... */ }, ClientDisconnect, UpstreamTimeout { /* ... */ }, Shutdown }
   ```

   `Cancellation` is freely clonable and read-only. `CancellationDriver` is owned by the request driver — typically the per-request task in `crates/riftgate/src/proxy.rs` — and gives exactly-one party the right to cancel.

2. **The `Request` struct gains a `cancel: Cancellation` field.** Filters, routers, and the upstream client receive read-only access. The request driver holds the driver half.

3. **The SSE framer ([Options `008`](008-stream-framing.md)) gains a terminal `Cancelled` state.** The existing `feed()` method polls `cancel.is_cancelled()` on each token boundary; on observed cancellation, it returns `Emit::Cancelled { bytes_seen, cause }` and the upstream connection driver issues `connection: close` (HTTP/1.1) or `RST_STREAM CANCEL` (HTTP/2 in v0.4+).

4. **The upstream client wraps every blocking await in `select! { _ = cancel.cancelled() => ..., x = upstream_op => ... }`.** This is mechanical; we ship a small helper `Cancellation::race(self, fut)` that does the select pattern in one line at every call site.

5. **Telemetry on cancellation.** Every cancellation emits an OTel span with attributes:
   - `riftgate.cancel.cause` — discriminant of `CancelCause`.
   - `riftgate.cancel.initiator` — `"driver"` (the request driver) or `"observer"` (an awaitee that detected the cancel).
   - `riftgate.cancel.bytes_seen_before_cancel` — for hedge analysis.
   - `riftgate.cancel.latency_us` — from `CancellationDriver::cancel()` to first observed cancellation point.

6. **`NFR-P10` test gate.** An integration test asserts that the median cancel-to-observed latency under sustained load is < 200µs and the p99 is < 1ms, on a Linux developer-grade host.

7. **Compliance via lint and review.** Clippy custom lint (or a justifying review-comment convention) flags `.await` on a `Pin<Box<dyn Stream>>` in the data path that is not gated by a `select!` with `cancel.cancelled()`. Until we have the lint, we document the requirement and the CODEOWNERS reviewer enforces.

8. **Backward compatibility.** The `Filter` trait gains no new method; `Router::on_response` gains no new parameter. The `Cancellation` lives inside `Request`. Existing `IdentityFilter` and `LoggingFilter` keep working without modification.

### Conditions under which we'd revisit

- If `CancellationToken` is deprecated or moved out of `tokio-util` in a major Tokio release, we re-evaluate the engine but not the contract. The Riftgate `Cancellation` newtype absorbs the change.
- If we add a non-Tokio runtime (e.g. glommio or monoio behind a feature flag, per [Options `002`](002-async-runtime.md)), we re-evaluate the primitive's runtime-coupling and may introduce a runtime-neutral trait `Cancellable` with Tokio and non-Tokio impls.
- If a starvation pattern emerges where cooperative cancellation misses regularly take longer than `NFR-P10` allows, we adopt a hybrid: the cooperative token plus a deadline-driven `AbortHandle` as a last-resort fallback.

## 7. What we explicitly reject

- **Future-drop as the v0.3 cancellation mechanism.** No telemetry, no graceful upstream signal, no async cleanup. Catalogued; remains the correct v0.1/v0.2 stance.
- **`AbortHandle` as the primary cancellation mechanism.** Coarse-grained, loses async cleanup, loses cause. Catalogued; remains useful as a hard-deadline fallback (see Conditions to revisit).
- **A hand-rolled `Arc<AtomicBool>`.** Reinvents `CancellationToken` poorly and loses community alignment.
- **`oneshot<CancelCause>` as the primary mechanism.** Cause-on-the-channel over-couples cause-discrimination across awaitees that do not need it. We adopt the sidecar-cause pattern instead, keeping the token observation cause-free at the hot path.
- **Preemptive cancellation via runtime trickery.** No `tokio_unstable` features, no custom executor patches. The cooperative contract is sound; preemptive cancellation is the C/POSIX-pthread-cancel mistake we decline to relive.
- **Cancellation as an error.** A cancelled stream is not an error from the gateway's perspective — it is a deliberate, observable, recoverable transition. The FSM gives it its own terminal state; replay treats it as a distinct outcome from `Error`.

## 8. References

1. Tokio, *`tokio_util::sync::CancellationToken`* documentation — <https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html>
2. Tokio, *Graceful shutdown with `CancellationToken`* — <https://tokio.rs/tokio/topics/shutdown>
3. ISO C++, *P0660R10: A Cooperatively Interruptible Joining Thread* (`std::jthread`, `std::stop_token`) — <https://wg21.link/p0660r10>
4. Nathaniel J. Smith, *Notes on structured concurrency, or: Go statement considered harmful* (Trio rationale) — <https://vorpus.org/blog/notes-on-structured-concurrency-or-go-statement-considered-harmful/>
5. Eric Niebler et al., *P2300R7: `std::execution`* (Sender/Receiver, including stop tokens) — <https://wg21.link/p2300r7>
6. Roman Elizarov, *Coroutines: Cancellation and Timeouts* (Kotlin) — <https://kotlinlang.org/docs/cancellation-and-timeouts.html>
7. Go language design, *`context` package* — <https://pkg.go.dev/context>
8. RFC 7230 §6, *HTTP/1.1: Message Syntax and Routing — Connection management* — <https://datatracker.ietf.org/doc/html/rfc7230#section-6>
9. RFC 9113 §6.4, *HTTP/2 — RST_STREAM* — <https://datatracker.ietf.org/doc/html/rfc9113#section-6.4>
10. Carl Lerche et al., *axum* graceful-shutdown patterns — <https://github.com/tokio-rs/axum>
11. hyper v1 server documentation, *connection lifecycle* — <https://hyper.rs/guides/1/server/graceful-shutdown/>
12. POSIX, *pthread_cancel(3)* — <https://man7.org/linux/man-pages/man3/pthread_cancel.3.html> (the preemptive mistake we decline to repeat).
