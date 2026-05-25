# 04.g LLD — Routing

> Backend selection: which backend gets which request. Pluggable strategies behind a single trait.
>
> Status: **shipped (v0.1, RoundRobin + ConstantRouter); v0.2 adds WeightedRandomRouter and the CircuitBreakerArbiter decorator; v0.3 adds `KvAwareRouter` and `HedgedRouter` as decorator-shaped routers stacking on top of the v0.2 baseline.** v0.3 routing-strategy decisions in [Options `025`](../05-options/025-v03-routing-strategies.md), [ADR `0022`](../06-adrs/0022-kv-aware-routing-prefix-trie.md) (KV-aware), and [ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md) (hedged). The v0.2 baseline decision lives in [Options `010`](../05-options/010-routing-strategy.md) and [ADR `0014`](../06-adrs/0014-weighted-random-router.md).

## Purpose

Decide which backend (or backends) should serve each request, given the current backend pool, the request properties (model, prompt, headers), and any signals from the observability plane (backend health, GPU pressure).

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/router.rs`](../../crates/riftgate-core/src/router.rs):

```rust
pub enum RoutingDecision {
    Send(BackendId),
    Hedge(Vec<BackendId>),
    Reject(StatusCode),
}

pub struct BackendSignal {
    pub circuit_state: CircuitState,
    pub gpu_pressure: Option<f32>,
    pub recent_p99_ms: f32,
}

pub trait Router: Send + Sync {
    fn route(
        &self,
        req: &Request,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> RoutingDecision;
    fn on_response(&self, _decision: &RoutingDecision, _outcome: &Outcome) {}
}
```

The trait is `Send + Sync` (unlike `AsyncIO` and `TimerSubsystem`) because a single `Arc<dyn Router>` is shared across all shards. This is fine: `RoundRobinRouter` uses an `AtomicUsize` cursor; future stateful impls (`KvAwareRouter`) will use lock-free or sharded structures.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `RoundRobinRouter` | shipped (v0.1, default) | `riftgate-router` | Atomic cursor over `BackendPool`. The v0.1 default for the binary. Statistical fairness verified by [`crates/riftgate-router/tests/fairness.rs`](../../crates/riftgate-router/tests/fairness.rs). |
| `ConstantRouter` | shipped (v0.1) | `riftgate-router` | Always returns `Send(backend_id)`. Used as a test harness so other crates can verify routing-agnostic behavior. |
| `WeightedRandomRouter` | **v0.2** | `riftgate-router` | Walker alias method (Vose 1991); O(1) sampling regardless of weight distribution; alias table rebuilt at config-load and config-reload. Capped at N = 32 eligible backends in v0.2. Per [ADR `0014`](../06-adrs/0014-weighted-random-router.md). |
| `CircuitBreakerArbiter<R>` | **v0.2** | `riftgate-router` | Decorator over any `Router` impl; filters `CircuitState::Open` backends out of the eligible set before delegating selection. Per [ADR `0016`](../06-adrs/0016-three-state-circuit-breaker.md). |
| `KvAwareRouter<R>` | **v0.3** | `riftgate-router` | Decorator over inner `Router`; in-tree prefix trie keyed by chunked xxHash3-64 byte-hashes; LRU-bounded entry count; `prefix_normalisation = "trim_trailing_whitespace"` default; falls back to the inner router when the trie misses or names an Open backend. Per [ADR `0022`](../06-adrs/0022-kv-aware-routing-prefix-trie.md). LMCache delegation catalogued and deferred. |
| `HedgedRouter<R>` | **v0.3** | `riftgate-router` | Decorator over inner `Router`; Dean–Barroso threshold-triggered hedge, degree=2, per-backend P²-estimator for first-byte p95 latency, global `hedge_max_fraction` budget (default 0.05); loser cancelled via the v0.3 cancellation primitive ([ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md)). Per [ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md). |

Decision rationale: [Options `010` (routing strategy)](../05-options/010-routing-strategy.md), [ADR `0014`](../06-adrs/0014-weighted-random-router.md), and (for the breaker decorator) [Options `011`](../05-options/011-circuit-breaker.md) + [ADR `0016`](../06-adrs/0016-three-state-circuit-breaker.md).

## Component context

### Architecture and dependencies

The router is invoked once per request, after the request-side filter chain. It consumes the typed `Request` and the current `BackendPool` snapshot, plus signals from the observability plane.

The router does **not** dispatch the request itself — it returns a decision; the scheduler dispatches. This keeps routing pure (testable in isolation) and dispatch concentrated.

### Patterns and conventions

- **Routers are pure functions of (request, pool, signals).** Side effects only via `on_response`.
- **Per-request decision, not per-stream.** Once routed, the stream stays on the chosen backend (modulo hedge cancellation).
- **Hedging is a wrapper, not a backend property.** Any router can be hedged; the hedging policy is configured separately.
- **No router holds locks during decision.** Pool snapshots are immutable; signals are atomic loads.

### Pitfalls

- **KV-aware routing trades tokenizer accuracy for hot-path latency.** v0.3 hashes raw bytes (xxHash3-64, chunked at 64 bytes per trie level) rather than tokenizing on the routing path — a tokenizer call exceeds the 50µs `NFR-P11` budget. The trade gives up a few percentage points of hit-rate on tokenizer-divergent inputs in exchange for staying inside the hot-path budget. Documented in [Options `025` §3.A.1](../05-options/025-v03-routing-strategies.md) and [ADR `0022`](../06-adrs/0022-kv-aware-routing-prefix-trie.md). LMCache delegation remains catalogued for a future impl if operator demand surfaces.
- **Hedged requests amplify upstream load** in proportion to the trigger frequency. v0.3 caps amplification at `hedge_max_fraction = 0.05` (≤ 5% of traffic) globally; per-tenant budgets are deferred until multitenancy lands. Telemetry (`hedge.fired`, `hedge.budget_blocked`, `hedge.bytes_wasted_total`) provides the data needed to tune the trigger threshold over time — see [ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md).
- **Stream cancellation is non-trivial** when both backends are streaming. v0.3 resolves the contract via the `Cancellation` newtype around `tokio_util::sync::CancellationToken` (per [Options `024`](../05-options/024-stream-cancellation.md), [ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md)): the request driver fans out, sets a timer at `primary.p95_first_byte_ms`, dispatches the secondary only if the timer fires before the primary's first byte arrives, and cancels the loser via `Cancellation::cancel(CancelCause::HedgedLoser { winner })`. The SSE framer's `Cancelled` terminal state triggers `connection: close` on HTTP/1.1 (and `RST_STREAM CANCEL` on HTTP/2 in v0.4+).
- **Sticky sessions** (consistent-hash routing for chat-style multi-turn) are a router concern; the request must carry a session ID. Cookie-based stickiness: out of scope.

### Standards and review gates

- Router changes require integration tests against a multi-backend mock.
- Hedging tests must verify that the slow backend's stream is actually cancelled (not just abandoned).
- KV-aware router benchmarks must show measurable cache-hit improvement on a realistic prefix-distribution workload.

## Testing strategy

- Mock backends with controllable latency and failure injection.
- Statistical fairness tests for `WeightedRandomRouter`.
- Cache-hit rate tests for `KvAwareRouter`.
- Hedge-cancellation correctness tests.

## Open questions

- Should we support session-affinity routing as a built-in? Recommend yes; sticky to a backend by `X-Session-Id` header. Deferred to a future minor.
- Should hedging be model-aware (e.g. don't hedge GPT-4 calls because of cost)? Recommend yes; per-route configuration. The v0.3 chain supports per-route disable; per-route enable-with-different-budget is a future option.
- Cross-replica KV-aware consistency. Two Riftgate replicas behind an L4 LB have independent prefix tries in v0.3. Operators wanting cross-replica consistency front Riftgate with a prefix-aware L4 LB or wait for a future LMCache-delegated impl.
- Tokenizer-accurate KV routing as a v1.0+ option with measurement-driven justification.
- Streaming-aware hedge trigger. v0.3 triggers on first-byte latency; some workloads have fast TTFB but slow full-response. A v0.4+ trigger could observe an in-flight token rate and fire a hedge mid-stream. Documented as a future enhancement.

OTel surfacing for hedged decisions is now established: `riftgate.router.hedged = true`, `riftgate.router.hedge_winner = <backend_id>`, `riftgate.router.hedge_bytes_wasted = <n>` on the parent request span, and `riftgate.cancel.cause = "HedgedLoser"` on the cancelled child span (per [ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md)).

## Data structures worth citing

The routing subsystem is a meeting point for several classical data structures. A contributor reading this LLD should understand which structure is relevant where, and why.

### Prefix trie / radix tree

Reference: tries and radix / patricia trees (Knuth TAOCP §6.3; the standard data-structures literature).

Two places a trie pays off inside routing:

1. **KV-cache-aware routing (`KvAwareRouter`).** Identical prompt prefixes should, when possible, land on the same backend — so the backend's KV cache hits. A prefix trie over tokenized prompts with backend-id leaves, matched greedily, gives us this behavior without claiming to be vLLM. The trie lives in the router itself when we are not delegating to `vllm-router`'s LMCache controller.
2. **Route-path dispatch.** The top-level HTTP path router (`/v1/chat/completions`, `/v1/embeddings`, `/v1/models`, `/mcp/*`) is a textbook radix-tree use case. An upstream candidate is `matchit` or a hand-rolled radix-tree; either way, route dispatch is O(|path|), not O(N_routes).

Both uses avoid per-request allocations by sharing the tree across requests (immutable after config load).

### Consistent hashing

Reference: consistent hashing (Karger et al., 1997; Lamping–Veach jump-consistent, 2014; Maglev hashing, 2016; Mirrokni et al., bounded-load consistent hashing, 2016).

Consistent hashing appears twice in the routing surface:

1. **Session-affinity routing (the open question above).** An `X-Session-Id` header mapped to a backend via consistent hashing gives stickiness without a shared session store. Bounded-load consistent hashing (Mirrokni et al., 2016) avoids hot-shard problems when backends are added or removed.
2. **Sharded rate-limit or cache future impls.** If the operator chooses to front Riftgate with a layer-7 LB that consistent-hashes by tenant, rate-limit state can stay local — see Options [`021` §3.7.3](../05-options/021-rate-limiting.md).

### Priority queue / heap

Reference: binary and d-ary heaps, priority queues (CLRS ch. 6).

When hedging a request, the router returns a `Vec<BackendId>`. At scheduling time, a heap ordered by (`expected_latency`, `cost`) picks the next candidate to dispatch on. This is an anticipated use once hedging matures — it is not in the initial `HedgedRouter` impl.

### What we do NOT use

- Bloom filters are NOT used in routing decisions. A false positive in "is this prefix in the KV cache" would cause a suboptimal backend choice, which is recoverable, but the memory savings over a direct trie lookup do not justify the complexity. Bloom filters are relevant in the (deferred) semantic cache, not here.
