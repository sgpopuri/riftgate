# ADR 0023. Hedged requests via Dean–Barroso threshold-triggered shape, degree=2, rate-limit-budget-aware

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [025-v03-routing-strategies](../05-options/025-v03-routing-strategies.md)
> **Deciders:** Sriram Popuri

## Context

[Options `010`](../05-options/010-routing-strategy.md) catalogued hedged requests as a v0.3 deliverable; [Options `025`](../05-options/025-v03-routing-strategies.md) revisits the design space with the v0.3 stream-cancellation primitive ([ADR `0020`](0020-stream-cancellation-cancellation-token.md)) now available. Four shapes were evaluated: always-hedge, threshold-triggered (Dean & Barroso, 2013), always-hedge with degree N, and per-route operator configuration only. Always-hedge doubles steady-state load; degree-N hedging shows diminishing returns past N=2; per-route-only hedging misses the dynamic-tail case. Threshold-triggered hedging — the canonical *Tail at Scale* shape — gives most of the latency benefit at a small fraction of the cost and composes cleanly with the v0.3 cancellation primitive.

## Decision

**`v0.3` ships `HedgedRouter<R>` in `crates/riftgate-router` as a decorator over an inner `Router`, using a Dean–Barroso threshold-triggered hedge with degree=2, a per-backend P²-estimator for first-byte p95 latency, and a global rate-limit-budget cap (`hedge_max_fraction`). The loser is cancelled via `Cancellation::cancel(CancelCause::HedgedLoser { winner })`. Always-hedge, degree > 2, and per-route-only hedging are rejected for v0.3.**

- Degree is fixed at 2 for v0.3; raising the cap requires a new ADR.
- Trigger threshold per backend is its rolling first-byte p95 (estimated via the P² algorithm; one estimator per backend with a configurable warmup window).
- `hedge_max_fraction` default 0.05 (≤ 5% of traffic may hedge); enforced globally to bound steady-state load amplification.
- `hedge_min_threshold_ms` default 50ms (never hedge if the primary's p95 is already that low — saves capacity on fast deployments).
- The router emits `RoutingDecision::Hedge(vec![primary, secondary])`; the request driver in `crates/riftgate/src/proxy.rs` is responsible for the actual fan-out: it dispatches the primary, sets a timer at `primary.p95_first_byte_ms`, and dispatches the secondary only if the timer fires before the primary returns its first byte.
- Loser cancellation uses the v0.3 cancellation primitive ([ADR `0020`](0020-stream-cancellation-cancellation-token.md)); telemetry records `bytes_seen_before_cancel` for trigger-threshold tuning over time.
- Composes under `CircuitBreakerArbiter` and stacks above `KvAwareRouter` ([ADR `0022`](0022-kv-aware-routing-prefix-trie.md)): `CircuitBreakerArbiter::new(HedgedRouter::new(KvAwareRouter::new(WeightedRandomRouter::new(...))))`.
- The `Router` trait surface in `riftgate-core` is unchanged; `RoutingDecision::Hedge` was declared in v0.2 ([ADR `0014`](0014-weighted-random-router.md)) for exactly this v0.3 fulfilment.

## Consequences

- **Positive:**
  - Bounded steady-state load amplification (≤ 5% by default), making capacity planning predictable.
  - Real p99 latency improvement on workloads with naturally-skewed tails (slow backend, partial GPU failure, NUMA-traffic-warp).
  - Reuses the v0.3 cancellation primitive; the loser's tail-end is cleanly cancelled (not abandoned) with cause attribution.
  - Composes with rate limiting (a hedged request consumes two limiter tokens) and the circuit breaker (Open backends are filtered before hedge sees them).
  - Telemetry (`hedge.fired`, `hedge.budget_blocked`, `hedge.winner`, `hedge.bytes_wasted_total`) provides the data needed to tune the trigger threshold over time.
- **Negative / accepted tradeoffs:**
  - Per-backend P² estimator state adds a small per-backend memory cost (a few hundred bytes per backend); bounded and documented.
  - The hedge-budget enforcement is global (single counter); per-tenant budgets are deferred to a future ADR if operator demand surfaces.
  - TTFB-shaped trigger (first-byte latency) is the wrong signal for some workloads (e.g. ones where TTFB is fast but full-response is slow); documented as a v1.0+ enhancement opportunity.
  - Hedged-request observability adds non-trivial span attribute volume; mitigated by sampling at the OTel layer (operator-controlled).
- **Future work this enables:**
  - Per-tenant hedge budgets become a clean v0.4+ addition once multitenancy ([Options `017`](../05-options/README.md)) lands.
  - Adaptive trigger thresholds (EWMA over p95 with feedback from `bytes_wasted_total`) become a measurement-driven improvement on top of the v0.3 P² baseline.
  - Cancellation cause `HedgedLoser` is queryable in the WAL (per [ADR `0013`](0013-append-only-file-wal.md)); replay-eval ([ADR `0021`](0021-external-replay-cli.md)) reports per-cause cancellation distribution.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship always-hedge as a v0.3 default; never as a production default.
  - Riftgate will not ship degree > 2 hedging in v0.3.
  - Riftgate will not hedge without a budget cap; uncapped hedging is a known cost-control footgun.

## Compliance

- `HedgedRouter` lives in `crates/riftgate-router/src/hedged.rs` and implements the existing `Router` trait.
- `crates/riftgate-router/tests/hedge_budget.rs` asserts that `hedge_fraction_observed ≤ hedge_max_fraction + tolerance` over 10k requests with a configured slow backend.
- `crates/riftgate-router/tests/hedge_cancellation.rs` asserts the loser's task observes `CancelCause::HedgedLoser { winner }` and that the upstream connection observes `connection: close` within the cancellation-latency budget.
- `crates/riftgate-router/tests/hedge_latency_improvement.rs` asserts measurable p99 improvement on a workload with 10% slow-backend artificial delay.
- `crates/riftgate-router/tests/p2_estimator.rs` covers the per-backend latency-quantile estimator under varied input distributions (uniform, log-normal, bimodal).
- A criterion bench at `crates/riftgate-router/benches/hedge_route.rs` measures per-`route()` cost; CI fails if p99 exceeds 50µs (`NFR-P11`).
- Configuration changes via TOML (`hedge_after_quantile`, `hedge_max_fraction`, `hedge_min_threshold_ms`) do **not** require a new ADR; raising the degree does.

## Notes

- The Dean–Barroso threshold-triggered shape is the right default because it gives strong tail-latency improvement with bounded capacity overhead. Production validation comes from Bigtable, Cassandra (`speculative_retry`), and Envoy's `request_hedging`. Riftgate is following well-trodden territory.
- The P² algorithm (Jain & Chlamtac, 1985) is chosen over t-digest or HDR histogram for the per-backend latency estimator because (a) it has O(1) update cost, (b) state per backend is fixed and small (5 quantile markers), and (c) accuracy at p95 is more than sufficient for trigger-threshold purposes. If we add SLO-grade quantile reporting later, t-digest is the right tool — but for the trigger, P² is correct.
- The decision to keep the hedge-budget global (rather than per-tenant) in v0.3 is intentional: per-tenant requires multitenancy infrastructure that hasn't been built yet. The global cap is the right "do no harm" default.
- The interaction with rate limiting ([Options `021`](../05-options/021-rate-limiting.md)) is deliberate: a hedged request consumes two limiter tokens. Tenants who do not want hedging counted twice can disable hedging on their routes via the per-route config knob (future, but the trait shape supports it).
