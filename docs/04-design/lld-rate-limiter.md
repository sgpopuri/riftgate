# 04.i LLD — Rate limiter

> Per-instance rate limiting on the hot path. Token-bucket by default, trait-shaped so a future distributed impl (Redis / Dragonfly / sharded GCRA) can drop in without breaking callers.
>
> Status: **outline-stage**. Filled out as `v0.2` lands the first in-proc impl; revisited only if a distributed impl is pursued.

## Purpose

Decide whether each incoming request is admitted or rejected before it consumes the filter chain, the router, and the scheduler. Enforce per-route and per-backend rate policies. Emit clean `429 Too Many Requests` with `Retry-After` when denied.

Explicitly not a concern of this subsystem: cross-replica coherence, tenant-billing enforcement, circuit-breaker fallback. Those are other traits that may *compose* with this one.

## Trait surface

```rust
// Sketch — actual signatures in riftgate-core
pub struct SubjectKey {
    pub tenant: TenantId,
    pub route: RouteId,
    pub backend: Option<BackendId>,
}

pub enum LimitDecision {
    Allow,
    Deny { retry_after: Duration },
}

pub trait RateLimiter: Send + Sync {
    fn check(&self, subject: &SubjectKey, cost: u32) -> LimitDecision;
}
```

The `cost` parameter carries the multi-dimensional weight of the request: `1` for request-count limiting, `token_count(prompt)` for TPM limiting.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `TokenBucketLimiter` | `v0.2` | `riftgate-core` | Lock-free in-proc; sharded `DashMap<SubjectKey, AtomicBucketState>`. |
| `NoopLimiter` | `v0.2` | `riftgate-core` | Pass-through; used when rate-limiting is disabled in config. |
| `RedisGcraLimiter` | future | `riftgate-rate-redis` (not yet) | Behind a `rate-limit-redis` feature flag. Lua-script-atomic GCRA. |

Decision rationale, candidates, and rejected alternatives: see [Options `021` (rate limiting)](../05-options/021-rate-limiting.md).

## Component context

### Architecture and dependencies

The rate limiter sits in the data plane, between the accept loop's queue and the scheduler's worker dispatch — after parsing, before filters run. Placing it here means we reject *before* paying filter-chain cost but *after* we can identify the tenant and route (which the parser has extracted).

Dependencies are deliberately minimal:

- The [allocator](lld-allocator.md) for the sharded map backing the bucket state.
- The [timer subsystem](lld-timers.md) only for `Instant::now()` (no timer-wheel registration; refill is lazy on each check).
- The [observability plane](lld-observability.md) for counter emission (`riftgate_rate_limit_denied_total`, etc.).

No dependency on the WAL, the filter chain, the router, or the scheduler.

### Patterns and conventions

- **Pure function of (subject, cost, state).** No side effects outside the atomic bucket update.
- **Lazy refill.** No per-subject timer; on each check, compute tokens-since-last-refill.
- **Sharded internal map.** Subject hash picks a shard; each shard is its own `DashMap` to avoid a single contended structure at high subject count. Citation: `systems/ch04 (lock-free structures)`.
- **Denied requests are first-class.** They emit OTel counter events, are NOT written to the WAL (at the `Deny` level — the request never reached the dispatch path), and return `429` with `Retry-After` computed from bucket-depletion time.
- **Configuration is per-route.** Operators configure `rate_per_sec`, `burst`, and optional `cost_fn` in `riftgate.toml`. Default cost is `1`.
- **The trait is not the config.** The config is a separate `RateLimitPolicy` struct; impls of `RateLimiter` consume the policy. This keeps the trait usable when config is dynamic (`v1.0` CRD-driven config).

### Pitfalls

- **Packed atomic state.** `TokenBucketLimiter` packs `(tokens_scaled, last_refill_nanos_since_epoch)` into a single `AtomicU64`. Scaling matters — we use microtokens (tokens × 1000) to avoid float drift. Off-by-one on the scale is the most common bug class here.
- **Clock skew from monotonic to wall time.** We use `Instant::now()` (monotonic) exclusively; never mix with `SystemTime`.
- **Subject-key cardinality.** A pathological policy (`limit per (tenant, route, backend, path_suffix, header_x)`) can cause the internal map to balloon. The `SubjectKey` shape deliberately bounds this to `(tenant, route, backend)`. Future extensions must make the cost of new dimensions explicit.
- **Cost = 0.** Rejected at config validation. A zero-cost request is a free-pass attack vector.
- **Negative `Retry-After`.** Floor to `0 s` before emitting. A client that receives `Retry-After: -1` has a worse experience than one receiving `0`.
- **Shard hotspotting.** If a single tenant dominates traffic, a single shard becomes the hot path. Monitor per-shard check-latency p99 and pick a shard-count larger than typical CPU count.
- **Future distributed impl boundary.** The trait must never expose in-proc-only guarantees (e.g. "strongly consistent across all calls within this process"). A future `RedisGcraLimiter` cannot provide that; therefore the trait contract must not promise it.

### Standards and review gates

- Rate-limiter changes require the `riftgate-bench` limiter microbenchmark to remain within [`NFR-P07`](../01-requirements/non-functional.md) (<100 µs at 1k RPS).
- Changes to `SubjectKey` require an ADR (the key shape is a durable part of the trait surface).
- Adding a new `RateLimiter` impl requires a conformance-test pass against the standard suite in `crates/riftgate-core/tests/rate_limit_conformance.rs`.
- Any impl that adds a new failure mode (e.g. Redis unreachable) requires a documented fallback policy (fail-open vs fail-closed).

## Testing strategy

- **Microbenchmark** — single-subject, multi-subject, contention-by-shared-shard, and pathological-subject-cardinality runs.
- **Conformance suite** — every impl runs the same set of tests (exact-burst, sustained-rate, multi-dimensional cost, fairness-under-contention).
- **Soak** — 24h run with a realistic subject distribution; verify no leak in the internal map, no drift in bucket accounting.
- **Fault injection** (future distributed impls only) — Redis unreachable, partial network, high latency; verify documented fallback is honored.

## Open questions

- Should denied requests update a per-subject "recent denial count" to feed an adaptive circuit breaker? Recommend yes for a `v0.3+` iteration, not in the first ship.
- Should we support per-method cost functions at the route level (e.g. "a streaming request costs 10× a non-streaming one")? Recommend yes via config, but keep the trait signature unchanged.
- How does this compose with [Options `012` (backpressure)](../05-options/012-backpressure.md)? The rate limiter is a policy-driven deny; backpressure is a queue-saturation deny. They can stack: rate-limit first (cheaper), backpressure second (needs queue state). Document the stacking order in `lld-scheduling.md` once both are in.
