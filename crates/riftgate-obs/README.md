# riftgate-obs

The v0.1 observability surface, per [Options 013](../../docs/05-options/013-observability-sink.md) and [ADR 0011](../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md).

```text
  data_plane --[Bus::publisher().publish(event)]-->  bounded MPSC channel
                                                          |
                                                          v
                                                  worker thread loop
                                                          |
                                                          v
                                                  sink.publish(event)
                                                          |
                                                +---------+---------+
                                                v                   v
                                          OtelSink            JsonStdoutSink
                                          (or MultiSink wrapping multiple sinks)
```

- **`Bus`** owns the bounded MPSC and the worker. `bus.publisher().publish(event)` is the data-plane API: non-blocking, drops on full, increments `riftgate_observability_dropped_total`.
- **`Publisher`** is a cheap-clone handle held by the data plane.
- **`OtelSink`** emits OpenTelemetry spans via `opentelemetry::global::tracer("riftgate")`. The OTLP/gRPC exporter is initialised by the binary's bootstrap (where the Tokio runtime exists); this sink just consumes events and converts them to span calls.
- **`JsonStdoutSink`** emits structured JSON one event per line. Default for [`NFR-OBS03`](../../docs/01-requirements/non-functional.md) and useful in environments without an OTel collector.
- **`MultiSink`** fans out to a `Vec<Arc<dyn ObservabilitySink>>`. The data-plane never-blocks invariant is enforced at the bus, not at any sink; `MultiSink` is a helper for callers that want to wire multiple sinks behind one trait object.
- **`spans`** module — the canonical span name registry per [`FR-006`](../../docs/01-requirements/functional.md).

## Tests

- `tests/bus_drop_on_full.rs` — saturates the bus and verifies that `publish` never blocks and that the dropped counter increments.
- `tests/multi_sink.rs` — verifies fan-out: every inner sink sees every event.
- `tests/span_name_uniqueness.rs` — compile-time-style check that the canonical span name constants are pairwise unique strings.
- `tests/otel_smoke.rs` — `#[ignore]` by default; run with `cargo test -- --ignored otel_smoke` against a local OTel collector to validate end-to-end OTLP export.
