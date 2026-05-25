//! Observability sink trait + supporting types + `InMemorySink` test impl.
//!
//! ```text
//!   data_plane --publish(event)--> [bounded MPSC]
//!                                       |
//!                       +---------------+---------------+
//!                       v               v               v
//!                  OtelSink         (PrometheusSink (BpfSink
//!                                    in v0.2)        in v0.4)
//!                       \               |               /
//!                        +-->  full?  --> drop + dropped_total++
//! ```
//!
//! Per [ADR 0011](../../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md)
//! the v0.1 production impls (`OtelSink`, `MultiSink`, `JsonStdoutSink`) live in
//! `crates/riftgate-obs`. This module declares the trait, the typed
//! `ObservabilityEvent` enum, and the `InMemorySink` second impl for unit
//! tests.
//!
//! See [`docs/04-design/lld-observability.md`](../../../docs/04-design/lld-observability.md)
//! and [Options 013](../../../docs/05-options/013-observability-sink.md).

use crate::types::RequestId;
use std::sync::Mutex;
use std::time::Duration;

/// Bounded set of allowed label *keys*.
///
/// Per [ADR 0011](../../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md)
/// the cardinality discipline lives in the type system: a `LabelKey` can
/// only take values from this enum. Adding a new label is a deliberate API
/// change that requires a dashboard plan.
///
/// **What is NOT here:** anything per-request (`request_id`), anything per-user
/// (`user_email`, `tenant_email`), anything stringly-typed by an end user.
/// These cardinality classes are unbounded and are excluded by construction.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum LabelKey {
    /// `route` — bounded by configured route table size.
    Route,
    /// `backend` — bounded by configured backend pool size.
    Backend,
    /// `model` — bounded by the model registry.
    Model,
    /// `method` — HTTP method (GET, POST, ...). Bounded.
    Method,
    /// `status` — HTTP status code class (`2xx`, `4xx`, `5xx`). Bounded.
    StatusClass,
    /// `tenant` — bounded by configured tenant table.
    Tenant,
    /// `event` — for compound metrics with several sub-events
    /// (e.g. `riftgate_timers_total{event=scheduled|cancelled|fired}`).
    Event,
}

impl LabelKey {
    /// Return the wire-format string for this label key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Backend => "backend",
            Self::Model => "model",
            Self::Method => "method",
            Self::StatusClass => "status_class",
            Self::Tenant => "tenant",
            Self::Event => "event",
        }
    }
}

/// Single label as a `(key, value)` pair.
///
/// `value` is `String` because OTel and Prometheus both accept arbitrary
/// strings; the cardinality safety is enforced at the *call site* by the
/// fact that values come from bounded sources (config-defined backend
/// names, registry-defined model names, status-class enum strings).
#[derive(Debug, Clone)]
pub struct Label {
    /// The label key (must come from the registered enum).
    pub key: LabelKey,
    /// The label value. Keep this bounded by construction.
    pub value: String,
}

/// Set of labels attached to a metric or span event.
#[derive(Debug, Default, Clone)]
pub struct Labels {
    inner: Vec<Label>,
}

impl Labels {
    /// Construct an empty `Labels`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with capacity for a known number of labels (avoids a
    /// reallocation on the hot path).
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
        }
    }

    /// Insert a label.
    ///
    /// Duplicates are preserved; OTel and Prometheus both treat duplicates
    /// as separate dimensions on most metric types.
    pub fn insert(mut self, key: LabelKey, value: impl Into<String>) -> Self {
        self.inner.push(Label {
            key,
            value: value.into(),
        });
        self
    }

    /// Iterate over labels in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.inner.iter()
    }

    /// Number of labels.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` if there are no labels.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Free-form attributes on a span. Distinct from [`Labels`] because span
/// attributes are not used as cardinality dimensions on metrics; they are
/// per-trace metadata that the OTel SDK indexes for search.
///
/// Keys are `&'static str` so attribute names are part of the binary's
/// public API by construction.
#[derive(Debug, Default, Clone)]
pub struct Attributes {
    inner: Vec<(&'static str, AttrValue)>,
}

impl Attributes {
    /// Construct empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
        }
    }

    /// Insert an attribute.
    pub fn insert(mut self, key: &'static str, value: impl Into<AttrValue>) -> Self {
        self.inner.push((key, value.into()));
        self
    }

    /// Iterate over attributes.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &AttrValue)> {
        self.inner.iter().map(|(k, v)| (*k, v))
    }

    /// Number of attributes.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` if there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Typed attribute value.
#[derive(Debug, Clone)]
pub enum AttrValue {
    /// A string. Preferred for human-readable values.
    Str(String),
    /// An i64. Most numeric attributes fit.
    I64(i64),
    /// An f64. Use sparingly; OTel encodes f64s less efficiently than ints.
    F64(f64),
    /// A bool.
    Bool(bool),
}

impl From<&str> for AttrValue {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}
impl From<String> for AttrValue {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}
impl From<i64> for AttrValue {
    fn from(n: i64) -> Self {
        Self::I64(n)
    }
}
impl From<u32> for AttrValue {
    fn from(n: u32) -> Self {
        Self::I64(i64::from(n))
    }
}
impl From<u64> for AttrValue {
    #[allow(clippy::cast_possible_wrap)]
    fn from(n: u64) -> Self {
        Self::I64(n as i64)
    }
}
impl From<f64> for AttrValue {
    fn from(n: f64) -> Self {
        Self::F64(n)
    }
}
impl From<bool> for AttrValue {
    fn from(n: bool) -> Self {
        Self::Bool(n)
    }
}

/// Profile sample (used by `BpfSink` in `v0.4`).
#[derive(Debug, Clone)]
pub struct ProfileSample {
    /// Stack frames as symbolicated strings.
    pub stack: Vec<String>,
    /// Weight of this sample (typically the number of CPU cycles).
    pub weight: u64,
}

/// Profile classification (CPU on/off, syscall stalls, etc.). New in `v0.4`.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum ProfileKind {
    /// On-CPU sampling (where is the gateway spending time?).
    OnCpu,
    /// Off-CPU sampling (what is the gateway waiting on?).
    OffCpu,
    /// Syscall-rate sampling.
    Syscall,
}

/// Typed observability event.
///
/// The data plane publishes these via [`ObservabilitySink::publish`]; sink
/// workers translate into the sink-specific format.
#[derive(Debug, Clone)]
pub enum ObservabilityEvent {
    /// A span has started.
    SpanStart {
        /// Per-request id; used to correlate across span events.
        request_id: RequestId,
        /// Canonical span name (one of the constants in
        /// `crates/riftgate-obs::spans`; see Phase H).
        name: &'static str,
        /// Free-form attributes on the span.
        attributes: Attributes,
    },
    /// A span has ended.
    SpanEnd {
        /// Per-request id (matches the start event).
        request_id: RequestId,
        /// Canonical span name (matches the start event).
        name: &'static str,
        /// Wall-clock duration of the span.
        duration: Duration,
    },
    /// A counter increment.
    Counter {
        /// Metric name (e.g. `"riftgate_requests_total"`).
        name: &'static str,
        /// Increment value (typically 1).
        value: u64,
        /// Bounded labels.
        labels: Labels,
    },
    /// A histogram observation.
    Histogram {
        /// Metric name (e.g. `"riftgate_request_duration_seconds"`).
        name: &'static str,
        /// Observed value.
        value: f64,
        /// Bounded labels.
        labels: Labels,
    },
    /// A profile sample (new in `v0.4` via `BpfSink`).
    Profile {
        /// Profile classification.
        kind: ProfileKind,
        /// Sample data.
        samples: Vec<ProfileSample>,
    },
}

/// Observability sink trait.
///
/// Implementations consume from the bounded MPSC bus (see Phase H,
/// `crates/riftgate-obs::bus`) and translate `ObservabilityEvent` into
/// their sink-specific wire format.
///
/// **`Send + Sync`** — sinks are constructed once at startup and shared by
/// the bus's worker pool.
///
/// **`publish` MUST NOT block.** This is the data-plane never-blocks
/// invariant (see [Options 013](../../../docs/05-options/013-observability-sink.md)
/// §6 and [ADR 0011](../../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md)).
/// In practice the bus enforces drop-on-full at the bus level; individual
/// sinks may queue internally but must not surface back-pressure to the
/// caller.
pub trait ObservabilitySink: Send + Sync {
    /// Publish an event. Non-blocking; drops on internal back-pressure.
    fn publish(&self, event: ObservabilityEvent);
}

/// In-memory sink for unit tests.
///
/// Stores every published event in a `Mutex<Vec<...>>` for inspection.
/// Useful for tests that assert "the data plane emitted span X" without
/// rounding-tripping through OTel.
pub struct InMemorySink {
    inner: Mutex<Vec<ObservabilityEvent>>,
}

impl InMemorySink {
    /// Construct an empty `InMemorySink`.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Drain all events captured so far.
    pub fn drain(&self) -> Vec<ObservabilityEvent> {
        let mut g = self.inner.lock().expect("InMemorySink poisoned");
        std::mem::take(&mut *g)
    }

    /// Snapshot the current event count.
    pub fn count(&self) -> usize {
        self.inner.lock().expect("InMemorySink poisoned").len()
    }
}

impl Default for InMemorySink {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservabilitySink for InMemorySink {
    fn publish(&self, event: ObservabilityEvent) {
        if let Ok(mut g) = self.inner.lock() {
            g.push(event);
        }
        // Mutex poisoning is treated as drop; we do not surface error
        // because the trait contract is non-blocking and infallible.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_key_strings_are_stable() {
        assert_eq!(LabelKey::Route.as_str(), "route");
        assert_eq!(LabelKey::Backend.as_str(), "backend");
        assert_eq!(LabelKey::Model.as_str(), "model");
        assert_eq!(LabelKey::Method.as_str(), "method");
        assert_eq!(LabelKey::StatusClass.as_str(), "status_class");
        assert_eq!(LabelKey::Tenant.as_str(), "tenant");
        assert_eq!(LabelKey::Event.as_str(), "event");
    }

    #[test]
    fn labels_builder_pattern() {
        let l = Labels::new()
            .insert(LabelKey::Backend, "openai-prod")
            .insert(LabelKey::Method, "POST");
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn in_memory_sink_captures_events() {
        let sink = InMemorySink::new();
        sink.publish(ObservabilityEvent::Counter {
            name: "riftgate_requests_total",
            value: 1,
            labels: Labels::new(),
        });
        assert_eq!(sink.count(), 1);
        let drained = sink.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn obs_sink_is_dyn_safe() {
        let _s: Box<dyn ObservabilitySink> = Box::new(InMemorySink::new());
    }
}
