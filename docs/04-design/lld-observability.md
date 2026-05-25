# 04.h LLD — Observability

> OTel traces, Prometheus metrics, eBPF profiles, and the token-level SLO aggregator. The observability plane in detail.
>
> Status: **shipped (v0.1, OTel + JSON over a bounded MPSC bus); v0.2 adds Prometheus; v0.4 adds `BpfSink`, `TokenLevelAggregator`, and the `GpuPressureSource` trait behind the same observability trait surface.** v0.4 decisions in [Options `014`](../05-options/014-ebpf-integration.md), [Options `027`](../05-options/027-token-level-metrics.md), [Options `028`](../05-options/028-gpu-pressure-correlation.md); ADRs [`0024`](../06-adrs/0024-ebpf-via-aya.md), [`0025`](../06-adrs/0025-token-level-metrics-probabilistic.md), [`0026`](../06-adrs/0026-gpu-pressure-via-dcgm-exporter.md).

## Purpose

Surface what is happening inside the gateway and at its backends, in enough detail that an SRE on call at 3am can answer "why is P99 high?" without guessing. Avoid coupling the data plane to the observability plane.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/observability.rs`](../../crates/riftgate-core/src/observability.rs):

```rust
pub enum ObservabilityEvent {
    SpanStart { request_id: RequestId, name: &'static str, attributes: Attributes },
    SpanEnd   { request_id: RequestId, name: &'static str, duration: Duration },
    Counter   { name: &'static str, value: u64, labels: Labels },
    Histogram { name: &'static str, value: f64, labels: Labels },
}

pub trait ObservabilitySink: Send + Sync {
    fn publish(&self, event: ObservabilityEvent);
}
```

The bus is a bounded `tokio::sync::mpsc` channel inside `crates/riftgate-obs/src/bus.rs`; the data plane only ever calls `Publisher::publish`, which is a non-blocking `try_send`. A drop is a published metric, not a stall: the `riftgate_observability_dropped_total` counter records every drop. See `crates/riftgate-obs/src/spans.rs` for the canonical span-name registry (`request.received`, `request.routed`, `request.first_token`, `request.completed`, etc.).

The `Profile` variant in the v0.0 sketch was removed from the trait; eBPF profiles in v0.4 will publish via a separate sink type to keep the v0.1 enum stable.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `OtelSink` | shipped (v0.1) | `riftgate-obs` | OTLP/gRPC export via `tonic`. Translates `SpanStart` / `SpanEnd` into OpenTelemetry spans; counters and histograms are recorded on the matching meter. |
| `JsonStdoutSink` | shipped (v0.1) | `riftgate-obs` | Structured-JSON-per-event sink for local dev and CI logs. One JSON object per line on stdout. |
| `MultiSink` | shipped (v0.1) | `riftgate-obs` | Fan-out wrapper: `MultiSink::new(vec![otel, json])` publishes to both. |
| `InMemorySink` | shipped (v0.1) | `riftgate-core` | Test-only sink that buffers events in a `Mutex<Vec<...>>` for assertions. |
| `PrometheusSink` | v0.2 | `riftgate-obs` | `/metrics` HTTP endpoint. |
| `BpfSink` | **v0.4** | `riftgate-obs` | Aya-based pure-Rust BPF programs (CPU on/off-time profiling, syscall stalls, TCP retransmits per upstream) loaded behind `cfg(all(target_os = "linux", feature = "bpf"))` and runtime-gated by `RIFTGATE_ENABLE_BPF=1`. Programs live in sibling crate `crates/riftgate-obs-bpf` compiled to `bpfel-unknown-none`. Per [ADR `0024`](../06-adrs/0024-ebpf-via-aya.md). |
| `TokenLevelAggregator` | **v0.4** | `riftgate-obs` | Three-substrate aggregator: per-`(tenant, model, route)` HDR histograms for aggregate TTFT / inter-token / jitter latency, Vitter Algorithm R reservoir (default `K=100`, 60 s window) for bounded random per-token spans, and per-token WAL `TokenEvent` records for forensic replay. Bounded dimension cap (default 10 000) with `(other, other, other)` fallback bucket. Per [ADR `0025`](../06-adrs/0025-token-level-metrics-probabilistic.md). |
| `DcgmScrapeSource` | **v0.4** | `riftgate-obs` | Default `GpuPressureSource` impl; HTTP-scrapes NVIDIA `dcgm-exporter` per backend at operator-configured cadence (default 5 s); publishes `Histogram` events for utilization and memory and updates the `BackendSignals::gpu_pressure` field via `ArcSwap<GpuPressure>`. Per [ADR `0026`](../06-adrs/0026-gpu-pressure-via-dcgm-exporter.md). |
| `NvmlSource` | **v0.4** | `riftgate-obs` | Feature-gated `gpu-nvml` alternate `GpuPressureSource` impl; in-process FFI via `nvml-wrapper`; for GPU-co-located Riftgate deployments. Empty-lib elsewhere; macOS and non-NVIDIA Linux compile cleanly. |
| `NoopGpuSource` | **v0.4** | `riftgate-core` | Default when no `gpu_pressure_source` block is configured for a backend. Returns `None`. |

Decision rationale: [Options 013 (observability sink)](../05-options/013-observability-sink.md), [Options 014 (eBPF integration)](../05-options/014-ebpf-integration.md), [Options 027 (token-level metrics)](../05-options/027-token-level-metrics.md), [Options 028 (GPU pressure correlation)](../05-options/028-gpu-pressure-correlation.md).

Foundational principle: eBPF (verifier, JIT, maps, kprobes / tracepoints / XDP / TC / LSM attachment points). Canonical references: kernel.org BPF documentation, Brendan Gregg's *BPF Performance Tools*, the Aya book.

## Component context

### Architecture and dependencies

The data plane publishes `ObservabilityEvent` values to a bounded `tokio::sync::mpsc` channel via the `Publisher` handle. A single observability worker drains the channel and forwards events into a `MultiSink` that fans out to every configured sink (OTel, JSON-stdout, future Prometheus). **The data plane never blocks on the observability plane.**

```text
   data plane  ----publish (try_send)---->  bounded MPSC bus  ----worker---->  MultiSink ---->  OtelSink
                                                                                          \--->  JsonStdoutSink
```

When the channel is full, `try_send` returns immediately, the event is dropped, and the `riftgate_observability_dropped_total` counter is incremented. This is intentional and documented as part of the contract.

The eBPF sink (v0.4) is the inverse direction: BPF programs running in the kernel publish into a perf ring or BPF ring buffer; a userland thread reads them and converts to `ObservabilityEvent`. The events then flow through the same bounded MPSC bus.

### Patterns and conventions

- **Drop on full.** A counter (`riftgate_observability_dropped_total`) tracks drops. The bus does not retry, does not buffer, does not block. This is the v0.1 contract per [Options 013](../05-options/013-observability-sink.md).
- **Canonical span names.** Every span name lives in `crates/riftgate-obs/src/spans.rs` as a `&'static str` constant. Adding a span requires adding it there; the registry is the schema.
- **Sampling at the source** (v0.4+). Per-token spans will be sampled (1 in 100 by default); full per-token data goes to the WAL, not OTel.
- **Schema stability.** Trace span names and metric names are part of the public API. Renaming requires a deprecation cycle and a new ADR.
- **Cardinality discipline.** No metric label is allowed to take unbounded values. `backend` is bounded by config; `model` is bounded by the registry. `request_id` is a span attribute, never a metric label.
- **`MultiSink` is the composition primitive.** Configuring observability means constructing the sink graph at startup; sinks themselves do not know about each other.

### Pitfalls

- **OTel SDK overhead.** The Rust OTel SDK has historically been the bottleneck in observability-heavy workloads. The `request.completed` span is fired from `PinnedDrop` on the streamed-response body to ensure it lands even if the body is dropped early — the cost of getting this wrong is a leaked span, not a leaked request.
- **High-cardinality metric labels** (e.g. `request_id` as a label) are a fast path to a melted Prometheus. The `Labels` API will guard against this when `PrometheusSink` lands; for v0.1 the convention is enforced by code review.
- **`Publisher::publish` must be cheap.** It is called from the request hot path; the implementation is `try_send` plus a counter increment, no allocation outside the event itself.
- **Drop counter underreports under contention.** The atomic increment is `Relaxed`; drops are measured per-shard and aggregated by the OTel exporter. Reading the counter mid-aggregation can underreport by a small bounded amount.
- **eBPF verifier rejections** (v0.4). Aya programs can grow complex enough to fail the kernel verifier. We test against multiple kernel versions (5.15 LTS and 6.1 LTS via a containerized harness in `crates/riftgate-obs/tests/bpf_verifier.rs`); CI gates on verifier-acceptance. Per [ADR `0024`](../06-adrs/0024-ebpf-via-aya.md).
- **Profiling overhead** (v0.4). Even sampled BPF profiling costs a few percent CPU. We document the cost in [`RUNBOOK.md`](../../RUNBOOK.md), default the sampling rate to 19 Hz (matching Linux `perf`), and let operators opt out via `RIFTGATE_ENABLE_BPF=0` (the default).
- **GPU-pressure staleness** (v0.4). `DcgmScrapeSource` reads on a 5 s cadence by default; routers see GPU pressure with documented bounded staleness. This is intentional — staleness is the cost of decoupling the routing hot path from GPU telemetry I/O. Per [ADR `0026`](../06-adrs/0026-gpu-pressure-via-dcgm-exporter.md).
- **Token-level dimension cap** (v0.4). The bounded `HashSet<(tenant, model, route)>` (default 10 000 capacity) prevents cardinality explosion in `TokenLevelAggregator`; overflow falls to `(other, other, other)` and emits `riftgate_observability_dimension_capped_total`. Operators with very high tenant cardinality raise the cap with documented memory cost.

### Standards and review gates

- New trace span names require an entry in `crates/riftgate-obs/src/spans.rs` and a corresponding glossary line in `docs/08-glossary.md`.
- New metrics require a dashboard query example.
- The trait surface is part of the v0.1 frozen surface — changes require a new ADR superseding [ADR 0011](../06-adrs/0011-mpsc-bus-with-otel-sink.md).
- eBPF program changes (v0.4) require verifier-acceptance tests on Linux 5.15+ and 6.1+ at minimum.

## Testing strategy

- Channel saturation tests — verify drops are counted and the data plane is unaffected.
- OTel collector smoke test — round-trip a trace and verify it lands.
- eBPF test harness using `bpftrace` and a tiny C harness as oracle.
- Long-running soak — verify no sink leak under steady load.

## Open questions

- Should we support per-tenant observability scoping? Recommend yes for `v1.0` via a `tenant` label and label-based filtering at the sink. Intersects with multitenancy ([Options `017`](../05-options/README.md)).
- Should we publish raw token streams to OTel as events? **Resolved (v0.4):** no — the WAL is the right place for full per-token data; OTel receives reservoir-sampled token sub-spans only. Per [ADR `0025`](../06-adrs/0025-token-level-metrics-probabilistic.md).
- How do we handle eBPF programs that need to evolve as kernels evolve? CO-RE (Compile Once Run Everywhere) for portability; track Aya releases closely. v0.4 floor is Linux 5.15 LTS per [ADR `0024`](../06-adrs/0024-ebpf-via-aya.md).
- Multi-vendor GPU telemetry (AMD ROCm, Habana, Inferentia). `v0.4` supports AMD via Prometheus scrape against ROCm SMI exporter (`DcgmScrapeSource` works against any compatible Prometheus endpoint). Native ROCm SMI / Habana / Inferentia impls become clean future additions behind the `GpuPressureSource` trait. The long-term option is the deferred sidecar per [Options `028` §3.3](../05-options/028-gpu-pressure-correlation.md).
- BPF-sourced byte-egress timestamps for sub-millisecond inter-token-latency precision. `v0.4` ships userspace `Instant::now()` as the default timestamp source; `TokenLevelAggregator` accepts BPF-sourced timestamps behind the `bpf-token-timestamps` feature when [ADR `0024`](../06-adrs/0024-ebpf-via-aya.md)'s BPF programs are enabled.
- CMS heavy-hitters extension (Options `027` §3.5) for "top-K tenants by token burn this hour" deferred to `v1.0` unless operator demand surfaces during `v0.4` close-out.

## Probabilistic structures for token-level metrics (`v0.4`)

When `v0.4` lands token-level SLOs ([`NFR-OBS04`](../01-requirements/non-functional.md)), the aggregator faces the cardinality problem directly: a production gateway sees millions of unique `(tenant, model, route)` combinations and hundreds of millions of tokens per day. We cannot keep exact per-group counts on the hot path; we use approximate data structures that give us bounded memory and bounded error.

References: streaming and randomized algorithms (Cormode–Muthukrishnan Count–Min Sketch, Flajolet et al. HyperLogLog, Vitter's reservoir sampling); data-intensive algorithms for cardinality and heavy-hitter approximation under bounded memory (CLRS ch. 9; Cormode and Yi, *Small Summaries for Big Data*, Cambridge 2020).

### HyperLogLog (HLL) for cardinality

**Used for:** estimating the number of unique values across a dimension — e.g. "how many distinct prompt hashes did tenant X send this hour," "how many unique tool names did an agent call this session."

**Why HLL here:**
- Fixed ~12 KB of memory gives ±2% accuracy across billions of distinct items.
- Mergeable: per-shard HLLs combine without loss.
- O(1) update per observation.

**Where it lives:** the `TokenLevelAggregator` keeps one HLL per metric-dimension tuple we care about. Merges happen asynchronously on a cadence; the hot path only updates the local HLL.

### Count-Min Sketch (CMS) for heavy hitters

**Used for:** approximate per-value counts where exact counts are prohibitive — e.g. "top-K tenants by token burn this hour," "prompt-hashes that dominate cache-miss volume."

**Why CMS here:**
- Fixed memory (a `d × w` matrix; typical `d=4, w=2^15` uses ~512 KB) for any cardinality.
- Overcount-only error: we never say a value has fewer occurrences than reality.
- Mergeable across shards (same as HLL).
- Pairs with a small min-heap to track the top-K without keeping all counts.

**Where it lives:** the `TokenLevelAggregator` and — optionally at `v0.4+` — the eBPF-sink correlator when we want to attribute hot-syscall signatures to a small set of callers without paying exact-count cost.

### Reservoir sampling for random traces

**Used for:** keeping a bounded, uniform random sample of traces across a metric dimension — e.g. "pick 1000 random slow traces from the last hour for forensic review."

**Why it's relevant:** the existing OTel sampling is a *rate*-based sampler (keep 1 in N). A reservoir sample is a *count*-bounded sampler (keep K uniformly). The two serve different purposes; the existing trace path uses the former, forensic workflows want the latter.

### What we do NOT use here

- **Bloom filters** in the hot observability path. A Bloom filter answers "have I seen this exact value before," which is not a question observability needs to answer on the hot path. The semantic-cache filter (deferred; see [Vision §8](../../00-vision.md)) would use one as a pre-check, but observability does not.
- **T-Digest / Q-Digest** for quantile estimation. Useful; we prefer the HDR-histogram approach for our p99 / p99.9 latency metrics because the distributions we care about are bounded-range. Revisit if we ever track an unbounded-range metric.
