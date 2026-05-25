# 04.g LLD — Routing

> Backend selection: which backend gets which request. Pluggable strategies behind a single trait.
>
> Status: **shipped (v0.1, RoundRobin + ConstantRouter); v0.2 adds WeightedRandomRouter and the CircuitBreakerArbiter decorator.** KV-aware and hedged routers are explicitly deferred to v0.3 per [Options `010`](../05-options/010-routing-strategy.md) and [ADR `0014`](../06-adrs/0014-weighted-random-router.md).

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
| `KvAwareRouter` | deferred to v0.3 | `riftgate-router` | Integrates with `vllm-router` LMCache or uses an internal prefix trie; paired with the v0.3 WASM extension surface. |
| `HedgedRouter` | deferred to v0.3 | `riftgate-router` | Wraps any inner router; emits `Hedge(...)` decisions; requires v0.3 stream-cancellation primitives. |

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

- **KV-aware routing requires tokenization.** If the gateway tokenizes, we add latency on the hot path. If we delegate to the backend, we need a fast tokenize endpoint. See [Options 010](../05-options/010-routing-strategy.md).
- **Hedged requests double upstream load** in steady state. Use only on tail-latency-sensitive routes.
- **Stream cancellation is non-trivial** when both backends are streaming. The router emits a cancellation signal; the scheduler ensures the slower one gets `connection: close` mid-stream.
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

- Should we support session-affinity routing as a built-in? Recommend yes; sticky to a backend by `X-Session-Id` header.
- Should hedging be model-aware (e.g. don't hedge GPT-4 calls because of cost)? Recommend yes; per-route configuration.
- How do we surface "this routing decision was hedged" to OTel? Recommend a span attribute `riftgate.router.hedged = true` and a `riftgate.router.hedge_winner` attribute.

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
