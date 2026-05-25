# 012. Backpressure policy

> **Status:** `recommended` — bounded request queue with a high-water-mark `503 Service Unavailable + Retry-After`; low-water resume; adaptive concurrency catalogued as a future extension behind the same trait.
> **Foundational topics:** backpressure as policy (Hellerstein, Carlini), drop-on-full ring buffers (LMAX Disruptor lineage), Little's law for queue-vs-latency reasoning, AIMD admission control (Netflix `concurrency-limits`, TCP-Vegas lineage)
> **Related options:** [`004 — request queue`](004-request-queue.md) (the substrate this policy sits on), [`011 — circuit breaker`](011-circuit-breaker.md) (sibling protection primitive), [`021 — rate limiting`](021-rate-limiting.md) (predictable backpressure source)
> **Related ADR:** [ADR `0017`](../06-adrs/0017-drop-newest-503-backpressure.md)

## 1. The decision in one sentence

> When the gateway's request queue saturates, what does Riftgate do to the next request — and how do we keep that decision honest, observable, and composable with the rate limiter and circuit breaker?

## 2. Context — what forces this decision

The v0.1 binary uses the tokio multi-thread runtime; queue depth is implicit in tokio's task queue and `mpsc::channel` capacity. v0.2 lands [`PerCoreScheduler` + `ShardedMpmcQueue`](003-concurrency-model.md), which makes the queue an explicit object with a configurable capacity. The moment the queue is explicit, the policy on saturation becomes a deliberate decision rather than an emergent behavior.

The forces:

- **[`FR-104`](../01-requirements/functional.md)** commits us to honest backpressure: when the gateway cannot accept more work, it must say so on the wire (not silently buffer or hang).
- **[`NFR-P05`](../01-requirements/non-functional.md)** bounds TTFT for streaming requests; any policy that adds admission latency to admitted requests is the wrong shape.
- **[`NFR-A03`](../01-requirements/non-functional.md)** bounds the worst-case allocator footprint: an unbounded queue is a direct violation.
- **The protection primitives must compose.** [Options `011`](011-circuit-breaker.md) and [Options `021`](021-rate-limiting.md) both produce `Reject` decisions on the hot path. They must share a vocabulary with the queue-saturation policy or the gateway has three different ways to say "no" that an operator has to debug independently.

Backpressure is a policy, not a mechanism. The mechanism is "the queue is full"; the policy is "what we do about it." This Options doc enumerates the policies.

## 3. Candidates

### 3.1. Drop-newest with `503 + Retry-After`

**What it is.** The queue has a fixed capacity and a high-water mark below it. When `push` is attempted at-or-above high-water, the request is rejected immediately with `503 Service Unavailable` and a `Retry-After` header computed from drain-rate × queue-depth. When the queue drops below a low-water mark, the gateway resumes accepting at full rate.

**Why it's interesting.**
- **Constant-time decision.** One atomic load (queue depth), one compare. Fits inside any other hot-path budget.
- **Honest to the client.** A 503 with `Retry-After` is the standard HTTP backpressure shape; every well-behaved client library already speaks it.
- **Composable with the rate limiter and circuit breaker.** All three speak `Reject + Retry-After`. An operator dashboard can attribute rejections to one of three sources (`rate-limit`, `circuit-open`, `queue-full`) by label without learning three vocabularies.
- **Bounded by construction.** The queue's capacity is the gateway's worst-case memory bound for in-flight requests; the allocator's worst-case footprint ([`NFR-A03`](../01-requirements/non-functional.md)) is therefore knowable up-front.
- **Hysteresis via high/low water marks** prevents the saw-tooth oscillation that a single threshold would produce under sustained near-capacity load.

**Where it falls short.**
- **Drops the freshest request.** A client retrying with backoff may keep losing — but this is exactly what `Retry-After` is for; well-behaved clients back off; misbehaving clients are the problem we are protecting the gateway from.
- **Does not auto-tune capacity.** The operator must pick a queue depth; misconfiguration shows up as either constant 503s (too low) or unbounded latency (too high). Mitigated by NFR-P07-style guidance in the LLD.

**Real-world systems that use it.** Nginx's `limit_conn_zone`, envoy's `overflow_action`, AWS API Gateway. The default shape of HTTP backpressure in the industry.

### 3.2. Drop-oldest (head-drop)

**What it is.** Queue is full; pop the oldest in-flight item from the head and drop it; push the new arrival at the tail.

**Why it's interesting.**
- Favors recent requests, which under bursty load is often what the operator wants ("yesterday's request is stale").
- Common in observability pipelines where older events are less valuable.

**Where it falls short.**
- **Wastes work already done.** A request popped from the head may have already been parsed, filtered, and routed; the cost of getting it that far is now sunk.
- **Surprising to clients.** The dropped client receives no response on a connection it already opened; from its perspective, the gateway hung and then closed. Diagnosing "why did my request disappear?" is much harder than "I got a 503."
- **Composes badly with the rate limiter.** A rate-limit-accept followed by a head-drop is a counted-but-not-served request, which makes the accounting confusing.

**Real-world systems that use it.** Logging pipelines (e.g. `fluentd`'s `buffer_overflow_action drop_oldest_chunk`); some message buses. Less common in request paths.

### 3.3. Block the accept loop

**What it is.** When the queue is full, the accept loop stops calling `accept(2)`; the kernel SYN queue absorbs the next few connections; once that fills, the kernel sends RSTs.

**Why it's interesting.**
- Zero work done in the gateway on rejected connections.
- Inherits whatever SYN-queue tuning the operator has done on the host.

**Where it falls short.**
- **Opaque on the wire.** Clients see TCP RST or timeout, not a 503. Cannot include `Retry-After`. Diagnosis requires `tcpdump`.
- **Couples gateway behavior to host kernel tuning** in non-obvious ways.
- **Cannot distinguish causes.** A client that saw a TCP RST has no idea whether it was rate-limited, the circuit was open, or the queue was full. The three protection primitives all converge to the same indistinguishable failure.
- **Bad for HTTP/2 and long-lived connections** where a single connection multiplexes many requests; blocking accept does not slow down a misbehaving multiplexed client.

**Real-world systems that use it.** Implicit in almost every accept-loop server when SYN backlog overflows. Not chosen *as* a policy by any modern gateway.

### 3.4. Block the producer (queue `push` waits)

**What it is.** `push` blocks (or `.await`s) until the queue drains below capacity.

**Why it's interesting.**
- No request is lost.
- Naturally throttles upstream load to gateway capacity.

**Where it falls short.**
- **Adds unbounded admission latency** — the producer (the accept loop in our case) waits arbitrarily long. The streaming TTFT budget ([`NFR-P05`](../01-requirements/non-functional.md)) is incompatible with this shape.
- **Couples accept-loop liveness to worker drain** — a slow shard stalls the accept loop, which stalls every other shard's intake.
- **Same observability problem as 3.3** — the client sees latency, not a structured rejection.

**Real-world systems that use it.** Some in-process job queues. Not a serious candidate for a network gateway.

### 3.5. Adaptive concurrency limit (Netflix `concurrency-limits` / TCP-Vegas AIMD)

**What it is.** The gateway tracks observed RTT (request latency) and uses an AIMD loop to choose the in-flight concurrency limit dynamically. When p95 latency rises, the limit shrinks; when latency is stable below target, the limit grows. Requests beyond the current limit are rejected with `503 + Retry-After`.

**Why it's interesting.**
- **Auto-tunes capacity** so the operator does not have to pick a queue depth by hand.
- **Latency-aware** — directly optimises the metric the operator cares about.
- **Battle-tested** at Netflix (the library is the canonical reference).

**Where it falls short.**
- **Two-knob system instead of one.** AIMD has its own parameters (`alpha`, `beta`, target latency); a misconfigured AIMD is worse than a hand-picked static queue depth.
- **Slow to react** to a step change in upstream latency.
- **Conflates upstream slowness with gateway saturation** — under a backend outage, AIMD will shrink the gateway's in-flight limit even though the gateway itself is healthy and the right answer is to open the circuit (Options [`011`](011-circuit-breaker.md)).
- **More to teach.** A v0.2-walking-skeleton-plus operator needs to learn one new policy primitive; we'd rather it be the simpler one.

**Real-world systems that use it.** Netflix's `concurrency-limits` Java library; Envoy's `adaptive_concurrency` filter. A real and serious choice — but a v0.3+ shape for Riftgate, not the default.

### 3.6. Unbounded queue (no policy)

**What it is.** Queue has no capacity limit; memory grows until OOM.

**Why it's interesting.** It is the default of every naïve implementation. Worth naming so we can reject it explicitly.

**Where it falls short.**
- **Violates [`NFR-A03`](../01-requirements/non-functional.md).** Worst-case allocator footprint is unbounded.
- **Trades 503s for OOM-kills**, which is the wrong direction.
- **Hides the saturation event from observability** until the kernel takes the process down.

**Real-world systems that use it.** Toy gateways. Not a serious candidate.

## 4. Tradeoff matrix

| Property | Drop-newest 503 | Drop-oldest | Block accept | Block push | Adaptive | Unbounded | Why it matters |
|---|---|---|---|---|---|---|---|
| Hot-path cost | O(1) atomic | O(1) + sunk work | none (kernel) | unbounded wait | O(1) + RTT bookkeeping | O(1) | NFR-P07 budget |
| Visible to client | 503+Retry-After | connection drop | TCP RST | latency | 503+Retry-After | OOM | Diagnosis at 3am |
| Bounded memory | yes | yes | yes (kernel) | yes (queue) | yes | **no** | NFR-A03 |
| Streaming TTFT preserved | yes | yes | yes | **no** | yes | yes | NFR-P05 |
| Composes with rate limiter + breaker vocabulary | yes (`Retry-After`) | no | no | no | yes | n/a | Three primitives must share one event vocabulary |
| Operator knobs | 2 (high/low water) | 1 (capacity) | 1 (capacity) | 1 (capacity) | 3+ (target, alpha, beta) | 0 | Fewer knobs = fewer misconfigurations |
| Reacts to upstream slowdown | no (queue depth proxy) | no | no | no | yes | no | Helpful but conflates with circuit-breaker domain |
| Existing Rust crates | trivial | trivial | trivial | trivial | none | trivial | Implementation cost |

## 5. Foundational principles

**Backpressure as policy (Hellerstein, Carlini, Akidau et al.).** The streaming-systems literature is explicit that backpressure is not a single mechanism but a family of policies that share an event vocabulary: *accept*, *delay*, *drop*, *reject*. The Riftgate posture — reject with `503 + Retry-After` — picks `reject` from this family and uses the HTTP layer to carry the event back to the client.

**Drop-on-full ring buffers (LMAX Disruptor).** The bounded-queue-with-explicit-saturation-policy pattern was popularised by the LMAX Disruptor and its descendants. The lesson is: making saturation a first-class event (rather than an emergent latency tail) is what makes a system debuggable under load.

**Little's law (`L = λW`).** Little's law tells us that for a queue at steady state, queue length equals arrival rate times residence time. This means picking a queue depth implicitly picks a worst-case admission latency for the requests at the tail. The high-water mark is the operator's lever for that tradeoff; the LLD names a default that targets the [`NFR-P06`](../01-requirements/non-functional.md) p99 latency budget.

**AIMD admission control (Jacobson, Brakmo, Netflix).** The AIMD lineage from TCP through Vegas to Netflix's `concurrency-limits` library shows that latency-aware admission can outperform a static threshold under variable load. We catalogue it as a future extension behind the same `BackpressurePolicy` trait rather than the v0.2 default; the conditions for revisit are named in §6.

**Composition with the rate limiter and circuit breaker.** Three protection primitives in a gateway are too many to debug independently. The decision is to make all three produce `Reject { retry_after, reason: DenialReason }` so that one observability surface and one client-side handler covers all of them.

## 6. Recommendation

**For `v0.2`: ship drop-newest with `503 + Retry-After`, bounded queue with high-water / low-water hysteresis, behind a `BackpressurePolicy` trait that accepts adaptive impls as future additions.**

Concretely:

1. The trait lives in `riftgate-core`:

   ```rust
   pub trait BackpressurePolicy: Send + Sync {
       fn on_enqueue(&self, depth: QueueDepth) -> AdmissionDecision;
   }

   pub enum AdmissionDecision {
       Accept,
       Reject { retry_after: Duration, reason: DenialReason },
   }
   ```

2. The default impl, `HighWaterPolicy`, holds `(capacity, high_water, low_water, drain_rate_hint)`. It maintains a single `AtomicBool gate_open`. Below `low_water` and currently-closed, it opens. At/above `high_water`, it closes. Between the two, it stays in its current state (hysteresis).
3. `retry_after` is computed as `(depth - low_water) / drain_rate_hint`, clamped to a configurable max.
4. `DenialReason` is shared with [`RateLimiter`](021-rate-limiting.md) and [`CircuitBreaker`](011-circuit-breaker.md) so OTel telemetry carries one structured cause label across all three primitives.
5. Config (TOML, per [Options `015`](015-config-model.md)):

   ```toml
   [backpressure]
   queue_capacity = 4096
   high_water     = 3686  # 90%
   low_water      = 2048  # 50%
   drain_rate_hint_per_sec = 2000
   max_retry_after_ms = 5000
   ```

6. Telemetry: `riftgate.queue.depth` (gauge), `riftgate.queue.rejected` (counter labelled by `reason`), `riftgate.queue.gate_state` (closed/open transitions). The dashboard story is "show me rejections by reason; show me queue depth over time; show me gate transitions."

### Conditions under which we'd revisit

- If operator feedback shows that hand-picked `queue_capacity` is the recurring source of v0.2 misconfiguration, an `AdaptiveConcurrencyPolicy` impl lands behind a feature flag. It replaces the default only after a v0.3 retro that names the AIMD parameter set.
- If we ever ship multi-tier QoS (Options `022`, gated on v0.2 retro), `BackpressurePolicy` extends with a per-class admission decision; the trait shape today (single `depth` argument) is intentionally minimal so the extension is additive.

## 7. What we explicitly reject

- **Drop-oldest (3.2).** Wastes already-done work and produces an indistinguishable failure shape on the wire. We will not ship it.
- **Block the accept loop or block the push (3.3, 3.4).** Both break TTFT ([`NFR-P05`](../01-requirements/non-functional.md)) and both make the rejection invisible to clients. We will not ship them.
- **Adaptive concurrency as the v0.2 default (3.5).** It is the right *future* shape but the wrong *first* shape. Catalogued; not shipped.
- **Unbounded queue (3.6).** Violates [`NFR-A03`](../01-requirements/non-functional.md). Will not ship.
- **A separate "backpressure binary" or sidecar.** The three protection primitives must live in the same kernel and share an event vocabulary; see §5.

## 8. References

1. Joseph M. Hellerstein, *The Declarative Imperative* (2010) — backpressure as a declarative policy primitive.
2. Tyler Akidau et al., *The Dataflow Model* (VLDB 2015) — backpressure in the streaming-systems literature.
3. Martin Thompson, Dave Farley, Michael Barker, Patricia Gee, Andrew Stewart, *Disruptor: High performance alternative to bounded queues for exchanging data between concurrent threads* (LMAX Technical Paper, 2011).
4. John D. C. Little, *A Proof for the Queuing Formula L = λW* (Operations Research, 1961).
5. Van Jacobson, *Congestion Avoidance and Control* (SIGCOMM 1988) — the AIMD lineage.
6. Lawrence S. Brakmo, Larry L. Peterson, *TCP Vegas: End to End Congestion Avoidance on a Global Internet* (IEEE JSAC, 1995).
7. Netflix, [`concurrency-limits`](https://github.com/Netflix/concurrency-limits) — the canonical reference for latency-aware admission control in services.
8. Envoy proxy, [`adaptive_concurrency` filter documentation](https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/filters/http/adaptive_concurrency/v3/adaptive_concurrency.proto).
9. RFC 7231 §6.6.4 — semantics of `503 Service Unavailable` and `Retry-After`.
10. Nginx, [`limit_conn_zone` documentation](http://nginx.org/en/docs/http/ngx_http_limit_conn_module.html).
