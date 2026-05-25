//! `JsonStdoutSink` — writes one line of JSON per event to stdout.
//!
//! Default for [`NFR-OBS03`](../../../docs/01-requirements/non-functional.md)
//! (structured logs). Also useful in environments without an OTel
//! collector (local development, CI test logs).

use riftgate_core::obs::{
    AttrValue, Attributes, Label, Labels, ObservabilityEvent, ObservabilitySink, ProfileKind,
};
use serde::Serialize;
use std::io::{self, Write};
use std::sync::Mutex;

/// One-line-per-event JSON sink to stdout.
///
/// Holds an `io::Write`-impl writer behind a `Mutex` for thread-safe
/// line-atomic emission. The default `new()` constructor uses
/// `io::stdout().lock()`-style serialization.
pub struct JsonStdoutSink {
    inner: Mutex<Box<dyn Write + Send>>,
}

impl JsonStdoutSink {
    /// Construct a `JsonStdoutSink` writing to stdout.
    pub fn stdout() -> Self {
        Self::new(Box::new(io::stdout()))
    }

    /// Construct a `JsonStdoutSink` writing to the given writer. Useful
    /// for tests that need to capture output.
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Self {
            inner: Mutex::new(writer),
        }
    }
}

impl Default for JsonStdoutSink {
    fn default() -> Self {
        Self::stdout()
    }
}

#[derive(Serialize)]
struct OutEnvelope<'a> {
    kind: &'a str,
    #[serde(flatten)]
    body: OutBody<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum OutBody<'a> {
    Span {
        request_id: String,
        name: &'a str,
        attributes: Vec<OutAttr<'a>>,
        duration_ms: Option<u64>,
    },
    Metric {
        name: &'a str,
        value: f64,
        labels: Vec<OutLabel<'a>>,
    },
    Profile {
        kind: &'a str,
        sample_count: usize,
    },
}

#[derive(Serialize)]
struct OutAttr<'a> {
    key: &'a str,
    value: serde_json::Value,
}

#[derive(Serialize)]
struct OutLabel<'a> {
    key: &'a str,
    value: &'a str,
}

fn attr_to_json(v: &AttrValue) -> serde_json::Value {
    match v {
        AttrValue::Str(s) => serde_json::Value::String(s.clone()),
        AttrValue::I64(n) => serde_json::Value::from(*n),
        AttrValue::F64(n) => serde_json::Value::from(*n),
        AttrValue::Bool(b) => serde_json::Value::from(*b),
    }
}

fn labels_to_out<'a>(labels: &'a Labels) -> Vec<OutLabel<'a>> {
    labels
        .iter()
        .map(|Label { key, value }| OutLabel {
            key: key.as_str(),
            value: value.as_str(),
        })
        .collect()
}

fn attributes_to_out<'a>(attrs: &'a Attributes) -> Vec<OutAttr<'a>> {
    attrs
        .iter()
        .map(|(k, v)| OutAttr {
            key: k,
            value: attr_to_json(v),
        })
        .collect()
}

impl ObservabilitySink for JsonStdoutSink {
    fn publish(&self, event: ObservabilityEvent) {
        let envelope = match &event {
            ObservabilityEvent::SpanStart {
                request_id,
                name,
                attributes,
            } => OutEnvelope {
                kind: "span_start",
                body: OutBody::Span {
                    request_id: request_id.to_string(),
                    name,
                    attributes: attributes_to_out(attributes),
                    duration_ms: None,
                },
            },
            ObservabilityEvent::SpanEnd {
                request_id,
                name,
                duration,
            } => OutEnvelope {
                kind: "span_end",
                body: OutBody::Span {
                    request_id: request_id.to_string(),
                    name,
                    attributes: Vec::new(),
                    duration_ms: Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
                },
            },
            ObservabilityEvent::Counter {
                name,
                value,
                labels,
            } => OutEnvelope {
                kind: "counter",
                body: OutBody::Metric {
                    name,
                    #[allow(clippy::cast_precision_loss)]
                    value: *value as f64,
                    labels: labels_to_out(labels),
                },
            },
            ObservabilityEvent::Histogram {
                name,
                value,
                labels,
            } => OutEnvelope {
                kind: "histogram",
                body: OutBody::Metric {
                    name,
                    value: *value,
                    labels: labels_to_out(labels),
                },
            },
            ObservabilityEvent::Profile { kind, samples } => OutEnvelope {
                kind: "profile",
                body: OutBody::Profile {
                    kind: match kind {
                        ProfileKind::OnCpu => "on_cpu",
                        ProfileKind::OffCpu => "off_cpu",
                        ProfileKind::Syscall => "syscall",
                    },
                    sample_count: samples.len(),
                },
            },
        };

        if let Ok(line) = serde_json::to_string(&envelope) {
            if let Ok(mut w) = self.inner.lock() {
                let _ = writeln!(w, "{line}");
            }
        }
    }
}
