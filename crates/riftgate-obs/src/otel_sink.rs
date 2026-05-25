//! `OtelSink` — emits OpenTelemetry spans for each consumed
//! `ObservabilityEvent`.
//!
//! The OTLP/gRPC exporter setup happens in the binary's bootstrap
//! (where the Tokio runtime exists; OTel SDK requires it). This sink
//! consumes events and converts them to span calls against the global
//! `opentelemetry::global::tracer("riftgate")`. It does NOT initialise
//! the SDK itself.
//!
//! Per [ADR 0011](../../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md):
//! - `SpanEnd` events become OTel spans (start + duration as attribute +
//!   end). v0.1 does not buffer SpanStart events; the data plane should
//!   emit a SpanEnd event for every meaningful span.
//! - `Counter` and `Histogram` events emit `tracing::trace!` log
//!   entries in v0.1; the OTLP metrics path lands in v0.2 alongside the
//!   `PrometheusSink`.
//! - `Profile` events are accepted but no-op'd in v0.1; the `BpfSink`
//!   in v0.4 consumes them.

use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::trace::{Span, Tracer};
use riftgate_core::obs::{AttrValue, ObservabilityEvent, ObservabilitySink};

/// OpenTelemetry-emitting sink. See module-level docs.
pub struct OtelSink {
    tracer_name: &'static str,
}

impl OtelSink {
    /// Construct an `OtelSink` that emits spans under the named tracer.
    /// The default name `"riftgate"` matches the canonical instrumentation
    /// scope in the OTel collector configuration shipped under
    /// `examples/minimal-proxy`.
    pub const fn with_tracer_name(tracer_name: &'static str) -> Self {
        Self { tracer_name }
    }

    /// Construct an `OtelSink` with the default tracer name `"riftgate"`.
    pub const fn new() -> Self {
        Self::with_tracer_name("riftgate")
    }
}

impl Default for OtelSink {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
fn attr_to_kv(key: &'static str, value: &AttrValue) -> KeyValue {
    match value {
        AttrValue::Str(s) => KeyValue::new(key, s.clone()),
        AttrValue::I64(n) => KeyValue::new(key, *n),
        AttrValue::F64(n) => KeyValue::new(key, *n),
        AttrValue::Bool(b) => KeyValue::new(key, *b),
    }
}

impl ObservabilitySink for OtelSink {
    fn publish(&self, event: ObservabilityEvent) {
        let tracer = global::tracer(self.tracer_name);
        match event {
            ObservabilityEvent::SpanEnd {
                request_id,
                name,
                duration,
            } => {
                let mut span = tracer.start(name);
                span.set_attribute(KeyValue::new("riftgate.request_id", request_id.to_string()));
                span.set_attribute(KeyValue::new(
                    "riftgate.duration_ms",
                    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
                ));
                span.end();
            }
            ObservabilityEvent::SpanStart {
                name,
                attributes,
                request_id,
            } => {
                // SpanStart is informational in v0.1: most workflows
                // emit SpanEnd which carries the duration. We still
                // emit a zero-duration span so SpanStart is observable.
                let mut span = tracer.start(name);
                span.set_attribute(KeyValue::new("riftgate.request_id", request_id.to_string()));
                for (k, v) in attributes.iter() {
                    span.set_attribute(attr_to_kv(k, v));
                }
                span.end();
            }
            ObservabilityEvent::Counter {
                name,
                value,
                labels: _,
            } => {
                tracing::trace!(
                    metric = name,
                    counter_value = value,
                    "counter (otlp metrics: v0.2)"
                );
            }
            ObservabilityEvent::Histogram {
                name,
                value,
                labels: _,
            } => {
                tracing::trace!(
                    metric = name,
                    histogram_value = value,
                    "histogram (otlp metrics: v0.2)"
                );
            }
            ObservabilityEvent::Profile { .. } => {
                // v0.4 BpfSink path; OtelSink does not consume profile
                // events.
            }
        }
    }
}
