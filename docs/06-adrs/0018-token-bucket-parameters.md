# ADR 0018. TokenBucketLimiter parameter set: packed AtomicU64 with SCALE = 65536, 64 DashMap shards, config-validated subject cardinality

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [023-token-bucket-parameters](../05-options/023-token-bucket-parameters.md)
> **Deciders:** Sriram Popuri

## Context

[Options `021`](../05-options/021-rate-limiting.md) and [ADR `0009`](0009-rate-limiter-trait-in-proc-only.md) commit Riftgate `v0.2` to an in-proc token-bucket impl of `RateLimiter`. Neither document names the implementation knobs: state packing, refill arithmetic, shard count, default rate/burst values, cost-function shape. [Options `023`](../05-options/023-token-bucket-parameters.md) explores four candidate state representations against `NFR-P07` (<100 µs at 1k RPS) and `NFR-A03` (bounded allocator footprint) and recommends a specific packing.

## Decision

**`TokenBucketLimiter` uses a `DashMap<SubjectKey, AtomicU64>` with 64 shards; each `AtomicU64` packs `(tokens_scaled: u32, last_refill_nanos: u32)` with `SCALE = 1 << 16` and a per-shard epoch anchor for nanos wraparound; the hot path is a Vyukov-style CAS loop; max subject cardinality is computed at config-validation time and enforced at load.**

Operator-facing knobs:

```toml
[[rate_limit]]
tenant  = "team-platform"        # optional; wildcard if omitted
route   = "openai-chat"          # optional; wildcard if omitted
backend = "openai-primary"       # optional; wildcard if omitted

rate_per_sec = 100               # required: sustained allowance
burst        = 200               # optional; default = 2 * rate_per_sec
cost         = "request"         # or "prompt_tokens"; default "request"
retry_after_floor_ms = 50        # optional; default 50
```

Internal defaults (compile-time, not operator-tunable in v0.2): `SCALE = 1 << 16`, 64 DashMap shards, per-shard epoch for 32-bit-nanos wraparound, max subject cardinality computed as `Σ tenants × routes × backends` across `[[rate_limit]]` entries plus wildcards.

## Consequences

- **Positive:**
  - Lock-free fast path; CAS loop converges in 1-2 iterations under realistic contention. Hot-key behaviour matches the LLM-workload pattern (a few power-user tenants).
  - 8 bytes of state per subject; bounded allocator footprint at config-validation time satisfies `NFR-A03`.
  - The packed `AtomicU64` representation maps directly to the future Redis-Lua / Dragonfly distributed impl — the Lua script is exactly the same CAS loop.
  - `cost = "prompt_tokens"` covers the TPM use case as a first-class config value, not an out-of-band extension.
  - Telemetry surface (`ratelimit.checked / denied / bucket_depth / cas_retries`) gives operators four signals that diagnose every realistic failure mode.
- **Negative / accepted tradeoffs:**
  - `SCALE = 1 << 16` caps burst at ~65k token-equivalents per subject; deployments with bursts beyond that need a future widened-packing ADR.
  - Per-shard epoch trick for 32-bit-nanos wraparound is one more concept in the implementation; documented inline and in [`docs/04-design/lld-rate-limiter.md`](../04-design/lld-rate-limiter.md).
  - Subject cardinality is fixed at config-load; new subjects beyond declared scopes are rejected at config time rather than at runtime. Acceptable tradeoff for `NFR-A03`; documented in the LLD.
  - No subject-TTL eviction in v0.2; entries persist for process lifetime. Revisit in v0.3 once cardinality patterns are observed.
- **Future work this enables:**
  - `RedisGcraLimiter` impl (per [ADR `0009`](0009-rate-limiter-trait-in-proc-only.md)) reuses the packed-state pattern unchanged.
  - User-defined cost functions land in v0.3 behind WASM filters ([Options `016`](../05-options/README.md)).
  - Adaptive shard count (auto-tuning) is additive — does not require a new ADR.
- **Future work this forecloses (until superseded):**
  - Riftgate will not expose `SCALE`, shard count, or epoch parameters as operator knobs in v0.2.
  - Riftgate will not ship a Mutex-based or two-atomics representation for the in-proc impl.

## Compliance

- `crates/riftgate-core::rate_limit::TokenBucketLimiter` implements the parameter set above.
- `crates/riftgate-core/tests/rate_limit_conformance.rs` covers exact-burst, sustained-rate, multi-dimensional cost, fairness-under-contention.
- `crates/riftgate-core/tests/loom_rate_limit.rs` exercises the CAS loop under `loom`.
- `benchmarks/rate_limit/` enforces `NFR-P07` (<100 µs per check at 1k RPS).
- `crates/riftgate-config` validator computes max subject cardinality from `[[rate_limit]]` entries and refuses load if the configured bound is exceeded.
- Changing SCALE, shard count, or the packing layout requires a new ADR superseding this one and re-running the bench gate.

## Notes

- The choice of `SCALE = 1 << 16` over `SCALE = 1_000_000` (microtokens) was driven by the TPM headroom requirement — see [Options `023` §3](../05-options/023-token-bucket-parameters.md). Microtokens would have been cleaner arithmetically but cap burst at ~4k token-equivalents, which is too tight for the prompt-token-cost case.
- The 64-shard default is conservative; v0.2 retro will look at `cas_retries` histograms from real deployments and decide whether to keep, raise, or lower it.
- `cost = "prompt_tokens"` is computed by the request-side filter chain *before* the rate-limit filter runs, so the limiter itself sees a `u32` cost and remains pure. The filter ordering is documented in [`docs/04-design/lld-rate-limiter.md`](../04-design/lld-rate-limiter.md).
