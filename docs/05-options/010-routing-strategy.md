# 010. Routing strategy

> **Status:** `recommended` — `v0.2` ships round-robin (already in v0.1) and adds weighted-random behind the same `Router` trait. KV-aware prefix routing and hedged requests are catalogued and explicitly deferred to `v0.3`. See [ADR `0014`](../06-adrs/0014-weighted-random-router.md).
> **Foundational topics:** weighted-random sampling (Walker's alias method, Vose 1991), KV-cache-aware prefix routing (vLLM, LMCache), hedged-request tail-latency reduction (Dean & Barroso, *The Tail at Scale*, 2013), least-loaded selection
> **Related options:** [`011 — circuit breaker`](011-circuit-breaker.md) (the breaker decorator skips Open backends), [`012 — backpressure`](012-backpressure.md), [`021 — rate limiting`](021-rate-limiting.md)
> **Related ADR:** [ADR `0014`](../06-adrs/0014-weighted-random-router.md). Successor / extension ADRs land in `v0.3` for KV-aware (Options `010` revisit) and hedged-requests.

## 1. The decision in one sentence

> Which routing strategies does `v0.2` ship behind the `Router` trait, and which strategies do we name now (KV-aware, hedged) but explicitly defer to `v0.3`?

## 2. Context — what forces this decision

v0.1 ships [`RoundRobinRouter`](../../crates/riftgate-router/) — atomic-cursor selection across N backends. This is correct for a walking skeleton and the natural starting point for the trait, but it is the wrong shape for two real v0.2 workloads:

1. **Heterogeneous capacity.** A deployment with one large backend and two small ones cannot express "send twice as much to the large one" with round-robin without duplicating backend entries in the config — a footgun.
2. **Health-aware skipping.** Once the [circuit breaker](011-circuit-breaker.md) lands, the router must skip Open backends. Round-robin's atomic-cursor advance has to be skipped-aware; the natural shape that emerges is `select_from(eligible_backends)`, which is also the natural shape for weighted-random.

Two further strategies are real but explicitly v0.3:

3. **KV-cache-aware routing** — route requests sharing a prompt prefix to the same backend so the upstream KV cache hits. This is the right thing to do; it requires either a prefix-trie data structure in the router or integration with vLLM's `lmcache` discovery. Both are v0.3 because they require the WASM extension surface ([Options `016`](README.md)) for the per-deployment prefix-hash policy.
4. **Hedged requests** — fire the same request to two backends and take whichever responds first; cancel the other. Reduces tail latency at the cost of doubled in-flight load. Requires stream-cancellation primitives that land with the v0.3 extension plane.

The forces:

- **`FR-103`** — per-backend health-aware routing in v0.2.
- **`FR-102`** — backend selection driven by declared weights (v0.2).
- **`NFR-P06`** — p99 latency budget; the upper bound on what tail-latency improvements (KV-aware, hedged) can buy us in v0.3.
- **The breaker integrates as a decorator** ([Options `011` §6](011-circuit-breaker.md)), which means the v0.2 routing surface stays minimal: pick from eligible backends.

## 3. Candidates

### 3.1. Round-robin (atomic cursor)

**What it is.** A single `AtomicU16` cursor; each request takes `cursor.fetch_add(1) % n_eligible`.

**Why it's interesting.**
- Trivial; correct under contention; cache-line-local atomic.
- Zero per-backend state.
- Already shipped in v0.1; v0.2 reuses it as the default.

**Where it falls short.**
- Cannot express weights without entry duplication.
- Strict round-robin under skipped backends needs a "next eligible" loop that has worst-case O(N).

**Real-world systems that use it.** Nginx default; HAProxy `roundrobin`. The universal first cut.

### 3.2. Weighted-random (Walker's alias method)

**What it is.** Each backend has a `weight: u32`. We build an alias table at config-load time (Vose 1991 — O(N) construction, O(1) sampling). At request time, we pick a slot uniformly, then either take that slot or its alias based on a second uniform draw. Total cost per selection: two RNG draws, one indirect load.

**Why it's interesting.**
- **O(1) sampling regardless of weight distribution.** Beats cumulative-weight-binary-search (O(log N)) on the hot path.
- **Weights are first-class.** `[[backend]] weight = 70` and `weight = 30` produces the expected 70:30 split. Operator intuition matches behaviour.
- **Composes cleanly with the breaker decorator.** When the breaker reports a subset of backends ineligible, we have two implementation choices: (a) rebuild a small alias table per request from the eligible subset (cheap if N is small), or (b) keep the full table and rejection-sample (cheap if Open fraction is low). The LLD names (a) as the v0.2 default with N ≤ 8 cap; above that we revisit.
- **Stable under config reload.** Adding/removing a backend invalidates the alias table; the rebuild is O(N) and happens on a config event, not the hot path.

**Where it falls short.**
- **Alias-table construction is non-trivial code.** Vose 1991 is well-documented and we use the standard small/large worklist algorithm.
- **The randomness is per-request.** For very small N (1-2 backends), the variance shows up before the law-of-large-numbers kicks in. Acceptable; documented.

**Real-world systems that use it.** Envoy's `WEIGHTED_LEAST_REQUEST` and weighted-cluster routing; Nginx's `upstream` `weight` parameter (which uses a smoothed weighted round-robin — a sibling); most modern load balancers.

### 3.3. Least-loaded (in-flight count)

**What it is.** Each backend has an `AtomicUsize in_flight`. Select the backend with the lowest count.

**Why it's interesting.**
- Latency-adaptive without explicit signal — slower backends accumulate more in-flight requests and stop being picked.
- Operator-intuitive ("send to whoever is least busy").

**Where it falls short.**
- **O(N) scan on the hot path.** For N=2-8 fine; for large fleets, less so.
- **Tie-breaking under cold start.** When all `in_flight` are equal, we degenerate to round-robin; the strategy is therefore "RR plus skew-correction."
- **Conflates routing with circuit-breaking signal.** A slow backend will hoard `in_flight` count, and the right primitive to handle that is the breaker, not the router.

**Real-world systems that use it.** HAProxy `leastconn`; Envoy `LEAST_REQUEST`. A real option, but the conflation-with-breaker concern makes it a v0.3+ shape for us if it lands at all.

### 3.4. KV-cache-aware prefix routing (deferred to v0.3)

**What it is.** Hash the request's prompt prefix (or a configured prefix length); route requests sharing that hash to the same backend so the backend's KV cache hits. Either implemented with a prefix trie in the router or by integration with the upstream's discovery API (vLLM's `lmcache`).

**Why it's interesting.**
- Real performance win for LLM workloads — KV-cache reuse can cut prefill time by an order of magnitude.
- Aligns with the v0.3 differentiation pillar (programmable Rust core + WASM extensions).

**Where it falls short.**
- Requires either a prefix-hash data structure on the hot path or a discovery integration; both are v0.3 work.
- Requires policy per deployment (which prefix length? which backends share a cache?); naturally a WASM extension.

**Real-world systems that use it.** vLLM-router (`lmcache`); Anyscale's routing layer. Real prior art.

### 3.5. Hedged requests (deferred to v0.3)

**What it is.** Fire the request to two backends; take whichever responds first; cancel the other. The Dean & Barroso (2013) *Tail at Scale* canonical reference.

**Why it's interesting.**
- Real p99 latency reduction.
- Lines up with v0.3's stream-cancellation primitives.

**Where it falls short.**
- Doubles in-flight load on every hedged request. Capacity planning becomes harder.
- Requires the v0.3 stream-cancellation primitive; cannot land standalone in v0.2.
- Subtle interaction with the rate limiter: a hedged request consumes two tokens; needs explicit policy.

**Real-world systems that use it.** Google's Bigtable client (the original reference); Envoy's `request_hedging` filter. Mature pattern; we want it, just not yet.

## 4. Tradeoff matrix

| Property | 3.1 RR | 3.2 Weighted-random | 3.3 Least-loaded | 3.4 KV-aware (v0.3) | 3.5 Hedged (v0.3) | Why it matters |
|---|---|---|---|---|---|---|
| Hot-path cost | 1 atomic add | 2 RNG draws | O(N) scan | hash + map lookup | 2x in-flight | NFR-P07 |
| Weight expressivity | none | first-class | none | n/a | n/a | FR-102 |
| Health-aware (composes with breaker) | yes | yes | yes | yes | yes | FR-103 |
| State per backend | 0 | weight + alias slot | atomic counter | prefix-trie node | n/a | Allocator footprint |
| KV-cache hit rate | unchanged | unchanged | unchanged | dramatically up | unchanged | LLM-workload tail |
| Tail latency | baseline | baseline | mild improvement | medium | strong | NFR-P06 |
| Implementation cost in v0.2 | shipped | medium | small | high (needs WASM) | high (needs stream cancel) | v0.2 capacity |
| Capacity planning impact | predictable | predictable | predictable | predictable | doubled load | Operator surprise |

## 5. Foundational principles

**Walker's alias method (Vose 1991).** The alias method gives O(1) weighted sampling with O(N) construction. This is the right hot-path shape: the operator picks the weights once at config-load and the router takes two RNG draws per request regardless of how skewed the distribution is.

**Health-aware selection via decorator (composition).** The breaker is a decorator over the router rather than a feature of the router (per [Options `011`](011-circuit-breaker.md) §6). This keeps each protection primitive focused and composes cleanly: any future router (weighted-random, KV-aware, hedged) inherits the breaker without modification.

**The tail at scale (Dean & Barroso, 2013).** The canonical reference for hedged requests and tail-latency reduction. We name the technique in v0.2 and defer the impl to v0.3 because the stream-cancellation primitives needed are v0.3 work.

**Sidecar / ambassador pattern (Microsoft *Cloud Design Patterns*, Hohpe *EIP*).** The gateway-as-ambassador framing means routing decisions are first-class observable events; every selection emits an OTel span with the backend chosen and the selection mechanism. The dashboard story is "show me selection distribution; show me eligibility events."

## 6. Recommendation

**For `v0.2`: ship round-robin (already in v0.1) and add weighted-random behind the `Router` trait. Health-awareness via the breaker decorator. KV-aware and hedged are catalogued and explicitly deferred to `v0.3`. Least-loaded is rejected for v0.2.**

Concretely:

1. The `Router` trait in `riftgate-core` is unchanged from v0.1. Its surface (`select_for(request) -> Option<BackendId>`, `report_outcome(...)`) accommodates every candidate above.
2. `WeightedRandomRouter` lands in `crates/riftgate-router`. Internal: Walker alias table built at config-load and on every config-reload event; per-request `Xoshiro256++` PRNG (fast, no syscalls). Cap N (eligible backends) at 32 for v0.2; above that, the LLD names the rebuild-vs-rejection-sample decision.
3. Config (per [Options `015`](015-config-model.md)):

   ```toml
   routing_strategy = "weighted_random"   # or "round_robin"

   [[backend]]
   name = "openai-primary"
   weight = 70

   [[backend]]
   name = "openai-secondary"
   weight = 30
   ```

   Missing `weight` defaults to 1 (uniform); RR ignores `weight`.
4. The binary wires `CircuitBreakerArbiter::new(WeightedRandomRouter::new(...))`; the decorator filters Open backends out of the eligible set passed to the inner router's selection.
5. Telemetry: `riftgate.routing.selected` (counter labelled by `backend, strategy`), `riftgate.routing.skipped` (counter labelled by `backend, reason`), `riftgate.routing.eligible_count` (gauge).

### Conditions under which we'd revisit

- If operator feedback (or our own benchmark) shows weighted-random produces unacceptable variance at small N (≤ 2), we land smoothed-weighted-round-robin as an additional impl (Nginx's shape) and let operators choose.
- KV-aware routing is the headline differentiator for v0.3. The conditions for revisit are not "if" but "when v0.3 opens" — the Options doc gets a successor or an extension at that point.
- Hedged requests land in v0.3 alongside the stream-cancellation primitives.

## 7. What we explicitly reject

- **Least-loaded as a v0.2 default.** Conflates with the breaker domain; the breaker is the cleaner primitive for handling slow backends. Catalogued; not shipped.
- **KV-aware in v0.2.** Requires v0.3 WASM extension surface for the per-deployment prefix-hash policy. Catalogued.
- **Hedged in v0.2.** Requires v0.3 stream-cancellation primitives. Catalogued.
- **Smoothed-weighted-round-robin as the only v0.2 weighted impl.** Alias method is the standard hot-path-O(1) shape; smoothed-WRR is the Nginx-specific variant. We can add it later if N=2 variance becomes a real complaint.
- **Random selection with no weights.** Strictly worse than weighted-random with uniform weights.
- **Routing-strategy plugins as WASM in v0.2.** Plugin extensibility is the v0.3 differentiator. v0.2 ships in-tree impls only.

## 8. References

1. Michael D. Vose, *A Linear Algorithm for Generating Random Numbers with a Given Distribution* (IEEE TSE, 1991) — alias method.
2. Donald E. Knuth, *The Art of Computer Programming*, Vol. 2 §3.4.1 — weighted sampling background.
3. Jeffrey Dean, Luiz André Barroso, *The Tail at Scale* (CACM, 2013) — hedged requests; the canonical reference.
4. vLLM, [`lmcache` documentation](https://github.com/vllm-project/vllm) — KV-cache-aware routing prior art.
5. Envoy proxy, [routing documentation](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/cluster_manager) — weighted clusters, hedging filter.
6. Nginx, [`upstream` module](http://nginx.org/en/docs/http/ngx_http_upstream_module.html) — `weight` and smoothed-WRR.
7. HAProxy, [`balance` algorithms documentation](https://docs.haproxy.org/2.8/configuration.html#4-balance) — roundrobin, leastconn, source.
8. Microsoft, *Cloud Design Patterns: Ambassador* — <https://learn.microsoft.com/azure/architecture/patterns/ambassador>.
9. Gregor Hohpe, Bobby Woolf, *Enterprise Integration Patterns* (Addison-Wesley, 2003).
10. David Vitter, [*Random sampling with a reservoir*](https://www.cs.umd.edu/~samir/498/vitter.pdf) (ACM TOMS, 1985) — sampling literature background.
