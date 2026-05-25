# 013. Observability sink

> **Status:** `recommended` — `OtelSink` (OTLP/gRPC export) plus `MultiSink` (fan-out) over a bounded MPSC channel with drop-on-full and a `riftgate_observability_dropped_total` counter, in `v0.1`. `PrometheusSink` lands in `v0.2`; `BpfSink` and `TokenLevelAggregator` land in `v0.4`. See [ADR `0011`](../06-adrs/0011-otel-default-sink-multisink-fanout.md).
> **Foundational topics:** OpenTelemetry (OTel) traces and OTLP transport, Prometheus exposition format, bounded ring-buffer drop-on-full (LMAX Disruptor lineage), backpressure as policy, sampling-at-the-source vs sampling-at-the-sink, cardinality discipline in metric labels
> **Related options:** [`014`](README.md) (eBPF integration — `BpfSink` is a peer impl that consumes from the same bus), [`029`](README.md) (async telemetry pipeline — optional, deepens this doc; deferred), [`011`](011-circuit-breaker.md) and [`012`](012-backpressure.md) (the same drop-on-full discipline applies to the request-side queue)
> **Related ADR:** [ADR `0011`](../06-adrs/0011-otel-default-sink-multisink-fanout.md)

## 1. The decision in one sentence

> What shape does the observability output surface take in `v0.1` — a single canonical exporter, a multi-sink fan-out, or a custom protocol — and how is the data plane prevented from blocking on it?

## 2. Context — what forces this decision

The observability plane has a load-bearing property the rest of the architecture inherits: **the data plane never blocks on the observability plane** ([`docs/03-architecture/hld.md` §6](../03-architecture/hld.md), [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md) Architecture). Drop on full, count the drop, do not retry, do not buffer indefinitely. The shape of the output sink is the second-order question; the bus-level discipline is non-negotiable and is the lens we use to evaluate every candidate below.

What `v0.1` actually needs is narrow:

- [`FR-006`](../01-requirements/functional.md) — emit OTel traces for each request with the canonical span sequence (`received`, `queued`, `dispatched`, `first_token`, `completed`); verifiable against a local OTel collector.
- [`NFR-OBS01`](../01-requirements/non-functional.md) — OpenTelemetry traces for every request with span names that match FR-006, exporter configurable per common backends.
- [`NFR-OBS03`](../01-requirements/non-functional.md) — structured logs (JSON, consistent field schema, configurable level).

Everything else — Prometheus metrics, eBPF profiles, token-level SLOs — is `v0.2+` work and is in scope here only insofar as the trait and bus shape we pick today must accommodate them tomorrow without a breaking change. The LLD's [Implementations table](../04-design/lld-observability.md) names them: `PrometheusSink` in `v0.2`, `BpfSink` and `TokenLevelAggregator` in `v0.4`. We commit to the bus and trait shape that lets each of these land as a peer `ObservabilitySink` impl, not as a special case.

A second forcing function: **cardinality discipline**. A metric label that takes unbounded values (e.g. `request_id`) is a fast path to a melted Prometheus or a blown OTel collector budget. The shape of the trait — particularly the `Labels` type — is where we encode this; we'd rather make a high-cardinality label *impossible to construct* than rely on review to catch it after the fact.

A third forcing function: **vendor coupling.** OTel is the right ecosystem bet (OTLP is the de-facto standard, every observability vendor speaks it), but committing to OTel's specific API at every call site would mean a future "we want to swap the SDK" requires touching the data plane. The trait surface is what insulates the data plane from the SDK choice.

## 3. Candidates

We evaluate five candidates spanning "ship one exporter and call it done" to "build our own protocol."

### 3.1. OTel-only sink (no fan-out)

**What it is.** The data plane publishes events directly into the OpenTelemetry SDK (via the `opentelemetry` and `opentelemetry-otlp` crates). A single exporter sends OTLP/gRPC to whatever collector the operator has configured (Tempo, Jaeger, vendor-hosted). No `ObservabilitySink` trait, no bus, no fan-out — every event is an OTel API call from the call site.

**Why it's interesting.**
- Smallest surface area. One dependency, one exporter, one config block.
- The most idiomatic shape for OTel-native code; matches what Tokio's instrumentation stack does by default.
- Zero engineering cost beyond wiring the SDK at startup.

**Where it falls short.**
- **No backpressure decoupling.** The OTel SDK's exporter has its own queue and its own backpressure semantics. If the collector goes down or slows, the OTel SDK can block (or drop, depending on configuration) — and the data-plane invariant ("never blocks on observability") becomes a property of the OTel SDK's tuning, not a property we control.
- **Couples the data plane to OTel.** Every span emission is `opentelemetry::trace::Tracer::start(...)` somewhere. A future "we also want a Prometheus sink" means either (a) call both APIs from the call site, or (b) refactor to introduce a trait — which is what we should have done in the first place.
- **No first-class drop counter.** OTel's exporter has internal drop metrics, but they are exporter-specific and not part of our public observability schema.
- **Cardinality discipline becomes per-call-site review.** Without a `Labels` type that constrains label keys, every `set_attribute` call is a place a high-cardinality label could land.

**Real-world systems that use it.** Many small services that have only one observability backend. Not the common shape for a kernel that intends to add more sinks over time.

### 3.2. OTel + Prometheus, both as direct call-site emitters

**What it is.** Same as 3.1, but with a parallel Prometheus exposition layer. Every metric-emitting site calls both the OTel API and the Prometheus client API. A `/metrics` HTTP handler serves the Prometheus output; OTLP/gRPC continues to handle traces.

**Why it's interesting.**
- Covers two of the three concrete sinks the roadmap names ([NFR-OBS01](../01-requirements/non-functional.md), [NFR-OBS02](../01-requirements/non-functional.md)).
- Each ecosystem's tooling is mature; `prometheus` and `opentelemetry-otlp` crates are both well-maintained.

**Where it falls short.**
- **Duplicate emission code.** Every metric site has two API calls; every span site has one. Asymmetric and accident-prone.
- **Same backpressure problem as 3.1, doubled.** Two sinks, each with their own backpressure semantics, both directly on the data path.
- **Same coupling problem as 3.1, doubled.**
- **eBPF events have no obvious home.** When `BpfSink` lands in `v0.4`, it doesn't fit either the OTel or the Prometheus call shape — so we end up introducing the trait and the bus *anyway*, retroactively, after the call sites have already been written.

**Real-world systems that use it.** Many production services in the early-2020s before the OTel collector matured. The pattern works but is widely understood as a transitional shape, not a target shape.

### 3.3. Trait-based `ObservabilitySink` + bounded MPSC bus + `MultiSink` fan-out (recommended)

**What it is.** The shape from [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md). The data plane publishes typed `ObservabilityEvent` values to a single bounded MPSC channel. Per-sink workers consume from the channel. `MultiSink` is itself an `ObservabilitySink` that fans out to N inner sinks. The bus has a fixed capacity; events that arrive when the bus is full are dropped and counted (`riftgate_observability_dropped_total`). Sampling decisions happen at the source (per-token spans are sampled 1-in-100 by default; full per-token data is only in the WAL).

In `v0.1`: the `ObservabilitySink` trait, an `OtelSink` impl over OTLP/gRPC, a `MultiSink` aggregator, the bounded-MPSC bus, the dropped counter, and the canonical span-name registry. `PrometheusSink` lands in `v0.2`; `BpfSink` and `TokenLevelAggregator` land in `v0.4`. All future sinks are peer impls of `ObservabilitySink`; none of them touch the data-plane call sites.

**Why it's interesting.**
- **The data-plane invariant is a property of the bus, not of any sink.** Drop-on-full is enforced at `publish` time, before any sink sees the event. No matter how badly an exporter behaves, the data plane never waits on it.
- **One pluggability seam.** Every future sink — Prometheus, eBPF, custom binary, log-based — is an `ObservabilitySink` impl behind the same trait. The data plane doesn't change shape when a new sink lands.
- **Cardinality discipline lives in the type system.** The `Labels` type can constrain which keys are allowed (the `backend` label is a bounded enum from config; the `model` label is from the registry; `request_id` is *not constructible* as a label).
- **Sampling-at-the-source is a first-class concept.** The bus carries already-sampled events; no SDK-internal sampling that we have to reverse-engineer.
- **Span names are part of the public API** and live in a single module (`crates/riftgate-obs::spans`). Renaming a span is a deprecation cycle, not a one-off edit.
- **Drop-on-full is the LMAX Disruptor pattern**, well-understood, observable, and matches the same backpressure-as-policy posture we use on the request-side queue ([Options `004`](004-request-queue.md), [Options `012`](README.md)).

**Where it falls short.**
- **More code than 3.1 or 3.2.** A trait, a bus, a fan-out wrapper, a span-name registry, a drop counter, sink workers, conformance tests. Bounded engineering cost (~500 lines + tests) but real.
- **Two-hop emission.** A span is enqueued onto the bus, then dequeued by the sink worker, then sent to OTel. Adds ~1–10 µs of latency to span emission (well below any user-visible budget) and one extra allocation per event (mitigated by per-event arena reuse where possible).
- **The bus is a synchronization point.** A bounded MPSC under high publish load is a contention candidate. We pick a per-shard MPSC variant where possible; the LLD's "Architecture" section documents the cross-shard fan-in.

**Real-world systems that use it.** Envoy's stats subsystem (a bounded queue feeding multiple sink configurations); Cassandra's metrics+tracing wiring (one bus, multiple reporters); the LMAX Disruptor (the canonical bounded-ring-buffer-with-drop-on-full design); Tokio's `tracing` crate pairs with subscribers in the same shape.

### 3.4. Custom binary protocol over Unix domain socket

**What it is.** Define a Riftgate-specific wire format for observability events; emit it over a Unix domain socket to a sidecar process; let the sidecar translate to OTel/Prometheus/whatever. Modeled loosely on systemd's journald socket protocol.

**Why it's interesting.**
- Cleanest separation between the data plane and any vendor SDK — the data plane only knows how to write our wire format.
- Enables out-of-process aggregation, batching, and re-emission without touching Riftgate.
- The sidecar can crash, restart, or be replaced independently of Riftgate.

**Where it falls short.**
- **A custom protocol is a maintenance liability we don't need.** OTLP already exists, has wide vendor support, and uses gRPC + Protobuf — battle-tested as a wire format. Inventing our own protocol means writing our own SDKs in every language we want to integrate.
- **Sidecar deployment is a v1.0 question, not a v0.1 question.** Forcing it now constrains operators who want to run Riftgate as a standalone binary.
- **Latency cost.** A Unix-socket hop per event is comparable to OTLP/gRPC over loopback; the win is operational, not performance.
- **No ecosystem.** Existing observability tooling speaks OTLP, Prometheus, Loki, Tempo. Speaking a custom protocol is a net negative for adoption.

**Real-world systems that use it.** systemd's journal protocol; some HFT shops with bespoke wire formats. Very specialized.

### 3.5. eBPF-as-sink (no userland sink at all)

**What it is.** Skip userland aggregation entirely. Mark request-lifecycle checkpoints with `usdt` (User Statically-Defined Tracing) probes; let an eBPF program in the kernel aggregate, label, and ship events to whatever observability backend the operator runs. The Riftgate process emits no userland metrics; everything is BPF-side.

**Why it's interesting.**
- Zero observability cost in the request path. `usdt` probes are essentially free (an unconditional jump to a no-op when not attached, a small trampoline when attached).
- Aligns with [Vision §3](../00-vision.md)'s "integrated eBPF observability" pillar in the most aggressive way possible.

**Where it falls short.**
- **eBPF is a `v0.4` commitment, not a `v0.1` one.** Demanding it as the sole observability path means Riftgate has no observability at all in `v0.1` through `v0.3`.
- **eBPF requires `CAP_BPF` (or `CAP_SYS_ADMIN`)** ([NFR-SEC05](../01-requirements/non-functional.md)). Many container runtimes block it. Many developers run on macOS where there is no eBPF. We need a userland sink for the common case.
- **No structured-log story** ([NFR-OBS03](../01-requirements/non-functional.md)). Logs are a userland concern by nature.
- **`usdt` semantics on Linux are subtle.** Aligning probes with the request lifecycle requires careful instrumentation that doesn't pay off until eBPF is attached.

**Real-world systems that use it.** Some database engines (Postgres has DTrace probes; eBPF can attach to them); some HFT shops. As a *complement* to userland observability, sane; as a *replacement*, a non-starter for a general-purpose gateway.

## 4. Tradeoff matrix

| Property | OTel-only (3.1) | OTel + Prom direct (3.2) | Trait + bus + MultiSink (3.3) | Custom protocol (3.4) | eBPF-only (3.5) | Why it matters |
|---|---|---|---|---|---|---|
| Data-plane never-blocks invariant | depends on SDK config | depends on both SDKs | enforced at the bus | enforced (socket write can drop) | enforced (BPF perf-buffer can drop) | The single load-bearing property. |
| Drop-on-full is a first-class metric | no (SDK-internal) | no | yes (`riftgate_observability_dropped_total`) | possible | yes (BPF perf-buffer counter) | Operators need to see when observability is degraded. |
| Pluggability for future sinks | no | retrofitted later | yes (peer `ObservabilitySink` impls) | yes (sidecar consumes our protocol) | no (everything is BPF) | Trait surface is the kernel contract. |
| Cardinality discipline in the type system | no (per-call-site review) | no | yes (`Labels` type) | possible | possible | A high-cardinality label is a real outage risk. |
| Span names as public API | yes (informal) | yes (informal) | yes (formal, single module) | yes | n/a | Renaming a span is a deprecation cycle. |
| Engineering cost in `v0.1` | very low | low | medium | high | very high | Walking-skeleton scope. |
| Latency added to call site | ~50 ns (SDK hot path) | ~100 ns | ~10 µs (enqueue + handoff) | ~10 µs (socket write) | <1 ns (`usdt` probe) | Well under the [`NFR-P05`](../01-requirements/non-functional.md) <5 ms TTFT budget. |
| Works on macOS dev box | yes | yes | yes | yes (Unix socket) | no | Tier-2 dev convenience. |
| Compatibility with `BpfSink` (`v0.4`) | retrofit | retrofit | natural (peer impl) | sidecar runs BPF | redundant | The kernel-and-userland correlation story matters. |
| Vendor coupling | high (OTel SDK) | high (both SDKs) | low (insulated by trait) | very low | low | Long-term maintainability. |
| Conformance-test surface | n/a | n/a | yes (drop-on-full, span-name stability, label cardinality) | sidecar-level | BPF-level | Tests are the contract. |

## 5. Foundational principles

**Drop-on-full bounded ring buffers (LMAX Disruptor lineage).** The Disruptor design — a single producer or multiple producers writing to a ring buffer with a fixed capacity, and consumers that may fall behind — is the canonical pattern for "publish must not block; if it would, drop and increment a counter." Every modern high-throughput observability stack ([Datadog Agent's batcher, Envoy's stats, Linux's perf buffer]) uses some variant of this shape. The discipline is: *the producer never waits*. This is the property the data-plane invariant requires, and the bounded-MPSC bus is the simplest realization that satisfies it.

**Backpressure as policy (Hellerstein and the broader flow-control literature).** The Hellerstein writings on backpressure (and the broader distributed-systems literature on flow control) make the case that backpressure must be an *explicit policy decision* surfaced to the operator, not a hidden buffer-grows-forever default. We apply the same rule on the request-side queue ([Options `004`](004-request-queue.md), [Options `012`](README.md)) and on the observability bus: bounded capacity, drop-on-full, counter, no exceptions. This is the same discipline behind nginx's `worker_connections` cap and Envoy's circuit-breaker stats.

**OpenTelemetry as the standard observability protocol.** OTLP (the OTel Line Protocol) is the de-facto interchange format in 2026: every observability vendor (Datadog, New Relic, Honeycomb, Grafana Cloud) speaks it; the open-source collectors (Tempo, Jaeger, Prometheus via OTel-exporter) speak it; the major runtimes have stable SDKs. The OTel specification (CNCF graduated project) and its semantic conventions are the right interoperability bet. We use OTLP/gRPC over `tonic` for `OtelSink` because gRPC over HTTP/2 is the common-ground transport across the ecosystem.

**Sampling-at-the-source vs sampling-at-the-sink.** The OTel sampling literature (in particular, the OTel Tail Sampling Processor design notes) is unambiguous: *head sampling* (decide at span start whether to keep) is cheap and predictable; *tail sampling* (decide at span end based on outcome) is more accurate but requires per-span buffering. We do *head sampling* at the source for per-token spans (1-in-100 by default) because the sheer volume makes tail sampling infeasible inside the data plane; tail-sampling-style decisions belong in the OTel collector or a downstream pipeline, not in the gateway.

**Cardinality discipline.** The Prometheus best-practices documentation and the OTel cardinality guidelines both warn that an unbounded label (e.g. `request_id`, `user_email`) produces an unbounded number of metric series and is the most common cause of observability backend outages. The defense is to make the label *unconstructible* — the `Labels` type accepts only keys from a registered set, and the values must come from bounded sources (config-defined backend names, registry-defined model names). This is enforced at the type level, not at review time.

## 6. Recommendation

**`v0.1` ships the trait-based shape from `docs/04-design/lld-observability.md`: an `ObservabilitySink` trait, an `OtelSink` impl exporting OTLP/gRPC over `tonic`, a `MultiSink` fan-out impl, a single bounded MPSC bus between the data plane and the sinks, and a `riftgate_observability_dropped_total` counter for events dropped at the bus.**

Concretely:

1. **Trait.** `ObservabilitySink` lives in `crates/riftgate-core::obs` per the LLD sketch:
   ```rust
   pub trait ObservabilitySink: Send + Sync {
       fn publish(&self, event: ObservabilityEvent);
   }
   ```
   `ObservabilityEvent` carries the `SpanStart` / `SpanEnd` / `Counter` / `Histogram` / `Profile` variants from the LLD.
2. **Bus.** `crates/riftgate-obs::bus` exposes a `Publisher` (held by the data plane, cheap-clone) and a `Subscriber` (held by the sink workers). Capacity is configurable per [Options `015`](015-config-model.md) with a sensible default (4096 events). Drop-on-full is the only behavior; there is no "block" or "buffer-grow" mode. The dropped count is exported as `riftgate_observability_dropped_total` (counter) on every sink that supports counters.
3. **Fan-out.** `MultiSink` is itself an `ObservabilitySink` that holds a `Vec<Arc<dyn ObservabilitySink>>` and forwards each event to each inner sink. The drop discipline is bus-level, not sink-level — `MultiSink` does not buffer.
4. **Sinks shipped in `v0.1`.** `OtelSink` (OTLP/gRPC via `opentelemetry-otlp` and `tonic`) plus `MultiSink` (fan-out aggregator). The conformance test in `crates/riftgate-obs/tests/drop_on_full.rs` verifies the bus behavior; the smoke test in `crates/riftgate-obs/tests/otel_smoke.rs` rounds-trips a span against a local OTel collector.
5. **Span-name registry.** `crates/riftgate-obs::spans` defines the canonical names from [`FR-006`](../01-requirements/functional.md) (`received`, `queued`, `dispatched`, `first_token`, `completed`) as `pub const` strings. Span emission sites use these constants; renaming requires a deprecation cycle.
6. **Cardinality discipline.** `crates/riftgate-obs::labels::Labels` accepts keys only from a registered enum. Values for high-risk labels (`backend`, `model`) must be of types whose construction is bounded by config / registry. There is no `set_attribute(&str, &str)` API on the public surface.
7. **`v0.2` adds `PrometheusSink`.** A `/metrics` HTTP endpoint handler that consumes from the same bus, aggregates into a Prometheus `Registry`, and serves the standard exposition format. No data-plane changes.
8. **`v0.4` adds `BpfSink` and `TokenLevelAggregator`.** Per the LLD, both are peer `ObservabilitySink` impls on the same bus; no data-plane changes.

### Conditions under which we'd revisit

- The bus contention measurement (`riftgate_observability_publish_latency_seconds`) shows the single bus becoming a hot point under realistic Riftgate load. We move to per-shard MPSC variants and a multiplexer in front of the sinks. The trait does not change.
- The OTel SDK turns out to add unacceptable per-event allocation cost. We add an event-arena alongside the bus and pre-format OTLP frames.
- A persona emerges that needs sub-millisecond observability accuracy (none on the roadmap). We promote `BpfSink` to a first-class data-path observer.

### What stays available behind feature flags

- `PrometheusSink` lands in `v0.2` and is on by default when its config block is present.
- `BpfSink` lands in `v0.4` behind `--features bpf` and requires `CAP_BPF` per [NFR-SEC05](../01-requirements/non-functional.md).
- A `JsonStdoutSink` for structured logs ([NFR-OBS03](../01-requirements/non-functional.md)) ships in `v0.1` alongside `OtelSink` (small, useful for dev and for environments without an OTel collector).
- A `#[cfg(test)] InMemorySink` ships in `riftgate-core` for unit tests that need to assert on emitted events without a live exporter. This is the FR-X02 second impl.

## 7. What we explicitly reject

- **OTel-only direct emission (no trait, no bus).** Couples the data plane to the OTel SDK and leaves the never-blocks invariant a property of SDK tuning rather than our architecture. Reconsider only if we ever decide observability *should* couple to a single SDK (we do not).
- **Direct dual emission to OTel and Prometheus from each call site.** Duplicate code, asymmetric, doesn't generalize to eBPF or any future sink. The trait + bus shape is dramatically cleaner.
- **A custom binary observability protocol over Unix socket.** OTLP exists, has the ecosystem, has the semantics we need. Not building a parallel standard.
- **eBPF-only observability.** We do not have eBPF until `v0.4`, and many deployments will never run with `CAP_BPF`. eBPF is a complement, not a replacement.
- **Block-on-publish backpressure.** Under no condition does the data plane wait for an exporter. This is the hard line.
- **Tail sampling inside the data plane.** Tail sampling requires per-span buffering, which the data plane will not do. If operators want tail sampling, the OTel collector handles it downstream.
- **`request_id` as a metric label.** Or any other unbounded value. The `Labels` API does not allow it; reviewers do not need to remember to catch it.

## 8. References

1. OpenTelemetry specification — <https://opentelemetry.io/docs/specs/otel/>
2. OTLP (OpenTelemetry Line Protocol) — <https://opentelemetry.io/docs/specs/otlp/>
3. OpenTelemetry semantic conventions — <https://opentelemetry.io/docs/specs/semconv/>
4. OpenTelemetry Tail Sampling Processor design — <https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/processor/tailsamplingprocessor>
5. Prometheus exposition format — <https://prometheus.io/docs/instrumenting/exposition_formats/>
6. Prometheus best practices on labels — <https://prometheus.io/docs/practices/naming/#labels>
7. LMAX Disruptor (Martin Thompson et al.) — <https://lmax-exchange.github.io/disruptor/>
8. Linux `perf_event_open(2)` and the BPF perf buffer / ring buffer — <https://man7.org/linux/man-pages/man2/perf_event_open.2.html>
9. The `opentelemetry`, `opentelemetry_sdk`, and `opentelemetry-otlp` Rust crates — <https://docs.rs/opentelemetry>, <https://docs.rs/opentelemetry_sdk>, <https://docs.rs/opentelemetry-otlp>
10. The `tonic` gRPC implementation for Rust — <https://docs.rs/tonic>
11. The `prometheus` Rust crate — <https://docs.rs/prometheus>
12. Brendan Gregg, *BPF Performance Tools* (Addison-Wesley 2019) — chapters on `usdt` probes and userland-kernel correlation.
13. The `tracing` crate (Tokio) and its subscriber model — <https://docs.rs/tracing>
14. Cindy Sridharan, *Distributed Systems Observability* (O'Reilly 2018).
