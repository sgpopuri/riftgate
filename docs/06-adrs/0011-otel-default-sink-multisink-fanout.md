# ADR 0011. ObservabilitySink trait + bounded-MPSC bus + OtelSink + MultiSink in v0.1

> **Date:** 2026-05-10
> **Status:** accepted
> **Options doc:** [013-observability-sink](../05-options/013-observability-sink.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs an observability output surface for `v0.1` that satisfies [`FR-006`](../01-requirements/functional.md) (OpenTelemetry traces with the canonical span sequence — `received`, `queued`, `dispatched`, `first_token`, `completed` — visible in a local OTel collector) and [`NFR-OBS01`](../01-requirements/non-functional.md), while preserving the load-bearing data-plane invariant from [`docs/03-architecture/hld.md` §6](../03-architecture/hld.md): **the data plane never blocks on the observability plane**. Full exploration of candidates (OTel-only direct emission, OTel + Prometheus direct emission, trait + bus + MultiSink, custom binary protocol, eBPF-only) lives in [Options `013`](../05-options/013-observability-sink.md).

The forces summarized: drop-on-full at the bus is the only acceptable backpressure posture; cardinality discipline must be enforced in the type system rather than at review time; span names are part of the public API; future sinks (`PrometheusSink` in `v0.2`, `BpfSink` and `TokenLevelAggregator` in `v0.4`) must be peer impls of the same trait.

## Decision

**Riftgate `v0.1` ships an `ObservabilitySink` trait in `riftgate-core`, an `OtelSink` impl (OTLP/gRPC over `tonic`) and a `MultiSink` fan-out impl in `riftgate-obs`, a single bounded MPSC bus between the data plane and the sinks, a `riftgate_observability_dropped_total` counter for events dropped at the bus, a canonical span-name registry, and a `Labels` type that constrains label keys to a registered enum.**

The discipline:

- The `ObservabilitySink` trait lives in `crates/riftgate-core::obs` per the sketch in [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md). One method: `publish(&self, event: ObservabilityEvent)`. `ObservabilityEvent` carries the `SpanStart` / `SpanEnd` / `Counter` / `Histogram` / `Profile` variants from the LLD.
- `crates/riftgate-obs::bus` exposes a `Publisher` (held by the data plane, cheap-clone) and a `Subscriber` (held by the sink workers). Capacity defaults to 4096 events; configurable via the `[obs] bus_capacity` key per [ADR `0012`](0012-static-toml-env-override-v01.md). Drop-on-full is the only behavior; no "block" or "buffer-grow" mode exists.
- The dropped count is exported as `riftgate_observability_dropped_total` (counter) on every sink that supports counters.
- `MultiSink` holds `Vec<Arc<dyn ObservabilitySink>>` and fans out each event to each inner sink. The drop discipline is bus-level, not sink-level — `MultiSink` does not buffer.
- `OtelSink` exports OTLP/gRPC via `opentelemetry-otlp` over `tonic`. Endpoint is configurable; default is `http://localhost:4317` (OTel collector convention).
- `JsonStdoutSink` ships in `v0.1` alongside `OtelSink` for structured logs ([`NFR-OBS03`](../01-requirements/non-functional.md)) and for environments without a collector.
- Span-name registry: `crates/riftgate-obs::spans` defines the canonical names from FR-006 as `pub const &'static str`. Span emission sites use these constants exclusively.
- Cardinality discipline: `crates/riftgate-obs::labels::Labels` accepts keys only from a registered enum. There is no `set_attribute(&str, &str)` API on the public surface.
- Sampling-at-the-source: per-token spans are sampled 1-in-100 by default; configurable. Full per-token data lives in the WAL (`v0.2`+).
- A `#[cfg(test)] InMemorySink` ships in `riftgate-core::obs` as the FR-X02 second impl for unit tests.
- `PrometheusSink` lands in `v0.2`. `BpfSink` and `TokenLevelAggregator` land in `v0.4`. All three are peer `ObservabilitySink` impls; no data-plane changes.

## Consequences

- **Positive:**
  - The data-plane invariant is enforced at the bus, not at any sink. No matter how badly an exporter behaves, the data plane never waits.
  - Drop-on-full is observable as a first-class metric, not an SDK-internal counter.
  - One pluggability seam: every future sink is an `ObservabilitySink` impl behind the same trait; data-plane call sites do not change.
  - Cardinality discipline lives in the type system (`Labels` rejects unregistered keys); high-cardinality outages cannot be caused by a stray `set_attribute("request_id", ...)` call.
  - Span names are formal public API; renaming is a deprecation cycle.
  - Sampling-at-the-source is a first-class concept; no SDK-internal sampling we have to reverse-engineer.
- **Negative / accepted tradeoffs:**
  - One extra hop on emission (publish → bus → sink worker → exporter). Adds ~10 µs of latency to span emission; well under the [`NFR-P05`](../01-requirements/non-functional.md) <5 ms TTFT budget.
  - One bounded MPSC under high publish load is a contention candidate. We pick a per-shard MPSC variant where measurement justifies it; the LLD documents the cross-shard fan-in.
  - More code than the OTel-only candidate (~500 lines + tests for the bus, MultiSink, OtelSink, JsonStdoutSink, span registry, Labels, conformance + smoke tests). Bounded engineering cost.
  - Sink workers are an additional task per sink; cost is small.
- **Future work this enables:**
  - `PrometheusSink` in `v0.2` consumes from the same bus; `/metrics` endpoint is a thin HTTP handler.
  - `BpfSink` and `TokenLevelAggregator` in `v0.4` consume from the same bus; userland-kernel correlation lives in their impl.
  - Per-tenant observability scoping (`v1.0`) layers on top via the `Labels` type.
  - Replay-driven trace re-emission via the WAL (`v0.2`+) without changing the bus shape.
- **Future work this forecloses (until superseded):**
  - We will not couple the data plane to the OTel SDK directly.
  - We will not allow direct dual emission to OTel and Prometheus from call sites.
  - We will not invent a custom binary observability protocol; OTLP is the wire format.
  - We will not allow tail-sampling inside the data plane; tail sampling belongs in the OTel collector or a downstream pipeline.
  - We will not allow unbounded label values (e.g. `request_id`); the `Labels` type enforces this.
  - We will not block-on-publish under any condition.

## Compliance

- `crates/riftgate-core::obs::ObservabilitySink` is the single trait; `OtelSink`, `MultiSink`, `JsonStdoutSink`, and `InMemorySink` are the impls in `v0.1`.
- `crates/riftgate-obs/tests/drop_on_full.rs` saturates the bus and verifies that `publish` never blocks and that `riftgate_observability_dropped_total` increments.
- `crates/riftgate-obs/tests/otel_smoke.rs` rounds-trips a span against a local OTel collector (run via docker-compose; skipped if the collector is unreachable, with a CI nightly that runs against a real collector).
- `crates/riftgate-obs::spans` is the canonical span-name registry; CI fails on emission sites that use string literals instead of the constants.
- `crates/riftgate-obs::labels::Labels` rejects unregistered keys at compile time (where possible) and at construction time (for dynamic keys).
- Adding a new `ObservabilitySink` impl requires passing the bus-conformance test (drop-on-full, cardinality discipline, span-name stability).

## Notes

- The choice of OTLP/gRPC over `tonic` follows the OTel ecosystem default. OTLP/HTTP is supported by the OTel SDK and can be added later behind a config switch (`[obs.otel] transport = "http" | "grpc"`).
- The bounded-MPSC with drop-on-full pattern is the LMAX Disruptor lineage; the same posture appears on the request-side queue ([ADR `0005`](0005-sharded-mpmc-queue.md)) and will appear on the future backpressure decision (Options [`012`](../05-options/README.md)).
- The 4096 default bus capacity is conservative; under sustained high publish load (eBPF-attached profiles in `v0.4`) we may raise it. The `riftgate_observability_dropped_total` counter is the operator-visible signal that capacity is undersized.
- Sampling-at-the-source for per-token spans (1-in-100 default) is *head sampling* — fast, predictable, and infeasible-to-tail-sample at our event rate. Tail-sampling-style decisions belong downstream in the OTel collector.
- The `Labels` type is the place we encode the [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md) "cardinality discipline" pitfall as a compile-time / construction-time guard. This is one of the few places in the kernel where we make a class of bug *unconstructible* rather than relying on review.
