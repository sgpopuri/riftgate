# 011. Circuit breaker

> **Status:** `recommended` — classic 3-state (closed / open / half-open) per backend, with a configurable failure threshold and a bounded half-open probe budget. See [ADR `0016`](../06-adrs/0016-three-state-circuit-breaker.md).
> **Foundational topics:** resilience patterns (Nygard, *Release It*); sliding-window failure-rate (Hystrix); adaptive concurrency limits (Netflix `concurrency-limits`); FSM-based protection primitives
> **Related options:** [`010 — routing strategy`](010-routing-strategy.md) (the breaker arbitrates which backend the router is allowed to select), [`012 — backpressure`](012-backpressure.md) and [`021 — rate limiting`](021-rate-limiting.md) (sibling protection primitives sharing one rejection vocabulary)
> **Related ADR:** [ADR `0016`](../06-adrs/0016-three-state-circuit-breaker.md)

## 1. The decision in one sentence

> When an upstream backend fails (timeouts, 5xx, connection errors), how does the gateway detect it, stop sending traffic, decide when to probe for recovery, and resume — all while staying composable with the rate limiter and backpressure policy?

## 2. Context — what forces this decision

In v0.1, the gateway forwards every request to the configured upstream and surfaces the upstream's failure verbatim. This is correct for a walking skeleton but wrong as soon as we have multiple backends and a real expectation of resilience: an unhealthy backend should be *skipped* by the router, and the decision to skip should be driven by observed signal, not by a static config flip.

The forces:

- **[`FR-103`](../01-requirements/functional.md)** commits us to per-backend health-aware routing in v0.2; the breaker is the data structure that implements "health-aware."
- **[`NFR-R01`](../01-requirements/non-functional.md)** bounds the time between a backend going down and the gateway routing around it (≤5 s p95).
- **[`NFR-OBS04`](../01-requirements/non-functional.md)** requires that every routing decision is attributable to a structured cause; "circuit was open" is one of the causes.
- **Composition.** [Options `021`](021-rate-limiting.md) (rate limit) and [Options `012`](012-backpressure.md) (queue saturation) already produce `Reject { retry_after, reason }`. The breaker must speak the same vocabulary.

The breaker is the protection primitive that operates on the *upstream* failure domain; the rate limiter and backpressure policy operate on the *ingress* failure domain. They are siblings, not alternatives.

## 3. Candidates

### 3.1. Classic 3-state (Nygard, *Release It*)

**What it is.** Per backend, a finite-state machine with three states:

- **Closed.** Healthy. Requests pass through. Each failure increments a counter; if the counter crosses `failure_threshold` within `failure_window`, transition to Open.
- **Open.** Unhealthy. Requests are rejected immediately with `Reject { reason: CircuitOpen }`. After `reset_timeout` elapses, transition to Half-Open.
- **Half-Open.** Probing. A bounded number of requests (`half_open_max_probes`) are allowed through. If they all succeed, transition to Closed. If any fails, transition back to Open and reset the timer.

```text
                         ┌────────────────┐
   failure_threshold     │                │
   exceeded ──────────►  │     Open       │
                         │                │
                         └───┬────────────┘
                             │ reset_timeout elapsed
                             ▼
                         ┌────────────────┐
   N probes succeed ◄────┤   Half-Open    │────► probe fails
   transition Closed     │                │      transition Open
                         └────────────────┘
```

**Why it's interesting.**
- **Operator-legible.** Three states, named transitions, one diagram. Every on-call playbook can describe it.
- **Bounded recovery work.** The half-open probe budget caps how much traffic a still-broken backend sees during recovery.
- **Composable.** Per backend; multiple backends fail independently. Pairs naturally with weighted-random routing (Options [`010`](010-routing-strategy.md)) which simply skips Open backends.
- **Standard.** Every retry / breaker library in the industry implements this shape; operator intuition transfers.

**Where it falls short.**
- **Threshold is a hard counter, not a rate.** A backend that fails at low absolute counts but high relative rate is harder to detect than under sliding-window. Mitigated by `failure_window`.
- **Half-open probe budget is its own knob.** Too low — slow recovery; too high — partial recovery storms. Defaults named in §6.

**Real-world systems that use it.** Hystrix (in its simple mode); Resilience4j; Polly; Envoy's `outlier_detection` (a variant). The canonical industry shape.

### 3.2. Sliding-window failure-rate (Hystrix advanced mode)

**What it is.** A sliding window over recent requests records (`success | failure`). The Open transition fires when `failures / window_size > failure_rate_threshold` *and* `window_size >= rolling_min`.

**Why it's interesting.**
- More statistically honest than an absolute counter: 10 failures in 100 requests is different from 10 failures in 10 requests, and the rate captures it.
- Hystrix popularised it; lots of prior art.

**Where it falls short.**
- **More state per backend.** A ring buffer of recent outcomes; per-backend memory grows with window size.
- **More knobs.** `failure_rate_threshold`, `window_size`, `rolling_min`. Defaults are not obvious; the Hystrix defaults were known to be tricky.
- **Same recovery shape** as 3.1, which means the half-open state and its probe budget come along anyway; the only saving is on the *detection* side, which §3.1 already handles adequately.

**Real-world systems that use it.** Hystrix advanced mode; Resilience4j with `SlidingWindowType.COUNT_BASED`. A reasonable choice, but the extra knobs are a v0.3+ shape for Riftgate.

### 3.3. Adaptive concurrency limit (Netflix `concurrency-limits`)

**What it is.** Per backend, track in-flight concurrency and observed latency. An AIMD loop chooses the in-flight limit; requests beyond the limit are rejected. The "breaker" never explicitly opens; it shrinks the limit to near-zero under failure.

**Why it's interesting.**
- Auto-tunes capacity; no operator-chosen thresholds.
- Latency-aware — directly optimises observable health.

**Where it falls short.**
- **Conflates the "is the backend up?" decision with the "how much can we send it?" decision.** The two are different concerns; one is binary, the other is continuous. Conflating them makes the dashboard story muddier.
- **Slow to react** to a clean step change (backend returns 5xx for every request). The AIMD loop has its own timescale; a hard breaker reacts in `failure_window`.
- **Same conceptual overlap as [Options `012` §3.5](012-backpressure.md)** — this is the upstream-side version of the ingress-side AIMD candidate we already rejected for v0.2.

**Real-world systems that use it.** Netflix's `concurrency-limits` library; Envoy's `adaptive_concurrency` filter (ingress-side). Catalogued as a future extension; not the v0.2 default.

### 3.4. No breaker; rely on routing + retries

**What it is.** The router picks a backend; if it fails, the client (or a retry middleware) retries against a different backend. No explicit per-backend state.

**Why it's interesting.** Simplest possible thing.

**Where it falls short.**
- Every retry pays the parsing / filter / routing cost again before learning the backend is down.
- No mechanism to *stop* sending traffic to a known-bad backend; the per-backend bad-traffic cost is unbounded.
- Violates `NFR-R01` time-to-route-around.

**Real-world systems that use it.** Naive load balancers. Not a v0.2 candidate for Riftgate.

## 4. Tradeoff matrix

| Property | 3.1 Three-state | 3.2 Sliding-window | 3.3 Adaptive | 3.4 None | Why it matters |
|---|---|---|---|---|---|
| Operator legibility | high (3 states, 1 diagram) | medium | low | n/a | On-call playbook quality |
| State per backend | counter + timer (16 B) | window buffer (~256 B) | RTT samples (~1 KB) | 0 | Per-backend memory |
| Number of knobs | 3 (`threshold`, `window`, `reset_timeout`) | 5 | 3+ (target, alpha, beta) | 0 | Misconfiguration surface |
| Reaction time to step failure | within `failure_window` | within `window_size` | slow (AIMD timescale) | none | `NFR-R01` |
| Bounded recovery work | yes (probe budget) | yes | n/a | n/a | Avoid recovery storms |
| Distinguishes up/down vs over-capacity | yes (explicit) | yes (explicit) | no (conflated) | n/a | Dashboard clarity |
| Composes with shared `DenialReason` vocabulary | yes | yes | yes | n/a | Three protection primitives, one event vocabulary |
| Existing Rust crates | `failsafe`, `tower::retry` | `failsafe` | none mature | n/a | Implementation cost |

## 5. Foundational principles

**Resilience patterns (Nygard, *Release It*).** The 3-state breaker is canonical in Nygard's book; the half-open probe is the load-bearing innovation that turns "stop sending" into "stop sending *and* discover when to resume." Every modern resilience library descends from this shape.

**FSM-based protection primitives.** The breaker is a table-driven FSM with three states and four transitions. Riftgate already commits to FSM-based parsers ([ADR `0007`](../06-adrs/0007-handrolled-fsm-parser.md)); reusing the FSM-as-protection-primitive pattern keeps the kernel's mental model consistent.

**Per-backend independence.** The decision to keep one breaker per backend (rather than one global breaker) is the same shared-nothing principle that drives the per-shard scheduler ([ADR `0004`](../06-adrs/0004-per-shard-default-stealing-opt-in.md)) and the per-tenant rate-limit subject key ([Options `021` §6](021-rate-limiting.md)). Failures of one backend do not penalise traffic to another.

**Composition with rate limiter and backpressure.** The three protection primitives share a `DenialReason` vocabulary so OTel telemetry, client-facing `Retry-After`, and operator dashboards have one structured cause label across all three. This is the same load-bearing decision documented in [Options `012` §5](012-backpressure.md).

## 6. Recommendation

**For `v0.2`: ship the classic 3-state breaker per backend, behind a `CircuitBreaker` trait in `riftgate-core`. One default impl (`ThreeStateBreaker`). Adaptive and sliding-window catalogued as future impls of the same trait.**

Concretely:

1. The trait lives in `riftgate-core`:

   ```rust
   pub trait CircuitBreaker: Send + Sync {
       fn admit(&self, backend: BackendId) -> AdmissionDecision;
       fn report_outcome(&self, backend: BackendId, outcome: Outcome);
   }

   pub enum Outcome { Success, Failure, Timeout }
   ```
2. `ThreeStateBreaker` impl stores per-backend `(state: CircuitState, failure_count: u32, last_transition: Instant, half_open_in_flight: u32)`.
3. The breaker integrates as a *router decorator*: any `Router` impl wrapped in `CircuitBreakerArbiter<R>` will skip backends in `Open` state and admit at most `half_open_max_probes` requests when `HalfOpen`.
4. `Outcome::Failure` is decided by the binary's upstream client based on HTTP status and timeouts (5xx, 408, 504, connect-timeout, read-timeout all count). 4xx (except 408/429) does NOT count — that is a client error, not a backend failure.
5. Telemetry: `riftgate.circuit.state` (gauge per backend), `riftgate.circuit.transitions` (counter labelled `from -> to`), `riftgate.circuit.rejected` (counter sharing `DenialReason::CircuitOpen` with the rate limiter and backpressure policy).
6. Config (per [Options `015`](015-config-model.md)):

   ```toml
   [circuit_breaker]
   failure_threshold      = 5            # consecutive failures (within window) to open
   failure_window_ms      = 10000        # 10s
   reset_timeout_ms       = 30000        # 30s closed -> half-open delay
   half_open_max_probes   = 3            # max in-flight probes when half-open
   ```

### Conditions under which we'd revisit

- If operator feedback shows that absolute-counter thresholds are wrong-shaped for their traffic (high-volume backends with low absolute-count failures), a `SlidingWindowBreaker` impl lands behind a feature flag.
- If we ship adaptive backpressure (Options `012` §3.5 revisited at v0.3 retro), an `AdaptiveBreaker` may be the upstream-side equivalent that lands alongside.
- If multi-region routing arrives in v0.4+, the breaker may need a "regional" mode where a regional Open suppresses all backends in that region.

## 7. What we explicitly reject

- **Sliding-window as the v0.2 default (3.2).** The extra knobs are not justified at our current operator-feedback level. Catalogued.
- **Adaptive concurrency limit as the v0.2 default (3.3).** Same reasoning as [Options `012` §6](012-backpressure.md) — right future shape, wrong first shape.
- **No breaker (3.4).** Violates `NFR-R01`. Will not ship.
- **Global breaker (one state shared across backends).** Couples failures; violates per-backend independence.
- **Counting 4xx (except 408/429) as failure.** Client errors are not backend failures; conflating them produces false Opens under buggy clients.

## 8. References

1. Michael Nygard, *Release It! Design and Deploy Production-Ready Software* (2nd ed., 2018) — circuit breaker pattern, §5.
2. Netflix, [Hystrix wiki](https://github.com/Netflix/Hystrix/wiki) — sliding-window failure-rate breaker.
3. Resilience4j, [`CircuitBreaker` documentation](https://resilience4j.readme.io/docs/circuitbreaker).
4. Polly (.NET), [`CircuitBreaker` policy](https://github.com/App-vNext/Polly).
5. Envoy proxy, [`outlier_detection` documentation](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/outlier).
6. Netflix, [`concurrency-limits`](https://github.com/Netflix/concurrency-limits).
7. Rust crate [`failsafe`](https://docs.rs/failsafe/) — a reference 3-state breaker.
8. RFC 7231 §6.6.4 — `503 Service Unavailable` semantics.
