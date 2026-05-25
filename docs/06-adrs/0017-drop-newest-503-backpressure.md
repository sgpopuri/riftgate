# ADR 0017. Drop-newest 503 backpressure with high/low water marks; adaptive concurrency deferred

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [012-backpressure](../05-options/012-backpressure.md)
> **Deciders:** Sriram Popuri

## Context

The v0.2 binary makes the request queue an explicit object (`ShardedMpmcQueue`); the saturation policy becomes a deliberate choice rather than emergent tokio behavior. Full exploration of candidates (drop-newest 503, drop-oldest, block-accept, block-push, adaptive concurrency, unbounded) and the tradeoff matrix live in [Options 012](../05-options/012-backpressure.md).

The forces summarized: [`FR-104`](../01-requirements/functional.md) commits us to honest backpressure on the wire; [`NFR-A03`](../01-requirements/non-functional.md) requires a bounded worst-case allocator footprint, which makes an unbounded queue a hard reject; [`NFR-P05`](../01-requirements/non-functional.md) bounds streaming TTFT, which rules out admission-delay shapes; the three protection primitives ([rate limiter](../05-options/021-rate-limiting.md), [circuit breaker](../05-options/011-circuit-breaker.md), this) must share one rejection vocabulary so an operator does not learn three.

## Decision

**v0.2 ships drop-newest `503 Service Unavailable + Retry-After` behind a `BackpressurePolicy` trait in `riftgate-core`; the default impl `HighWaterPolicy` uses a bounded queue with high/low water-mark hysteresis; adaptive concurrency is catalogued as a future impl of the same trait.**

- `BackpressurePolicy::on_enqueue(depth) -> AdmissionDecision::{Accept, Reject { retry_after, reason }}` is the surface.
- `DenialReason` is shared with [`RateLimiter`](../05-options/021-rate-limiting.md) and [`CircuitBreaker`](../05-options/011-circuit-breaker.md) so the OTel `riftgate.queue.rejected` counter (and the client-visible `Retry-After`) carries one structured cause label across all three primitives.
- Config (TOML, per [ADR 0012](0012-static-toml-env-override-v01.md)): `[backpressure] queue_capacity, high_water, low_water, drain_rate_hint_per_sec, max_retry_after_ms`.
- The `gate_open` flag is a single `AtomicBool` updated under the high/low-water hysteresis rule defined in [Options 012 §6](../05-options/012-backpressure.md).

## Consequences

- **Positive:**
  - Constant-time admission decision (one atomic load, one compare); fits inside the per-request budget set by [`NFR-P07`](../01-requirements/non-functional.md).
  - Bounded worst-case in-flight memory; allocator footprint is knowable from `queue_capacity`.
  - Honest to clients: `503 + Retry-After` is the standard HTTP backpressure shape; well-behaved clients already speak it.
  - Composes with the rate limiter and circuit breaker via the shared `DenialReason` vocabulary. One observability surface covers all three.
  - Hysteresis (high/low water) prevents saw-tooth oscillation under sustained near-capacity load.
- **Negative / accepted tradeoffs:**
  - Operator must pick `queue_capacity` by hand; misconfiguration shows up as constant-503s (too low) or unbounded latency (too high). Mitigated by per-deployment guidance in [`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md).
  - Does not auto-react to upstream slowdown; that signal flows through the circuit breaker instead (which is the cleaner decomposition).
  - The freshest request is the one rejected; clients that retry tightly will keep losing. `Retry-After` is the mitigation; misbehaving clients are exactly what we are protecting the gateway from.
- **Future work this enables:**
  - `AdaptiveConcurrencyPolicy` impl (Netflix `concurrency-limits` shape) lands behind a feature flag without trait surgery.
  - Per-class admission (Options `022`, gated on v0.2 retro) is additive — `on_enqueue` extends to take a class identifier.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship drop-oldest, block-accept, or block-push as backpressure shapes.
  - Riftgate will not ship an unbounded queue.
  - Riftgate will not auto-tune `queue_capacity` in v0.2.

## Compliance

- `crates/riftgate-core::backpressure::BackpressurePolicy` is the single trait; `HighWaterPolicy` is the v0.2 default impl.
- `crates/riftgate-core/tests/backpressure_conformance.rs` exercises every impl through accept/reject/hysteresis/retry-after-calculation conformance cases.
- `crates/riftgate/tests/backpressure_integration.rs` floods the binary and asserts that 503s appear above high-water and that 200s resume below low-water.
- A criterion bench in `benchmarks/backpressure/` measures the per-request cost of `on_enqueue`.
- Adding a new `BackpressurePolicy` impl that changes the trait surface requires a new ADR superseding this one.

## Notes

- The decision to share `DenialReason` across `RateLimiter`, `CircuitBreaker`, and `BackpressurePolicy` is the load-bearing one: it is what makes the three primitives composable rather than three parallel vocabularies. If a future contributor proposes a fourth protection primitive, it must speak the same vocabulary or supersede this ADR.
- Hysteresis (high-water > low-water) is non-negotiable. A single threshold produces saw-tooth oscillation under sustained near-capacity load; this is well-documented in the LMAX Disruptor literature and we will not relitigate it.
- The default `queue_capacity = 4096` is sized to a 2-core dev machine; production deployments size up. The LLD names the rule of thumb.
