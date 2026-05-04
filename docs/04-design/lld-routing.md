# 04.g LLD — Routing

> Backend selection: which backend gets which request. Pluggable strategies behind a single trait.
>
> Status: **outline-stage**. Filled out as `v0.2` (RR, weighted) and `v0.3` (KV-aware, hedged) land.

## Purpose

Decide which backend (or backends) should serve each request, given the current backend pool, the request properties (model, prompt, headers), and any signals from the observability plane (backend health, GPU pressure).

## Trait surface

```rust
// Sketch
pub enum RoutingDecision {
    Send(BackendId),
    Hedge(Vec<BackendId>),       // race; first to respond wins
    Reject(StatusCode),
}

pub struct BackendSignal {
    pub circuit_state: CircuitState,
    pub gpu_pressure: Option<f32>,        // 0.0-1.0, if available (v0.4+)
    pub recent_p99_ms: f32,
}

pub trait Router: Send + Sync {
    fn route(&self, req: &Request, pool: &BackendPool, signals: &BackendSignals) -> RoutingDecision;
    fn on_response(&self, _decision: &RoutingDecision, _outcome: &Outcome) {}
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `RoundRobinRouter` | `v0.1` | `riftgate-router` | Trivial. The default. |
| `WeightedRandomRouter` | `v0.2` | `riftgate-router` | Per-backend weights. |
| `KvAwareRouter` | `v0.3` | `riftgate-router` | Integrates with `vllm-router` LMCache OR uses an internal prefix trie. |
| `HedgedRouter` | `v0.3` | `riftgate-router` | Wraps any inner router; emits `Hedge(...)` decisions. |

Decision rationale: [Options 010 (routing strategy)](../05-options/010-routing-strategy.md).

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

Reference: `trees/ch05 (tries and string trees)`.

Two places a trie pays off inside routing:

1. **KV-cache-aware routing (`KvAwareRouter`).** Identical prompt prefixes should, when possible, land on the same backend — so the backend's KV cache hits. A prefix trie over tokenized prompts with backend-id leaves, matched greedily, gives us this behavior without claiming to be vLLM. The trie lives in the router itself when we are not delegating to `vllm-router`'s LMCache controller.
2. **Route-path dispatch.** The top-level HTTP path router (`/v1/chat/completions`, `/v1/embeddings`, `/v1/models`, `/mcp/*`) is a textbook radix-tree use case. An upstream candidate is `matchit` or a hand-rolled radix-tree; either way, route dispatch is O(|path|), not O(N_routes).

Both uses avoid per-request allocations by sharing the tree across requests (immutable after config load).

### Consistent hashing

Reference: `hashing/ch07 (consistent hashing)`.

Consistent hashing appears twice in the routing surface:

1. **Session-affinity routing (the open question above).** An `X-Session-Id` header mapped to a backend via consistent hashing gives stickiness without a shared session store. Bounded-load consistent hashing (Mirrokni et al., 2016) avoids hot-shard problems when backends are added or removed.
2. **Sharded rate-limit or cache future impls.** If the operator chooses to front Riftgate with a layer-7 LB that consistent-hashes by tenant, rate-limit state can stay local — see Options [`021` §3.7.3](../05-options/021-rate-limiting.md).

### Priority queue / heap

Reference: `trees/ch04 (heaps and priority queues)`.

When hedging a request, the router returns a `Vec<BackendId>`. At scheduling time, a heap ordered by (`expected_latency`, `cost`) picks the next candidate to dispatch on. This is an anticipated use once hedging matures — it is not in the initial `HedgedRouter` impl.

### What we do NOT use

- Bloom filters are NOT used in routing decisions. A false positive in "is this prefix in the KV cache" would cause a suboptimal backend choice, which is recoverable, but the memory savings over a direct trie lookup do not justify the complexity. Bloom filters are relevant in the (deferred) semantic cache, not here.
