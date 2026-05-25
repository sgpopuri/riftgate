# ADR 0016. Three-state circuit breaker per backend; sliding-window and adaptive deferred

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [011-circuit-breaker](../05-options/011-circuit-breaker.md)
> **Deciders:** Sriram Popuri

## Context

v0.2 introduces multi-backend routing ([Options `010`](../05-options/010-routing-strategy.md)) and a real expectation of resilience under partial-backend failure. Without a per-backend protection primitive, the router will keep selecting an unhealthy backend until external retries route around it; this violates `NFR-R01` (≤5 s p95 time-to-route-around). Full exploration of candidates (classic 3-state, sliding-window failure-rate, adaptive concurrency, none) and the tradeoff matrix live in [Options `011`](../05-options/011-circuit-breaker.md).

The forces summarised: `FR-103` commits us to per-backend health-aware routing; `NFR-R01` bounds the reaction time; `NFR-OBS04` requires a structured cause for every rejection; the breaker must speak the same `DenialReason` vocabulary as the rate limiter ([Options `021`](../05-options/021-rate-limiting.md)) and the backpressure policy ([Options `012`](../05-options/012-backpressure.md)) so the gateway has one rejection vocabulary, not three.

## Decision

**`v0.2` ships a `CircuitBreaker` trait in `riftgate-core` with one impl, `ThreeStateBreaker`, a per-backend classic 3-state FSM (Closed / Open / Half-Open) with a bounded half-open probe budget; integration into the router is by decorator (`CircuitBreakerArbiter<R>`); sliding-window and adaptive variants are catalogued for future impls of the same trait.**

- Trait surface: `admit(backend) -> AdmissionDecision`, `report_outcome(backend, Outcome)`. `Outcome ∈ {Success, Failure, Timeout}`. `AdmissionDecision::Reject` carries `DenialReason::CircuitOpen` and a `retry_after`.
- State per backend: `(state, failure_count, last_transition, half_open_in_flight)`. Per-backend independence is non-negotiable.
- Failure classification: HTTP `5xx`, `408`, `504`, connect-timeout, read-timeout count as `Failure` or `Timeout`. `4xx` (except 408 and 429) does NOT count; client errors are not backend failures.
- Config (per [ADR `0012`](0012-static-toml-env-override-v01.md)):

  ```toml
  [circuit_breaker]
  failure_threshold      = 5
  failure_window_ms      = 10000
  reset_timeout_ms       = 30000
  half_open_max_probes   = 3
  ```

## Consequences

- **Positive:**
  - Operator-legible (3 states, 1 diagram); standard on-call vocabulary.
  - Bounded recovery work via half-open probe budget; no recovery storms.
  - Per-backend independence; failures of one backend do not penalise traffic to another.
  - Decorator integration (`CircuitBreakerArbiter<R>`) keeps the `Router` trait unchanged and composes with any current and future routing impl (RR, weighted-random, future hedged).
  - Shared `DenialReason` vocabulary with rate limiter and backpressure: one observability surface covers all three.
  - Reaction time bounded by `failure_threshold` × per-request time within `failure_window_ms`; meets `NFR-R01`.
- **Negative / accepted tradeoffs:**
  - Absolute-counter threshold can be wrong-shaped for very-high-volume backends; mitigated by `failure_window_ms`. Sliding-window variant available as a future impl when operator feedback warrants.
  - `half_open_max_probes` is its own knob; defaults named in the LLD.
  - No latency-based health signal in v0.2; the breaker reacts to failures, not slowdowns. Slowdown handling lives in [Options `012`](../05-options/012-backpressure.md) (queue depth) and a future adaptive variant.
- **Future work this enables:**
  - `SlidingWindowBreaker` and `AdaptiveBreaker` impls land behind feature flags without trait surgery.
  - Regional/zonal breaker modes layer on top in v0.4+ (one global state per region, transitions per region).
  - Per-tenant breaker scope (Options `017` multitenancy) extends the `BackendId` key shape additively.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship a global (cross-backend) breaker.
  - Riftgate will not count 4xx (except 408/429) as backend failure.
  - Riftgate will not auto-tune breaker thresholds in v0.2.

## Compliance

- `crates/riftgate-core::circuit::CircuitBreaker` is the single trait; `ThreeStateBreaker` is the v0.2 default.
- `crates/riftgate-router::CircuitBreakerArbiter<R>` is the router decorator and wraps `RoundRobinRouter` and `WeightedRandomRouter` in the v0.2 binary.
- `crates/riftgate-core/tests/circuit_conformance.rs` exercises every state transition (Closed → Open on threshold, Open → HalfOpen on timeout, HalfOpen → Closed on N successes, HalfOpen → Open on any failure).
- `crates/riftgate/tests/circuit_integration.rs` drives a synthetic failing backend and asserts that the circuit opens within `failure_window_ms`, that subsequent requests get `503 + Retry-After + DenialReason::CircuitOpen`, and that recovery occurs through the half-open probe budget.
- A criterion bench in `benchmarks/circuit_breaker/` measures the per-request cost of `admit + report_outcome`.
- Adding a new `CircuitBreaker` impl that changes the trait surface requires a new ADR superseding this one.

## Notes

- The choice to integrate as a *decorator* (wraps any `Router`) rather than as a *modification* to the `Router` trait is load-bearing: it keeps `Router` focused on selection and composes cleanly with the future `HedgedRouter` (per [Options `010`](../05-options/010-routing-strategy.md)).
- 4xx (except 408 and 429) is deliberately excluded from the failure signal because a buggy client should not cause the breaker to open and penalise other clients. The 408 and 429 exceptions are because both can be backend-driven under load.
- The breaker shares `DenialReason` with the rate limiter ([ADR `0009`](0009-rate-limiter-trait-in-proc-only.md)) and backpressure ([ADR `0017`](0017-drop-newest-503-backpressure.md)) by design. If a future contributor proposes a fourth protection primitive, it must speak the same vocabulary or supersede the three ADRs that established it.
