# ADR 0009. RateLimiter trait + in-proc token-bucket only in v0.2; distributed impls deferred

> **Date:** TBD (target acceptance: at the open of `v0.2`)
> **Status:** proposed
> **Options doc:** [021-rate-limiting](../05-options/021-rate-limiting.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate sits in front of LLM backends that enforce their own per-tenant TPM / RPM / concurrency limits. Without metering on the way in, the gateway pays the parsing, filter-chain, and routing cost on requests that the backend will reject; without per-tenant policy, a single misbehaving client can exhaust shared gateway resources before the backend pushes back. Full exploration of the design space — fixed-window, sliding-window, token bucket, leaky bucket, GCRA, and the four families of distributed extensions — lives in [Options `021`](../05-options/021-rate-limiting.md).

The forces summarized: [Vision §4](../00-vision.md) explicitly declines to ship a globally-coherent rate limiter in `v1.0`; per-instance rate limiting is the committed scope, and any cross-replica coherence is a future distributed implementation of the same trait, not a kernel feature. [`FR-108`](../01-requirements/functional.md) commits us to an in-proc token-bucket limiter in `v0.2`; [`NFR-P07`](../01-requirements/non-functional.md) bounds the enforcement overhead to <100 µs per request at 1k RPS.

## Decision

**Riftgate `v0.2` ships a `RateLimiter` trait in `riftgate-core` with one in-proc, lock-free token-bucket implementation; distributed implementations are deferred and accepted as future ADRs that supersede this one.**

The discipline:

- The `RateLimiter` trait is defined in `riftgate-core` per the sketch in [Options `021` §6](../05-options/021-rate-limiting.md). The `check(subject, cost) -> LimitDecision` signature must accept distributed implementations (Redis-Lua GCRA, Dragonfly, sharded local + gossip, consistent-hash + sticky routing) without a breaking change.
- The default impl, `TokenBucketLimiter`, lives in `riftgate-core` and uses a sharded `DashMap<SubjectKey, AtomicBucketState>` with a single `AtomicU64` packing `(tokens_scaled, last_refill_nanos)` on the lock-free fast path.
- Denied requests return `429 Too Many Requests` with a `Retry-After` header computed from bucket-depletion time and are first-class in OTel telemetry.
- A `NoopLimiter` is also in `riftgate-core` for benchmarks, dev mode, and any operator who explicitly disables rate limiting.
- A future `RedisGcraLimiter` (or any other distributed impl) ships in its own crate (e.g. `crates/riftgate-rate-limit-redis`) behind a feature flag; a new ADR supersedes this one when that lands.

## Consequences

- **Positive:**
  - Operators get a working per-instance rate limiter in `v0.2` with one well-shaped knob set (`rate_per_sec`, `burst`, `cost_fn`).
  - The trait is the abstraction boundary; a future distributed impl can replace `TokenBucketLimiter` at the config level without recompiling callers.
  - The cost function (`cost = 1` for RPM limiting; `cost = token_count(prompt)` for TPM limiting) is a first-class parameter, which matches what LLM operators actually configure.
  - Audit-grade visibility: every denial emits an OTel counter with the structured `DenialReason`.
- **Negative / accepted tradeoffs:**
  - No cross-replica coherence in `v1.0`. Operators running multi-replica deployments either configure per-replica limits (`limit / N_replicas`) or place a proper rate-limit gateway in front. This is documented in [Vision §4](../00-vision.md).
  - The in-proc impl's subject-key cardinality is bounded by `(tenant, route, backend)`; pathological key shapes that explode the internal map are caught at config-validation time, not at runtime.
  - We accept the small but non-zero correctness work of the lock-free CAS loop on packed atomic state; `loom` tests cover the critical paths.
- **Future work this enables:**
  - A `RedisGcraLimiter` impl (Lua-script-atomic GCRA over Redis or Dragonfly) without breaking the trait.
  - Composition with the future `CircuitBreaker` (Options `011`) and the future backpressure policy (Options `012`) through a shared event vocabulary.
  - Priority-aware rejection (Options `022`, gated on the `v0.2` retro) layered on top of `RateLimiter` rather than inside it.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship a Redis-backed rate limiter as the default in `v0.2` or `v1.0`.
  - Riftgate will not ship a leaky-bucket-as-queue limiter that adds admission latency to admitted requests; that shape is incompatible with our streaming TTFT guarantees ([`NFR-P05`](../01-requirements/non-functional.md)).
  - Riftgate will not ship an unbounded queue or fixed-window-only limiter — both are explicitly rejected in [Options `021` §7](../05-options/021-rate-limiting.md).

## Compliance

- `crates/riftgate-core::rate_limit::RateLimiter` is the single trait; `TokenBucketLimiter` and `NoopLimiter` are the `v0.2` impls.
- `crates/riftgate-core/tests/rate_limit_conformance.rs` runs every `RateLimiter` impl through the same conformance suite (exact-burst, sustained-rate, multi-dimensional cost, fairness-under-contention).
- A `loom` test on the lock-free CAS loop in `TokenBucketLimiter` lives in `crates/riftgate-core/tests/loom_rate_limit.rs`.
- A microbenchmark in `benchmarks/rate_limit/` enforces [`NFR-P07`](../01-requirements/non-functional.md) (<100 µs at 1k RPS).
- Adding a new `RateLimiter` impl that introduces a new failure mode (e.g. Redis unreachable) requires a new ADR superseding this one, with a documented fail-open vs fail-closed posture.

## Notes

- The trait shape — `check` returning `LimitDecision { Allow, Deny { retry_after } }` — is deliberately designed so a Redis-backed impl that surfaces `Retry-After` semantics fits without a second method or a breaking change. This is the place where future-compatibility discipline shows up.
- `TokenBucketLimiter` over GCRA is a posture choice, not a correctness one: the two are mathematically equivalent. Token bucket is the more operator-legible primitive ("a bucket of size `burst` that refills at `rate` per second"); GCRA reads better in proofs but requires more explanation. We pick legibility for `v0.2` and keep GCRA as the obvious shape for the future distributed impl.
- The decision to keep `v1.0` per-instance only is in the spirit of [Vision §4](../00-vision.md): we do not ship a globally-coherent rate limiter as a kernel feature; we ship the trait and one good in-proc impl, and we let the community (or future Riftgate releases) author the distributed impls.
