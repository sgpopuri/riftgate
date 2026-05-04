# 04.h LLD — Observability

> OTel traces, Prometheus metrics, eBPF profiles, and the token-level SLO aggregator. The observability plane in detail.
>
> Status: **outline-stage**. Filled out as `v0.2` (OTel + Prom) and `v0.4` (eBPF + token SLOs) land.

## Purpose

Surface what is happening inside the gateway and at its backends, in enough detail that an SRE on call at 3am can answer "why is P99 high?" without guessing. Avoid coupling the data plane to the observability plane.

## Trait surface

```rust
// Sketch
pub enum ObservabilityEvent {
    SpanStart { request_id: RequestId, name: &'static str, attributes: Attributes },
    SpanEnd { request_id: RequestId, name: &'static str, duration: Duration },
    Counter { name: &'static str, value: u64, labels: Labels },
    Histogram { name: &'static str, value: f64, labels: Labels },
    Profile { kind: ProfileKind, samples: Vec<ProfileSample> },     // v0.4+
}

pub trait ObservabilitySink: Send + Sync {
    fn publish(&self, event: ObservabilityEvent);
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `OtelSink` | `v0.1` | `riftgate-obs` | OTLP/gRPC export. |
| `PrometheusSink` | `v0.2` | `riftgate-obs` | `/metrics` HTTP endpoint. |
| `BpfSink` | `v0.4` | `riftgate-obs` | Aya-based BPF programs publish into the same channel. |
| `TokenLevelAggregator` | `v0.4` | `riftgate-obs` | TTFT, inter-token latency, jitter histograms. |
| `MultiSink` | `v0.1` | `riftgate-obs` | Fan-out to multiple sinks. |

Decision rationale: [Options 013 (observability sink)](../05-options/013-observability-sink.md), [Options 014 (eBPF integration)](../05-options/014-ebpf-integration.md).

Source-systems chapter: `Ch16 (eBPF and kernel programmability)`.

## Component context

### Architecture and dependencies

The data plane publishes `ObservabilityEvent` values to a bounded MPSC channel. A dedicated observability worker (or per-sink workers) consume from the channel and translate into the sink-specific format. **The data plane never blocks on the observability plane.**

The eBPF sink is the inverse direction: BPF programs (running in the kernel) publish into a perf ring or BPF ring buffer that a userland thread reads and converts to `ObservabilityEvent`. This event then flows through the same channel as data-plane events.

### Patterns and conventions

- **Drop on full.** A counter (`riftgate_observability_dropped_total`) tracks drops. We do not retry or buffer.
- **Sampling at the source.** Per-token spans are sampled (1 in 100 by default); full per-token data is only in the WAL.
- **Schema stability.** Trace span names and metric names are part of the public API. Renaming requires a deprecation cycle.
- **Cardinality discipline.** No metric label is allowed to take unbounded values. `backend` is bounded by config; `model` is bounded by the registry.

### Pitfalls

- **OTel SDK overhead.** The Rust OTel SDK has historically been the bottleneck in observability-heavy workloads. We benchmark and monitor.
- **High-cardinality metric labels** (e.g. `request_id` as a label) are a fast path to a melted Prometheus. The `Labels` API guards against this.
- **eBPF verifier rejections.** Aya programs can grow complex enough to fail the kernel verifier. We test against multiple kernel versions.
- **Profiling overhead.** Even sampled BPF profiling costs a few percent CPU. We document the cost and let users opt out.

### Standards and review gates

- New trace span names require a glossary entry.
- New metrics require a dashboard query example.
- eBPF program changes require verifier-acceptance tests on Linux 5.15+ and 6.1+ at minimum.

## Testing strategy

- Channel saturation tests — verify drops are counted and the data plane is unaffected.
- OTel collector smoke test — round-trip a trace and verify it lands.
- eBPF test harness using `bpftrace` and a tiny C harness as oracle.
- Long-running soak — verify no sink leak under steady load.

## Open questions

- Should we support per-tenant observability scoping? Recommend yes for `v1.0` via a `tenant` label and label-based filtering at the sink.
- Should we publish raw token streams to OTel as events? Recommend no — too expensive. WAL is the right place for per-token data.
- How do we handle eBPF programs that need to evolve as kernels evolve? CO-RE (Compile Once Run Everywhere) for portability; track Aya releases closely.

## Probabilistic structures for token-level metrics (`v0.4`)

When `v0.4` lands token-level SLOs ([`NFR-OBS04`](../01-requirements/non-functional.md)), the aggregator faces the cardinality problem directly: a production gateway sees millions of unique `(tenant, model, route)` combinations and hundreds of millions of tokens per day. We cannot keep exact per-group counts on the hot path; we use approximate data structures that give us bounded memory and bounded error.

References: `advanced/ch09 (streaming and randomized algorithms)`, `systems/ch10 (data-intensive algorithms)`.

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
