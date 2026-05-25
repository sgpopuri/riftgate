# 027. Token-level metrics: how Riftgate measures TTFT, inter-token latency, and per-token cardinality without melting Prometheus

> **Status:** recommended
> **Foundational topics:** streaming sketches (Count–Min Sketch, HyperLogLog), reservoir sampling (Vitter), HDR histograms, OTel head-vs-tail sampling, WAL-versus-metrics split for high-cardinality data, per-token timing semantics
> **Related options:** [008](008-stream-framing.md), [009](009-request-log.md), [013](013-observability-sink.md), [014](014-ebpf-integration.md), [028](028-gpu-pressure-correlation.md)
> **Related ADR:** [ADR 0025](../06-adrs/0025-token-level-metrics-probabilistic.md)

## 1. The decision in one sentence

> Pick the shape and substrate of the per-token observability data structure that lands in `v0.4` — what we measure per token, where it lives (OTel attributes vs WAL vs probabilistic sketch), and how we keep cardinality bounded.

## 2. Context — what forces this decision

`v0.4`'s `NFR-OBS04` adds TTFT (time to first token), inter-token latency, and token jitter as first-class observability. The [observability-plane document](../03-architecture/observability-plane.md) names these explicitly; the [observability LLD](../04-design/lld-observability.md) reserves the `TokenLevelAggregator` sink and even sketches the candidate probabilistic structures (HLL, CMS, reservoir sampling). What the LLD does *not* do is pick the sampling policy, the per-token-attribute placement (OTel vs WAL), or the cardinality-control structure. Those are the loads of this Options doc.

The forces:

- **Cardinality explodes by design.** A production gateway sees on the order of `(num_tenants × num_models × num_routes)` distinct buckets and on the order of `O(10^8)` tokens per day. Carrying token IDs as Prometheus label values would melt any backend within a day. The standard answer — keep counts approximate via sketches — is well-charted streaming-algorithms territory.
- **TTFT is a streaming property.** It cannot be computed from a single `histogram` observation taken at request-end; it must be observed at first-token emission to the client. This requires coordinated emission from `SseFramer::feed()` in [`crates/riftgate-parser/src/sse.rs`](../../crates/riftgate-parser/src/sse.rs) (which already emits `Emit::Token(start..end)`) and the IO subsystem at the byte-egress point.
- **Inter-token latency is per-stream.** Computing it requires per-request state (last-token-emit-time). That state already exists on the request task; what's new is the *publish* side.
- **The bus is frozen.** Everything publishes via `ObservabilityEvent` over the bounded MPSC bus per [ADR `0011`](../06-adrs/0011-otel-default-sink-multisink-fanout.md). Token-level data must fit in `Counter`, `Histogram`, and `SpanStart` / `SpanEnd` variants — or we add a new variant, which costs an ADR superseding `0011`. We prefer the former.
- **The WAL exists and is the right home for per-token bytes.** [ADR `0013`](../06-adrs/0013-append-only-file-wal.md) shipped a per-shard append-only file WAL. Per-token data — full token boundaries, byte offsets, timestamps — belongs there, not in OTel.
- **Replay-eval consumes token-level data.** The `riftgate-replay eval` CLI per [ADR `0021`](../06-adrs/0021-external-replay-cli.md) computes aggregate metrics (TTFT distributions, schema-conformance rates) over recorded streams. The eval surface and the live metrics surface should not invent separate per-token formats.

The bedrock question: of the data we emit *per token*, what is observable in OTel/Prometheus, what is bounded by a sketch, what is sampled, and what goes to the WAL only?

## 3. Candidates

### 3.1. Per-token OTel events — no sampling, no sketches

**What it is.** Every token emission produces an `ObservabilityEvent::SpanStart`-style event with full attributes: `(request_id, token_index, byte_offset, model, tenant, latency_since_last_token)`. The event flows through the bus and out to OTel; the OTel collector handles cardinality control downstream.

**Why it's interesting.** Maximum fidelity. Every per-token question is answerable from OTel without joining to the WAL. Operators with dedicated trace backends (Tempo, Jaeger) get rich data for free. There is exactly one place to look.

**Where it falls short.** Catastrophic at scale. `O(10^8)` events per day per Riftgate replica, each with 5–10 attributes, saturates any OTel collector and any Prometheus-shaped derived metrics path. The bus's drop-on-full property turns into "we drop most tokens during peak load" — which silently corrupts the very SLOs we are trying to measure. The cardinality of `token_index` alone (unbounded) makes derived histograms unusable.

**Real-world systems that use it.** None at production scale. Some research and benchmarking setups do this temporarily.

### 3.2. Reservoir-sampled per-token events plus full-fidelity WAL

**What it is.** Per-token events are sampled at OTel emission via Vitter's reservoir sampling — keep a bounded `K`-sized reservoir per `(tenant, model, route)` tuple per time window. The reservoir is published to OTel as `K` per-token spans per window per tuple. The *full* per-token byte boundaries and timestamps go to the WAL, where they cost append-only-disk-write rather than network bandwidth or OTel collector cardinality. Token *aggregates* (TTFT histograms, inter-token-latency histograms) are computed locally via HDR-style histograms and published to OTel/Prometheus per normal cadence.

**Why it's interesting.** Three-layer split matches the three questions operators ask. *Aggregate behaviour* → HDR histogram via Prometheus (bounded by histogram-bucket count, not by token count). *Distribution shape and slow-tail forensics* → bounded random sample via OTel (count-bounded, not rate-bounded; favours rare slow streams over hot fast ones). *Per-token forensics for one specific request* → WAL, joined to OTel span by `request_id`. Reservoir sampling is uniformly random within a window, which is the right property for forensic review (no rate-throttling bias).

**Where it falls short.** Three places to look instead of one. Reservoir-sample reads are not realtime — they're per-window summaries. Operators must understand which substrate answers which question; we document this. The WAL grows quickly when per-token data lands there; segment rotation cadence must increase (operator-configurable). The reservoir-sampling state per `(tenant, model, route)` tuple is small (`K` events × event size) but the *number of tuples* grows; we bound it with an HLL-tracked dimension allowlist or sketch-based eviction.

**Real-world systems that use it.** This shape — aggregate-via-histogram + sample-via-reservoir + forensic-via-log — is the canonical observability pattern in CockroachDB, Cassandra, and Kafka. Honeycomb's `BubbleUp` uses a related but distinct shape (head-sampled wide events).

### 3.3. Rate-sampled per-token events (1-in-N), no WAL coupling

**What it is.** Per-token events are head-sampled at the source: emit 1 in 100 tokens (or 1 in `N`, operator-configurable). Sampled tokens go to OTel with full attributes; unsampled tokens contribute only to local aggregates. The WAL is independent and continues to record requests at the granularity already shipped in `v0.2`.

**Why it's interesting.** Simple. The 1-in-N pattern is what OTel itself defaults to; it's familiar to every observability operator. No new substrate.

**Where it falls short.** Rate sampling has the well-known forensic bias: rare slow tails are systematically *under*-represented in the sample. The operator who needs to see "show me the slow streams" gets the wrong answer because the slow streams are themselves rare and the sampler discards 99% of them. For inter-token-latency forensics specifically — the precise thing `NFR-OBS04` exists to surface — this is the wrong sampler shape. Honeycomb's tail-sampling and reservoir-sampling literature [3] makes this case directly.

**Real-world systems that use it.** Most OTel deployments default to head-sampling. It's adequate for request-level overview; it's the wrong tool for token-level forensics.

### 3.4. CMS + HLL aggregates, no per-token event stream at all

**What it is.** No per-token OTel events. Instead, per `(tenant, model, route)` keep three structures: an HDR histogram of inter-token latency, an HLL estimating distinct token sequences (proxy for diversity), and a Count–Min Sketch of token-prefix heavy hitters. These structures publish aggregates to OTel/Prometheus on a 10-second cadence. Per-token bytes go to the WAL only.

**Why it's interesting.** Bounded memory regardless of cardinality. HDR for latency, HLL for cardinality, CMS for heavy hitters — each with well-understood error bounds (CMS overcount-only, HLL ±2% at fixed memory). Fits the streaming-algorithms canon directly. The Aggregate-Only-In-Metrics shape composes cleanly with Prometheus's recording-rules model.

**Where it falls short.** Operators lose the ability to inspect *any* individual token event without going to the WAL. "Show me a slow stream" requires WAL access and a replay tool, not an OTel query. Forensic friction goes up. The HLL distinct-token-sequence metric is interesting research but not load-bearing for any SLO; we'd add the structure and never use it.

**Real-world systems that use it.** Datadog's per-host aggregate APIs use CMS-like structures internally for top-K endpoint summaries. Most observability platforms expose this pattern only as a derived metric, not as the *only* metric.

### 3.5. Hybrid — `§3.2` plus `§3.4`'s CMS for heavy-hitters dimension

**What it is.** `§3.2`'s three-layer split (HDR-histogram + reservoir-sample + WAL) for latency forensics, plus `§3.4`'s CMS only for "top-K tenants by token burn this hour" and similar heavy-hitter questions. The CMS is operator-opt-in and lives in the `TokenLevelAggregator`.

**Why it's interesting.** Adds heavy-hitter detection (which `§3.2` alone leaves to OTel collector downstream) without losing forensic-replay or the reservoir's count-bounded sampling property.

**Where it falls short. ** Larger surface area to maintain. Three sketch families (HDR, reservoir, CMS) plus the WAL is four substrates. We pay maintenance cost for every one. CMS is straightforward to add later as a follow-on, so the discipline question is: include it in `v0.4` or defer to `v1.0`?

## 4. Tradeoff matrix

| Property | 3.1 OTel-only | 3.2 Reservoir+HDR+WAL | 3.3 Rate-sample | 3.4 CMS+HLL aggregates | 3.5 Hybrid | Why it matters |
|----------|---------------|------------------------|------------------|------------------------|------------|----------------|
| Bounded cardinality under load | no | yes | no (still hits OTel) | yes | yes | Production gateways see `O(10^8)` tokens/day. |
| Forensic-replay coverage | yes | yes (via WAL) | no | yes (via WAL only) | yes | "Show me this specific slow stream" must be answerable. |
| Rare-tail capture quality | yes | yes (reservoir is uniform) | no (rate-sampler bias) | n/a | yes | Slow streams are exactly the events we need. |
| Number of substrates operator must learn | 1 | 3 | 1 | 1 (+WAL) | 4 | More substrates → more docs, more cognitive load. |
| Realtime aggregate metric availability | per-event | per-cadence (10s) | per-event | per-cadence | per-cadence | Dashboard refresh latency. |
| OTel collector cost | catastrophic | low (bounded reservoir) | medium (1-in-N rate) | very low | low-medium | Bounds the operator's infra spend. |
| WAL write rate | low | high (per-token) | low | high | high | Storage planning; ADR `0013` accounted for some headroom. |
| Memory per process (sketch state) | ~0 | ~MB | ~0 | ~MB | ~MB | Fits comfortably in modern hosts. |
| Heavy-hitter top-K queryability | yes (downstream) | indirectly | yes (downstream) | yes (native) | yes (native) | "Top-K tenants by token burn" is a frequent operator question. |
| Implementation cost in `v0.4` | low (no sketches) | medium | low | medium | high | Milestone scope. |
| Sampling bias for slow streams | unbiased | unbiased | biased toward fast | n/a | unbiased | Critical for tail-latency SLOs. |

## 5. Foundational principles

Two literatures meet here. The streaming-algorithms canon — Cormode and Muthukrishnan's Count–Min Sketch [1], Flajolet et al.'s HyperLogLog [2], Vitter's Reservoir Sampling [3], Misra–Gries summaries [4], the t-digest [5] — gives us substrates whose error bounds are mathematically known and whose memory footprint is independent of cardinality. The observability-platform canon — Gil Tene's HDR histograms [6], Honeycomb's wide-event-plus-tail-sampling shape [7], OTel sampling specifications [8] — gives us the operational patterns and the vocabulary operators already know.

The decision-relevant insight from the streaming-algorithms side is that *the error bound is the contract*. CMS overcounts but never undercounts (so heavy hitters are never missed, only inflated); HLL's standard error is `1.04 / sqrt(m)` for `m` registers (so 12 KB of memory gets ±2% across billions of distinct items); reservoir sampling is *uniform within the window* (so rare slow events are kept proportionally to their occurrence, not their rate). Each of these is a different probabilistic guarantee, and matching the guarantee to the question is the substantive engineering work.

The decision-relevant insight from the observability-platform side is that *latency tails matter more than means*, and the sampling scheme has to preserve them. Head-sampling at a fixed rate discards rare slow events at the same rate as common fast ones; tail-sampling or reservoir-sampling preserve them. Honeycomb's `BubbleUp` literature and the broader "wide events" school [9] make this explicit. Our `NFR-OBS04` ("operators can identify which backend stutters") is fundamentally a tail-sampling problem dressed up as a token-latency problem.

The WAL is the third substrate that earns its place by being *append-only and replay-able*. ARIES-style write-ahead logging [10] proves the pattern at the database level; the file-WAL design in [ADR `0013`](../06-adrs/0013-append-only-file-wal.md) is the same pattern at the gateway level. Per-token data is bulky but compresses well (LLM tokens are typically 2–4 bytes of UTF-8 byte-aligned data after tokenisation); the WAL absorbs this with documented segment-rotation cadence.

The cross-cutting design principle: *match the substrate to the question*. Aggregate latency → HDR histogram, because that's what HDR was built for. Random forensic sample → reservoir, because reservoir is uniform within the window. Per-token bytes for one specific request → WAL, because the WAL is built for replay and the OTel path is built for aggregation. We do not invent a new substrate; we compose four well-understood ones.

## 6. Recommendation

**Adopt `§3.2` — reservoir-sampled per-token events plus full-fidelity WAL — with HDR histograms for aggregate latency metrics. Defer the CMS heavy-hitters extension (`§3.5`) to `v1.0` unless an operator surfaces it as a blocker during `v0.4` close-out.**

Concretely:

- **Aggregate latency metrics** (`riftgate_ttft_seconds`, `riftgate_inter_token_seconds`, `riftgate_token_jitter_seconds`, all already named in the [observability-plane document](../03-architecture/observability-plane.md)) are computed via HDR histograms in `TokenLevelAggregator` and published to OTel/Prometheus via the existing `Histogram` variant of `ObservabilityEvent`. No new bus variant.
- **Per-token forensic samples** use Vitter's Algorithm R reservoir, sized `K = 100` events per `(tenant, model, route)` tuple per 60-second window. Sampled events publish as `SpanStart`/`SpanEnd` with bounded attributes: `request_id`, `token_index`, `latency_since_last_token`, `byte_offset`. The `model` and `tenant` are span-context attributes inherited from the parent request span, not new label dimensions on the metric.
- **Per-token bytes** (full token boundaries, byte offsets, timestamps) go to the WAL. The WAL schema is extended with a `TokenEvent { request_id, token_index, byte_offset_start, byte_offset_end, emit_timestamp_ns }` record type; segment rotation cadence drops to match the higher write rate (operator-configurable; default 256 MiB or 60 s, whichever first).
- **Cardinality control on the dimension allowlist** uses a simple bounded `HashSet<(tenant, model, route)>` with operator-configured capacity (default `10_000`). Beyond the cap, new dimensions fall to a `(other, other, other)` bucket and emit a `riftgate_observability_dimension_capped_total` counter. No HLL needed — the cap-and-fallback pattern is simpler, debugges easier, and matches the existing Prometheus discipline.
- **Sampling policy** for OTel: reservoir for per-token spans (count-bounded, unbiased); the request-root span continues to use OTel's normal head-sampling because that's already shipped and unchanged.
- **`TokenLevelAggregator`** implements `ObservabilitySink`; it does not subscribe to the bus directly. The parser ([`crates/riftgate-parser/src/sse.rs`](../../crates/riftgate-parser/src/sse.rs)) and the IO subsystem coordinate to emit `Emit::Token { boundary, emit_timestamp_ns }` to the aggregator via a per-shard MPSC, mirroring the existing observability bus. The aggregator drains, updates HDR histograms, updates the reservoir, writes WAL records, and publishes aggregate `Histogram` events on its cadence.
- **eBPF coupling** [Options `014`](014-ebpf-integration.md) is optional and orthogonal. BPF can timestamp byte-egress at the syscall boundary with nanosecond accuracy; without BPF, we use the userspace `Instant::now()` at the write call site (sufficient for TTFT under millisecond budgets, less precise for sub-millisecond inter-token latency on very fast streams). The aggregator accepts both sources behind the same trait.

**Conditions to revisit:**

- Operator request surfaces "top-K tenants by token burn" as a frequent dashboard query — promote `§3.5`'s CMS extension.
- The HDR-histogram bucket count proves insufficient (a workload distribution outside HDR's documented range) — adopt t-digest [5] as the histogram substrate.
- The reservoir window of 60 s proves too coarse or too fine — make the window operator-configurable per-tuple.
- The WAL token-record write rate exceeds the disk-IO budget under sustained load — adopt block compression (zstd or LZ4 at segment-write time, traded against replay CPU cost).
- A future ABI extension lets WASM filters [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md) read token boundaries — coordinate the aggregator's emit path with the filter chain's `on_response_chunk` host function.

**Non-default candidates kept available:**

- The CMS heavy-hitters extension (`§3.5`) is documented as the `v1.0` follow-up. No implementation work in `v0.4`.
- The Bloom-filter and semantic-cache structures discussed in [Vision `§8`](../00-vision.md) remain explicitly out of the observability surface; if the semantic-cache filter lands, it owns its own structures.

## 7. What we explicitly reject

- **OTel-only per-token events (`§3.1`).** Cardinality is catastrophic; we keep this option as a stress-test fixture in CI but never as a production default. Revisit only if OTel collector cardinality controls become substantially cheaper.
- **Rate-sampled per-token (`§3.3`)** as the *only* sampler. Fails the rare-slow-tail forensic case, which is the load-bearing case for `NFR-OBS04`. We continue to use head-sampling for the request-root span (already shipped) but reject it for the token sub-spans.
- **CMS + HLL aggregates as the only path (`§3.4`).** Loses individual-event forensic-replay coverage outside the WAL. We borrow CMS as a `§3.5` follow-on, not as a replacement.
- **Per-tenant configurable sampling policies** in `v0.4`. The complexity is real (per-tenant `K`, per-tenant window) and the operator demand is unproven. Defer to `v1.0` alongside multitenancy ([Options `017`](README.md)).

## 8. References

1. Graham Cormode, S. Muthukrishnan. *An improved data stream summary: the count-min sketch and its applications.* J. Algorithms, 2005. <https://sites.cs.ucsb.edu/~suri/cs290/CormodeMuthu.pdf>
2. Philippe Flajolet, Éric Fusy, Olivier Gandouet, Frédéric Meunier. *HyperLogLog: the analysis of a near-optimal cardinality estimation algorithm.* AofA, 2007. <https://algo.inria.fr/flajolet/Publications/FlFuGaMe07.pdf>
3. Jeffrey S. Vitter. *Random Sampling with a Reservoir.* ACM TOMS, 1985. <https://www.cs.umd.edu/~samir/498/vitter.pdf>
4. Jayadev Misra, David Gries. *Finding repeated elements.* Science of Computer Programming, 1982.
5. Ted Dunning. *Computing Extremely Accurate Quantiles Using t-Digests.* <https://github.com/tdunning/t-digest>
6. Gil Tene. *HdrHistogram: A High Dynamic Range Histogram.* <http://hdrhistogram.org/>
7. Charity Majors, Liz Fong-Jones, George Miranda. *Observability Engineering.* O'Reilly, 2022. ISBN 978-1492076445.
8. OpenTelemetry sampling specification. <https://opentelemetry.io/docs/specs/otel/trace/sdk/#sampling>
9. Honeycomb engineering blog. *Sampling at Scale.* <https://www.honeycomb.io/blog/dynamic-sampling-by-example>
10. C. Mohan et al. *ARIES: A transaction recovery method supporting fine-granularity locking and partial rollbacks using write-ahead logging.* ACM TODS, 1992.
11. Graham Cormode, Ke Yi. *Small Summaries for Big Data.* Cambridge University Press, 2020. ISBN 978-1108477444.
12. CLRS, ch. 9 (medians and order statistics) for reservoir-sampling background.
13. OTel profiling and logging signal specifications. <https://opentelemetry.io/docs/specs/otel/>
