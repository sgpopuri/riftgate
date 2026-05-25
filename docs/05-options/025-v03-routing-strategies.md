# 025. v0.3 routing strategies — KV-cache-aware and hedged requests

> **Status:** `recommended` — `v0.3` ships `KvAwareRouter` (in-tree prefix trie, hash-only — no tokenizer) and `HedgedRouter` (p99-triggered, fixed degree=2, rate-limit-budget-aware). LMCache-delegated KV routing is catalogued and rejected for `v0.3`. See [ADR `0022`](../06-adrs/0022-kv-aware-routing-prefix-trie.md) and [ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md). This Options doc is the v0.3 successor to [Options `010`](010-routing-strategy.md).
> **Foundational topics:** prefix tries / radix trees (Knuth TAOCP §6.3), hash-based prompt-prefix fingerprinting (vLLM's prefix-aware routing prior art; LMCache lookup model), hedged requests for tail-latency reduction (Dean & Barroso, *The Tail at Scale*, 2013), Google Bigtable client's hedging contract, FNV-1a / xxHash for hot-path hashing, bounded-load consistent hashing (Mirrokni et al., 2016) for the fallback when the trie misses.
> **Related options:** [`010 — routing strategy`](010-routing-strategy.md) (the v0.2 doc this supersedes for the KV-aware and hedged candidates), [`011 — circuit breaker`](011-circuit-breaker.md) (the decorator wraps both new routers), [`021 — rate limiting`](021-rate-limiting.md) (hedging consumes rate-limit budget), [`024 — stream cancellation`](024-stream-cancellation.md) (hedged routing's primary consumer of cancellation), [`016 — extension mechanism`](016-extension-mechanism.md) (per-route hedging policy is configurable, but in v0.3 we keep it in TOML, not WASM)
> **Related ADR:** [ADR `0022`](../06-adrs/0022-kv-aware-routing-prefix-trie.md) and [ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md)

## 1. The decision in one sentence

> What concrete shape do the two v0.3 routing strategies — KV-cache-aware prefix routing and hedged requests — take in `crates/riftgate-router`, given that the trait surface is fixed and the deferred-from-v0.2 problem space is now load-bearing?

## 2. Context — what forces this decision

[Options `010` §3.4–§3.5](010-routing-strategy.md) catalogued KV-aware routing and hedged requests for v0.3. v0.2 [ADR `0014`](../06-adrs/0014-weighted-random-router.md) shipped `RoundRobinRouter` and `WeightedRandomRouter` behind the `Router` trait and explicitly punted the two v0.3 candidates. The trait surface in [`crates/riftgate-core/src/router.rs`](../../crates/riftgate-core/src/router.rs) already accommodates them:

```rust
pub enum RoutingDecision {
    Send(BackendId),
    Hedge(Vec<BackendId>),     // declared in v0.2 for v0.3 to fulfil
    Reject(StatusCode),
}
```

And the [v0.3 stream-cancellation primitive](024-stream-cancellation.md) lands in this milestone, removing the only true blocker on hedging.

What we have to decide now, with all the deferral excuses gone:

1. **KV-aware: prefix-trie or LMCache delegation?** Both are real. The in-tree trie is independent and operator-simple. LMCache delegation makes the gateway part of a larger vLLM ecosystem at the cost of an external dependency and an over-the-wire lookup per request.
2. **KV-aware: hash the bytes, or tokenize first?** Hashing bytes is fast but fragile to whitespace and unicode normalisation. Tokenising matches what the backend actually caches, but tokenising on the hot path costs latency we don't have.
3. **Hedged: always, threshold-triggered, or per-route?** Always-hedge doubles upstream load. Threshold-triggered (only hedge after observing the primary is slow) is the published Dean–Barroso shape. Per-route is operator-controlled.
4. **Hedged: degree?** Two backends or N? Bigtable's hedged-read uses degree=2 with a 95th-percentile-latency timer. Going higher trades capacity for tail.
5. **Hedged: how does this compose with rate limiting and circuit breaking?** A hedged request consumes two tokens at the limiter and stresses two backends; the circuit breaker must agree on which (if either) failure counts.

Requirements this is load-bearing for:

- **`FR-203`** — KV-cache-aware routing for LLM-prefill optimisation.
- **`FR-202`** — hedged-request support for tail-latency reduction.
- **`NFR-P11`** — p99 wall-clock latency for the routing decision must remain ≤ 50µs at p99 across all impls; `KvAwareRouter` is the budget-sensitive one.
- **`NFR-COST04`** — operators must be able to express "hedge no more than X% of traffic" so doubled load is bounded.

## 3. Candidates

The doc covers KV-aware (§3.A) and hedged (§3.B) separately because the two decisions are independent: an operator can pick KV-aware without hedging, hedged without KV-aware, or both. The candidates and tradeoff matrices reflect that.

### 3.A — KV-cache-aware routing

#### 3.A.1. In-tree prefix trie, byte-hash keys

**What it is.** Build a prefix trie keyed by a rolling hash (FNV-1a or xxHash3, 64-bit) of the request's prompt bytes. Each trie node carries an `Option<BackendId>` — the last backend that served this prefix. On a `route()` call, we hash the request's prompt-prefix in chunks of `prefix_chunk_size` bytes (default 64), walk the trie greedily, and return the deepest backend match. On `on_response`, we annotate the trie with the chosen backend so the next request with the same prefix hits.

**Why it's interesting.**
- **Zero external dependency.** Operator deploys Riftgate, configures `routing_strategy = "kv_aware"`, done.
- **Fast.** Hash is O(prefix_length / 64); trie walk is O(depth) ≤ O(prefix_length / 64). Both fit inside `NFR-P11` with margin.
- **Bounded memory.** Trie pruned with an LRU policy (default 100k entries); hash-collision-resistant within reason for 64-bit hashes.
- **Aligns with vLLM's behaviour.** vLLM caches by token-prefix; byte-prefix is a slightly weaker signal but correlates strongly (same bytes → same tokens for the same tokenizer).
- **Composes with the breaker decorator.** When the trie picks a backend that the breaker reports Open, we fall back to the inner router (weighted-random by default).

**Where it falls short.**
- **Whitespace / unicode normalisation sensitivity.** Two requests whose prompts differ only in trailing newline hash to different bins. We mitigate with a documented `prefix_normalisation = "trim_trailing_whitespace" | "nfkc" | "none"` config — default "trim_trailing_whitespace".
- **Hash collisions.** 64-bit FNV-1a has known weaknesses; xxHash3 is stronger. We use xxHash3-64 (in `xxhash-rust`, no_std-friendly) and document the residual collision risk (negligible at 100k-entry scale).
- **No cross-process state.** Two Riftgate replicas behind an L4 LB have independent tries; identical traffic does not concentrate on the same backend unless the L4 LB is consistent-hash-by-prefix-header (which the operator can opt into via the future [Options `017` multitenancy](README.md) story).

**Real-world systems that use it.** Custom in-tree prefix routing in several production LLM gateways. Generic radix-tree HTTP routers (`matchit`) use the same data structure for a different key.

#### 3.A.2. LMCache delegation

**What it is.** `KvAwareRouter` becomes a thin HTTP client to [LMCache](https://github.com/LMCache/LMCache) (or vLLM's prefix-aware router service); each request consults the LMCache lookup endpoint to decide which backend has the prefix cached. The trie lives in LMCache, not in Riftgate.

**Why it's interesting.**
- **Reuses upstream-ecosystem semantics.** Same notion of "this backend cached this prefix" as the rest of the vLLM stack.
- **Cross-replica consistency for free.** LMCache is the source of truth; all Riftgate replicas see the same routing decisions.
- **Active research community.** LMCache evolves; we ride the upstream improvements.

**Where it falls short.**
- **Adds a network hop on the routing hot path.** LMCache lookup is typically gRPC or HTTP; even a 1ms LAN round-trip eats half of our `NFR-P11` 50µs budget. We would have to introduce per-request caching of LMCache decisions, which reintroduces the trie we tried to escape.
- **Hard dependency on a non-Riftgate service.** Operators who do not run LMCache cannot use `kv_aware`. Defeats the operator-simplicity case.
- **Couples Riftgate's release cadence to LMCache's protocol stability.**
- **vLLM-specific.** Backends that are not vLLM (TGI, Ollama, OpenAI proxy chains) get no benefit.

**Real-world systems that use it.** `vllm-router` reference deployments. Useful in single-stack vLLM environments; awkward in mixed-backend gateways.

#### 3.A.3. Bounded-load consistent hashing (no trie)

**What it is.** Hash the prompt-prefix bytes, map the hash modulo `n_backends` (or via jump-consistent hashing) — but with Mirrokni et al.'s bounded-load adjustment that prevents any one backend from carrying more than `(1+ε) × avg_load`.

**Why it's interesting.**
- **No state.** Pure function; no trie to maintain; no LRU.
- **Cross-replica consistent automatically.** Identical hash → identical backend across replicas.
- **Bounded load.** No hot-spotting on a popular prefix.

**Where it falls short.**
- **No real KV-cache hit guarantee.** Two requests sharing a 4KB prefix and differing at byte 4097 hash to different bins — we lose the cache hit we are trying to capture. The trie's "share the longest common prefix" semantics are what produces the cache hit; pure hashing loses it.
- **The bounded-load adjustment defeats the cache-locality property when load is skewed.** When a popular backend hits the load ceiling, requests get redirected — losing the cache hit again.

**Real-world systems that use it.** Some content-addressable caches and L4 load balancers; rarely as an LLM-prefix router.

### 3.B — Hedged requests

#### 3.B.1. Always-hedge

**What it is.** Every request goes to two backends in parallel; the first response wins.

**Why it's interesting.**
- **Maximum tail-latency improvement.** No request waits for the slowest backend.
- **Trivial to implement.** `RoutingDecision::Hedge(vec![a, b])` for every request.

**Where it falls short.**
- **Doubles steady-state load.** Capacity planning becomes "size for 2x peak." Operators correctly refuse.
- **Doubles rate-limit consumption.** A tenant rate-limited at 100 RPS now sees 50 effective RPS, surprising operators.
- **Worst-case for cost-sensitive deployments.** Paying double on every request to shave a tail you might not have is a bad trade.

**Real-world systems that use it.** Rarely; some research gateways. Documented as a teaching example, not a production default.

#### 3.B.2. Threshold-triggered (Dean–Barroso shape)

**What it is.** Send the request to backend A. Start a timer (default: backend A's recent p95 latency, refreshed every minute). If the timer fires before A's first byte arrives, dispatch the same request to backend B; whichever responds first wins, the loser is cancelled (per [Options `024`](024-stream-cancellation.md)). If A responds before the timer fires, no hedge happens; cost is unchanged.

**Why it's interesting.**
- **Bounded extra load.** Only the slowest ~5% of requests hedge, by construction. Capacity overhead is small and predictable.
- **Canonical reference.** Dean–Barroso 2013 *The Tail at Scale*; Google Bigtable's hedged-read uses exactly this shape; Cassandra's `speculative_retry` is the same idea.
- **Composes with cancellation.** The loser is cancelled via the v0.3 cancellation primitive — clean, observable, telemetered.

**Where it falls short.**
- **Requires per-backend latency tracking.** The timer threshold is backend A's p95; we need a running quantile estimator per backend. Solvable (P² algorithm, t-digest, or HDR histogram); not free.
- **First-byte-of-SSE is the trigger event, not full-response-time.** TTFT-shaped trigger; we time the first byte. Choose this deliberately; document it.
- **Tuning interaction with the breaker.** If the timer fires often for a specific backend, the breaker should consider that backend "slow"; the integration is straightforward but real.

**Real-world systems that use it.** Google Bigtable; Cassandra; Envoy's `request_hedging` filter; Google SRE Book chapter 22.

#### 3.B.3. Always-hedge with degree N

**What it is.** Fire to N backends in parallel; take the first.

**Why it's interesting.**
- N=3 cuts the tail further than N=2 in theory.

**Where it falls short.**
- **Capacity overhead is N×.** N=3 means 3x capacity. Operators refuse.
- **Diminishing returns past N=2** in published studies (Dean–Barroso themselves observe this).

**Real-world systems that use it.** Almost none. Catalogued for completeness.

#### 3.B.4. Per-route operator configuration only

**What it is.** No automatic triggering. Operators flag specific routes as `hedge = true` and accept the always-hedge cost on those routes.

**Why it's interesting.**
- Maximum operator control.
- Predictable cost (operator declared it).

**Where it falls short.**
- **Misses the dynamic-tail use case.** A backend that becomes slow at 03:00 due to a partial GPU failure benefits from automatic hedging; static configuration cannot react.
- **Operator burden.** Tail-latency-sensitive routes are not always knowable in advance.

**Real-world systems that use it.** Some configuration-driven gateways. Useful as a *complement* to threshold-triggered, not an alternative.

## 4. Tradeoff matrix

### KV-aware

| Property | 3.A.1 In-tree trie | 3.A.2 LMCache | 3.A.3 Consistent hash | Why it matters |
|---|---|---|---|---|
| Hot-path cost | hash + O(depth) | LAN RTT (~1 ms) | hash only | `NFR-P11` 50µs. |
| Real KV-cache hit | yes (longest prefix) | yes (upstream truth) | weak (exact-match only) | The point of the strategy. |
| External dependency | none | LMCache | none | Operator simplicity. |
| Cross-replica consistency | no (independent tries) | yes | yes | Multi-replica deployments. |
| Memory footprint | O(LRU capacity) | none locally | none | Allocator pressure. |
| Backend-stack lock-in | none | vLLM | none | Backend pluggability. |
| Telemetry depth | full (we own the trie) | partial (upstream owns) | full | OTel attribution. |
| Migration to LMCache later | trait-shaped | n/a | trait-shaped | Future-proofing. |

### Hedged

| Property | 3.B.1 Always | 3.B.2 Threshold | 3.B.3 Always degree N | 3.B.4 Per-route only | Why it matters |
|---|---|---|---|---|---|
| Extra load in steady state | 2× | small (~5%) | N× | bounded by config | `NFR-COST04`. |
| Tail latency improvement | maximum | strong | maximum | route-specific | The goal. |
| Capacity-planning surprise | severe | mild | severe | none | Operator trust. |
| Rate-limit interaction | doubled cost | small | N× cost | declared | `021` integration. |
| Implementation complexity | low | medium | low | low | v0.3 cost. |
| Compose with cancellation | yes | yes | yes | yes | `024` integration. |
| Tunable per route | poor fit | good fit | poor fit | only fit | Operator control. |
| Reacts to dynamic tail | yes | yes | yes | no | Production reality. |
| Canonical reference | none | Dean–Barroso 2013 | none | none | Pedagogical clarity. |

## 5. Foundational principles

**Prefix tries / radix trees (Knuth TAOCP §6.3).** The longest-common-prefix property is the algorithmic reason a trie matches the KV-cache-hit objective. We are not building a generic prefix-search structure; we are building one whose leaf data is `Option<BackendId>` and whose internal nodes carry an LRU touch counter for eviction. The trie footprint is bounded by the LRU capacity; the walk cost is bounded by the chunked-hash depth.

**vLLM prefix-aware routing prior art.** The decision to hash bytes rather than tokenise is *deliberate* and is what makes the v0.3 in-tree impl viable: byte-prefix hashing is a small fraction of the cost of running a tokenizer on the hot path. We accept the documented edge cases (whitespace, normalisation) as acceptable v0.3 trade-offs; tokenizer-accurate KV routing is a v1.0+ option if benchmarks warrant.

**Dean–Barroso, *The Tail at Scale* (CACM, 2013).** The canonical reference. The paper establishes that (a) tail-latency dominates user-perceived performance in fan-out systems, and (b) threshold-triggered hedging gives most of the benefit of always-hedge at a small fraction of the cost. We follow the paper's recipe almost exactly: the trigger is the primary backend's recent p95 first-byte latency, the degree is two, and the loser is cancelled.

**Bounded-load consistent hashing (Mirrokni et al., 2016).** Catalogued for completeness; rejected for the v0.3 KV path because it loses the longest-prefix property. The technique remains relevant for future session-affinity routing (per [`docs/04-design/lld-routing.md`](../04-design/lld-routing.md) open questions).

**Composition with circuit breaker decorator and rate limiter.** Both new routers compose under `CircuitBreakerArbiter` ([ADR `0016`](../06-adrs/0016-three-state-circuit-breaker.md)) — Open backends are filtered before the trie or hedge sees them. Hedged requests count as two events at the rate limiter ([Options `021`](021-rate-limiting.md)) for the tenant's overall budget but as one event at the per-request log (only one wins). This composition is documented in the LLD refresh.

**FSM-based cancellation transition.** The hedged loser's cancellation, per [Options `024`](024-stream-cancellation.md), is a typed transition with `CancelCause::HedgedLoser { winner: BackendId }`. Telemetry on the loser's cancellation includes `bytes_seen_before_cancel`, which is the data we need to refine the trigger threshold over time (the "if the loser had already streamed 80% of the response when cancelled, we hedged too late or unnecessarily" feedback loop).

## 6. Recommendation

**For KV-aware routing: ship the in-tree prefix-trie router (§3.A.1) with xxHash3-64 byte-hashing, configurable LRU capacity, and `prefix_normalisation = "trim_trailing_whitespace"` by default. Reject LMCache delegation for v0.3.**

**For hedged routing: ship the Dean–Barroso threshold-triggered hedge (§3.B.2) with degree=2, p95-first-byte-latency timer per backend (P² estimator), and a rate-limit-budget-aware policy (`hedge_max_fraction` config). Reject always-hedge and per-route-only for v0.3.**

Concretely:

1. **`KvAwareRouter` in `crates/riftgate-router`:**

   ```rust
   pub struct KvAwareRouter<R: Router> {
       fallback: R,                    // typically WeightedRandomRouter
       trie: PrefixTrie,               // LRU-bounded
       config: KvAwareConfig,
   }

   pub struct KvAwareConfig {
       pub prefix_chunk_bytes: usize,           // default 64
       pub max_trie_entries: usize,             // default 100_000
       pub prefix_normalisation: PrefixNorm,    // default TrimTrailingWhitespace
       pub min_prefix_bytes_to_route: usize,    // default 256 (don't route trivially-short prompts)
   }
   ```

   On `route()`: hash the prefix; walk the trie; if a leaf names an eligible backend (passes breaker filter), return `Send(backend)`. Otherwise, delegate to `fallback.route(...)`.

   On `on_response()`: annotate the trie with the chosen backend. LRU touches update on hit.

2. **`HedgedRouter` in `crates/riftgate-router`:**

   ```rust
   pub struct HedgedRouter<R: Router> {
       inner: R,                       // typically WeightedRandomRouter or KvAwareRouter
       latencies: BackendLatencyStats, // per-backend P² estimator for first-byte p95
       config: HedgeConfig,
   }

   pub struct HedgeConfig {
       pub degree: usize,                       // fixed at 2 in v0.3; doc says "increase requires new ADR"
       pub hedge_after_quantile: f32,           // default 0.95 (p95)
       pub hedge_max_fraction: f32,             // default 0.05 (≤5% of traffic may hedge)
       pub hedge_min_threshold_ms: u32,         // default 50 (never hedge faster than this)
   }
   ```

   On `route()`: returns `Hedge(vec![primary, secondary])` only when (a) the global `hedge_max_fraction` budget permits and (b) the primary's recent p95 first-byte latency exceeds `hedge_min_threshold_ms`. Otherwise returns `Send(primary)`. The actual decision *to fire the second leg* lives in the request driver: it starts the primary, sets a timer at `primary.p95_first_byte_ms`, and dispatches the secondary only if the timer fires.

   On `on_response()`: feeds first-byte latency into the P² estimator; tracks hedge-budget consumption.

3. **Composition.** The binary wires `CircuitBreakerArbiter::new(HedgedRouter::new(KvAwareRouter::new(WeightedRandomRouter::new(...))))`. The decorator stack reads: breaker filters → hedge decides to fan out → KV-aware picks the primary → weighted-random is the fallback. Both new routers are decorator-shaped over an inner `Router`, mirroring the v0.2 breaker pattern.

4. **Configuration:**

   ```toml
   routing_strategy = "weighted_random"  # base layer
   kv_aware = true                       # decorator
   hedge   = true                        # decorator

   [kv_aware]
   prefix_chunk_bytes        = 64
   max_trie_entries          = 100_000
   prefix_normalisation      = "trim_trailing_whitespace"
   min_prefix_bytes_to_route = 256

   [hedge]
   degree                  = 2
   hedge_after_quantile    = 0.95
   hedge_max_fraction      = 0.05
   hedge_min_threshold_ms  = 50
   ```

5. **Telemetry:**
   - `riftgate.routing.kv_aware.hit` / `.miss` counters labelled by depth bucket.
   - `riftgate.routing.kv_aware.trie_evictions` counter.
   - `riftgate.routing.hedge.fired` counter; `riftgate.routing.hedge.budget_blocked` counter.
   - `riftgate.routing.hedge.winner` histogram labelled by `winner = primary|secondary`.
   - `riftgate.routing.hedge.bytes_wasted_total` (bytes the loser had streamed before cancellation).

6. **Test gates.** Statistical tests for trie hit-rate on a synthetic prompt-prefix workload; correctness tests for hedge-loser cancellation (the loser's `CancelCause::HedgedLoser { winner }` must be observed in tracing); budget tests that `hedge_fraction_observed ≤ hedge_max_fraction + tolerance` over 10k requests; latency-improvement tests showing p99 improvement on a workload with 10% slow-backend artificial delay.

### Conditions under which we'd revisit

- If a deployment hits the trie LRU eviction limit consistently, we revisit `max_trie_entries`'s default. Adjustment is config-only; no ADR needed.
- If operator demand grows for LMCache integration, we add `LmcacheRouter` as a third impl behind the same `Router` trait — `KvAwareRouter` and the new impl coexist; operators pick one.
- If degree=3 hedging becomes empirically justified for a real workload (which we doubt), we open a new ADR explicitly raising the cap.
- If the hedge-budget enforcement causes underhedging on bursty workloads (the rate-limit-budget runs out exactly when the tail spikes), we revisit the budget shape (per-tenant, per-minute, EWMA) without changing the trigger semantics.

## 7. What we explicitly reject

- **LMCache delegation in v0.3.** Network hop on the routing hot path; external-service dependency; vLLM-specific. Catalogued; we keep the trait-shape so a future impl can land without breakage.
- **Tokenizer-accurate KV routing in v0.3.** Tokenising on the hot path costs too much for the marginal hit-rate improvement over byte-hash prefix. Revisit at v1.0 with measured data.
- **Bounded-load consistent hashing for KV-aware routing.** Loses the longest-prefix property; not the right tool. Retained as a relevant data structure for future session-affinity, where it is the right tool.
- **Always-hedge.** Doubles steady-state load. Catalogued as a teaching example; never a production default in Riftgate.
- **Degree > 2 hedging.** Diminishing returns; capacity cost. Decision lockable; revisit only with measured production data.
- **Per-route-only hedging (without a dynamic trigger).** Misses the dynamic-tail use case; remains available as an additional configuration knob on top of threshold-triggered.
- **KV-aware routing as a WASM filter rather than a `Router` impl.** Tempting (it would be programmable), but routing is too hot-path for the WASM dispatch cost; routing strategy stays as in-tree Rust under the `Router` trait. The WASM extension surface ([Options `016`](016-extension-mechanism.md)) is for filters, not for routing decisions in v0.3.

## 8. References

1. Jeffrey Dean, Luiz André Barroso, *The Tail at Scale* (Communications of the ACM, 2013) — <https://research.google/pubs/the-tail-at-scale/>
2. Google SRE Book, chapter 22 — *Addressing Cascading Failures* (hedged-request discussion) — <https://sre.google/sre-book/addressing-cascading-failures/>
3. Apache Cassandra documentation, *Speculative retry* — <https://cassandra.apache.org/doc/latest/cassandra/operating/read_repair.html>
4. Envoy proxy, *Request hedging filter* — <https://www.envoyproxy.io/docs/envoy/latest/configuration/http/http_filters/router_filter>
5. Donald E. Knuth, *The Art of Computer Programming*, Volume 3: *Sorting and Searching* (2nd ed., 1998) — §6.3 on digital searching (tries).
6. Vahab Mirrokni, Mikkel Thorup, Morteza Zadimoghaddam, *Consistent Hashing with Bounded Loads* (arXiv 1608.01350, 2016) — <https://arxiv.org/abs/1608.01350>
7. Yann Collet, *xxHash specification* — <https://cyan4973.github.io/xxHash/>
8. vLLM project, *prefix-caching* documentation — <https://docs.vllm.ai/en/latest/usage/automatic_prefix_caching.html>
9. LMCache — <https://github.com/LMCache/LMCache>
10. Raj Jain, Imrich Chlamtac, *The P² Algorithm for Dynamic Calculation of Quantiles and Histograms Without Storing Observations* (CACM, 1985) — the per-backend p95 estimator.
11. David R. Karger et al., *Consistent Hashing and Random Trees* (STOC 1997).
12. John Lamping, Eric Veach, *A Fast, Minimal Memory, Consistent Hash Algorithm* (2014) — jump-consistent hash.
