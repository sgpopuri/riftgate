# ADR 0014. Weighted-random router (Walker alias method) added in v0.2; KV-aware and hedged deferred to v0.3

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [010-routing-strategy](../05-options/010-routing-strategy.md)
> **Deciders:** Sriram Popuri

## Context

v0.1 ships `RoundRobinRouter`. v0.2 introduces multi-backend deployments with heterogeneous capacity (`FR-102`) and a real expectation of health-aware routing (`FR-103`). Full exploration of candidates (round-robin, weighted-random, least-loaded, KV-aware, hedged) and the tradeoff matrix live in [Options `010`](../05-options/010-routing-strategy.md).

The forces summarised: `FR-102` requires first-class weights; `FR-103` requires per-backend health-aware skipping (provided by the [circuit breaker](../05-options/011-circuit-breaker.md) decorator); `NFR-P07` bounds the per-request cost; KV-aware prefix routing and hedged requests are real but require infrastructure (WASM extension surface, stream cancellation) that lands in v0.3.

## Decision

**`v0.2` adds `WeightedRandomRouter` to `crates/riftgate-router`, implemented with Walker's alias method (Vose 1991) for O(1) sampling; the existing `RoundRobinRouter` remains; the `Router` trait surface in `riftgate-core` is unchanged; KV-aware prefix routing and hedged requests are explicitly deferred to v0.3.**

- Operator picks via `routing_strategy = "round_robin" | "weighted_random"` in TOML config.
- `[[backend]] weight = N` is honored by `weighted_random` and ignored by `round_robin`; missing `weight` defaults to 1.
- The binary always wires `CircuitBreakerArbiter<R>` over the chosen router (per [ADR `0016`](0016-three-state-circuit-breaker.md)); the decorator filters Open backends out of the eligible set before the router samples.
- Alias-table construction is O(N) at config-load and on config-reload; the hot path is two RNG draws (`Xoshiro256++`) plus one indirect load. Capped at N = 32 eligible backends for v0.2.

## Consequences

- **Positive:**
  - First-class weights match operator intuition: `weight = 70` and `weight = 30` produces a 70:30 split without entry duplication.
  - O(1) hot-path selection regardless of weight distribution; fits comfortably inside `NFR-P07`.
  - Trait surface unchanged, so the binary's wiring code is invariant under router choice; the breaker decorator composes with both impls.
  - Alias table is rebuilt on config-reload events, not on the hot path; reload cost is O(N).
- **Negative / accepted tradeoffs:**
  - Variance at small N (1-2 backends) before the law of large numbers kicks in; documented in the LLD. If operator complaints arise, smoothed-WRR lands as an additional impl.
  - N capped at 32 in v0.2; for larger fleets the LLD will name the rebuild-vs-rejection-sample tradeoff. Revisit at v0.3 retro if a real deployment hits the cap.
  - No latency-aware routing in v0.2; slow backends are the breaker's domain.
- **Future work this enables:**
  - `KvAwareRouter` (prefix-trie or `lmcache` integration) lands in v0.3 behind the same trait, paired with the WASM extension surface.
  - `HedgedRouter` lands in v0.3 alongside stream-cancellation primitives.
  - Smoothed-WRR (Nginx shape) can land as an additional impl without a new ADR if N=2 variance becomes a complaint.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship least-loaded as a v0.2 router (catalogued; rejected for v0.2 because it conflates with the breaker domain).
  - Riftgate will not ship a routing-strategy plugin surface in v0.2; WASM-pluggable routing is a v0.3 deliverable.

## Compliance

- `crates/riftgate-router::WeightedRandomRouter` is the v0.2 impl.
- `crates/riftgate-router/tests/weighted_random_distribution.rs` asserts that observed selection counts match configured weights within a statistical tolerance over 10k samples.
- `crates/riftgate-router/tests/alias_method.rs` covers alias-table construction edge cases (single backend, zero-weight, all-equal weights, very-skewed weights).
- `crates/riftgate-router/tests/circuit_decorator_integration.rs` asserts that `CircuitBreakerArbiter` correctly hides Open backends from both router impls.
- A criterion bench in `crates/riftgate-router/benches/` measures per-selection cost for both impls at N ∈ {2, 8, 32}.
- Adding a new `Router` impl requires no new ADR (the trait is extensible by design); adding a new routing-strategy *config value* requires a one-line config-schema update and a test.

## Notes

- Walker's alias method over cumulative-weight-binary-search is a deliberate choice: at small N the difference is negligible, but the alias-method shape generalises to any N without changing the hot path's algorithmic complexity.
- The decision to keep `Router` trait unchanged was load-bearing during v0.1 design (it was sized to accommodate weighted, KV-aware, and hedged). The v0.2 implementation confirms that the v0.1 surface was correct; no trait surgery needed.
- KV-aware routing is the v0.3 headline. The Options doc names it explicitly so the v0.3 plan inherits a clear extension point.
