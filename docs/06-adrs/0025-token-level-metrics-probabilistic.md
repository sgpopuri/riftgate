# ADR 0025. Token-level metrics via reservoir-sampled OTel spans + HDR-histogram aggregates + per-token WAL records

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [027-token-level-metrics](../05-options/027-token-level-metrics.md)
> **Deciders:** Sriram Popuri

## Context

`v0.4`'s `NFR-OBS04` requires TTFT, inter-token latency, and token-jitter metrics with sufficient fidelity that an SRE can identify a stuttering backend. The naive shape — one OTel event per token with full attributes — saturates any production observability backend within hours. Five shapes were evaluated in [Options `027`](../05-options/027-token-level-metrics.md): all-OTel, reservoir+HDR+WAL, rate-sampled OTel, CMS+HLL aggregates only, and a hybrid. The bedrock insight is that *the substrate must match the question*: aggregate latency wants HDR histograms; rare-tail forensics wants count-bounded uniform sampling (reservoir, not rate); per-token forensics for one specific request wants the WAL.

## Decision

**`v0.4` ships `TokenLevelAggregator` as a new `ObservabilitySink` impl in `crates/riftgate-obs` that composes three substrates: per-`(tenant, model, route)` HDR histograms for aggregate latency, Vitter's Algorithm R reservoir for bounded random per-token spans, and per-token WAL records for forensic replay. CMS heavy-hitters and all-OTel per-token events are explicitly rejected for `v0.4`.**

- Aggregate metrics (`riftgate_ttft_seconds`, `riftgate_inter_token_seconds`, `riftgate_token_jitter_seconds`) are computed via HDR histograms (Gil Tene's `HdrHistogram` library, with a documented bounded range) per `(tenant, model, route)` tuple and published as `ObservabilityEvent::Histogram` on a 10-second cadence. No new bus variant; the existing trait from [ADR `0011`](0011-otel-default-sink-multisink-fanout.md) suffices.
- Per-token forensic samples use Vitter's Algorithm R reservoir, default `K = 100` events per tuple per 60-second window. Sampled events publish as `ObservabilityEvent::SpanStart` / `SpanEnd` with bounded attributes only: `request_id`, `token_index`, `latency_since_last_token_ns`, `byte_offset`. `model` and `tenant` are inherited from the parent span context; they are *not* metric labels.
- Per-token byte boundaries and timestamps go to the WAL via a new `TokenEvent { request_id, token_index, byte_offset_start, byte_offset_end, emit_timestamp_ns }` record type extending the [ADR `0013`](0013-append-only-file-wal.md) schema. Segment-rotation cadence reduces to 256 MiB or 60 s (operator-configurable) to absorb the higher write rate.
- Dimension-cardinality control uses a bounded `HashSet<(tenant, model, route)>` (default capacity `10_000`); new dimensions beyond the cap fall into an `(other, other, other)` bucket and emit `riftgate_observability_dimension_capped_total`. No HLL — the cap-and-fallback pattern is simpler and easier to debug.
- `TokenLevelAggregator` does not subscribe to the existing observability bus directly. The parser ([`crates/riftgate-parser/src/sse.rs`](../../crates/riftgate-parser/src/sse.rs)) and the IO subsystem coordinate to publish `Emit::Token { boundary, emit_timestamp_ns }` to the aggregator via a per-shard MPSC, mirroring the existing publisher pattern. The aggregator updates HDR histograms, updates the reservoir, writes WAL records, and publishes aggregate `Histogram` events on its cadence.
- Timestamp source: userspace `Instant::now()` at the write call site by default. If [ADR `0024`](0024-ebpf-via-aya.md)'s BPF programs are enabled and the `bpf-token-timestamps` feature is on, the aggregator accepts BPF-sourced byte-egress timestamps instead (nanosecond accuracy at the syscall boundary) behind the same trait.

## Consequences

- **Positive:**
  - Bounded cardinality regardless of token volume. The HDR-histogram count and the reservoir size are both functions of `(tenant, model, route)` cardinality, which the dimension allowlist bounds explicitly.
  - Rare slow streams are sampled in proportion to their occurrence, not their rate. Vitter's Algorithm R is uniformly random within the window. The `NFR-OBS04` "identify the stuttering backend" workflow is no longer biased toward fast streams.
  - Per-token forensic replay remains possible without OTel-collector saturation. `riftgate-replay eval` ([ADR `0021`](0021-external-replay-cli.md)) can read the new WAL `TokenEvent` records and re-derive any per-token statistic.
  - Substrates match questions: HDR for "what's the typical inter-token latency," reservoir for "show me 100 random slow streams," WAL for "exactly what happened on request X."
- **Negative / accepted tradeoffs:**
  - Three substrates, three places to look. Operators must understand which substrate answers which question; we document this in the [observability LLD](../04-design/lld-observability.md).
  - Aggregate metrics are per-cadence (10 s), not per-event. Dashboard refresh latency is at minimum 10 s. Acceptable for the SLO-grade workflow this targets.
  - WAL write rate increases proportionally with per-token data; segment-rotation cadence drops. Storage planning needs to account for this.
  - The HDR-histogram bounded range (typically 1 µs to 1 hour) covers every realistic token-latency scenario but does *fail loudly* on out-of-range values. We document the range and trust the library's overflow telemetry.
- **Future work this enables:**
  - CMS heavy-hitters extension (Options `027` §3.5) as a `v1.0+` follow-on if "top-K tenants by token burn this hour" becomes a frequent operator query.
  - Token-level metrics that integrate with the WASM filter chain's future `on_response_chunk` host function ([ADR `0019`](0019-wasm-extension-mechanism.md))'s v2 ABI.
  - BPF-sourced byte-egress timestamps via [ADR `0024`](0024-ebpf-via-aya.md) for sub-millisecond inter-token-latency precision on very fast streams.
  - Per-tenant configurable sampling policies once [Options `017`](../05-options/README.md) multitenancy lands.
- **Future work this forecloses (until superseded):**
  - Riftgate will not emit one OTel event per token in `v0.4`. The all-OTel candidate is documented and rejected.
  - Riftgate will not use rate-sampling (1-in-N) for token sub-spans. The request-root span continues to use OTel's normal head-sampling, but token sub-spans go through the reservoir.
  - Riftgate will not invent its own quantile-estimation library; HDR histograms are the substrate. If a workload distribution exceeds HDR's documented range, we adopt t-digest (Cormode-Dunning) per the Options doc revisit clause.

## Compliance

- `crates/riftgate-obs/src/token_level/` houses the aggregator, the per-tuple HDR histograms, and the reservoir.
- `crates/riftgate-obs/tests/token_level_reservoir.rs` asserts that the reservoir is uniformly random within the window across a synthetic stream of 10⁶ tokens (chi-square goodness-of-fit test, well within tolerance).
- `crates/riftgate-obs/tests/token_level_cardinality.rs` asserts that the dimension cap is enforced and that overflow buckets emit the documented `riftgate_observability_dimension_capped_total` counter.
- `crates/riftgate-replay/tests/eval_token_events.rs` asserts that `riftgate-replay eval` correctly reads the new `TokenEvent` WAL records and reproduces the per-token statistics.
- A criterion bench at `crates/riftgate-obs/benches/token_level_dispatch.rs` measures aggregator hot-path cost per token; the budget is `< 1 µs per token` for HDR update + reservoir update on the userspace timestamp path. CI fails above the budget.
- Changing the reservoir size `K`, the window duration, the dimension cap, or the HDR-histogram bounds via TOML does **not** require a new ADR. Changing the substrate (HDR → t-digest, reservoir → some other sampler) **does**.

## Notes

- The decision to keep the dimension cap as a `HashSet` rather than an HLL is deliberate: HLL gives us *estimated* cardinality with bounded memory, but the question we're answering is "*should this dimension be tracked individually or fall into other?*" — that's a discrete membership question, not a cardinality-estimation question. A plain bounded `HashSet` is the right tool.
- The 60-second reservoir window default balances responsiveness (operators see new slow-stream samples within a minute) against storage cost (`K=100` samples per tuple × number of tuples × samples kept across windows). Operator-configurable.
- The 10-second aggregate-publish cadence matches Prometheus's typical scrape interval. Operators with sub-second scrape budgets can drop the cadence; HDR-histogram update is `O(1)` and the merge cost is small.
- The reservoir publishes through OTel as spans because that's the right shape for forensic forensics ("show me 100 traces of slow streams"). It could equally publish through OTel as `LogRecord` events; we prefer spans because they join naturally to the parent request span via the existing `SpanContext`.
- We deliberately reject any *new* `ObservabilityEvent` variant for token-level data. The existing `SpanStart`, `SpanEnd`, `Counter`, `Histogram` cover every shape we need; adding a `Token` variant would require superseding [ADR `0011`](0011-otel-default-sink-multisink-fanout.md) and we have not seen a need.
