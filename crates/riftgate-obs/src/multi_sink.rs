//! Fan-out sink. See module-level docs.

use riftgate_core::obs::{ObservabilityEvent, ObservabilitySink};
use std::sync::Arc;

/// `ObservabilitySink` that forwards each event to multiple inner
/// sinks.
///
/// Use this in conjunction with [`crate::Bus`] when more than one sink
/// needs to consume the bus output (e.g. an `OtelSink` + a
/// `JsonStdoutSink` for development). The data-plane never-blocks
/// invariant is enforced at the bus, not at `MultiSink`; if any inner
/// sink blocks, only the worker thread is blocked.
#[derive(Clone, Default)]
pub struct MultiSink {
    sinks: Vec<Arc<dyn ObservabilitySink>>,
}

impl MultiSink {
    /// Construct an empty `MultiSink`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a sink. Returns `self` for builder-style chaining.
    pub fn with(mut self, sink: Arc<dyn ObservabilitySink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Number of inner sinks.
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// `true` if no inner sinks are registered.
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

impl ObservabilitySink for MultiSink {
    fn publish(&self, event: ObservabilityEvent) {
        for sink in &self.sinks {
            sink.publish(event.clone());
        }
    }
}
