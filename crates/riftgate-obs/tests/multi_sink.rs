//! `MultiSink` fan-out test.

use riftgate_core::obs::{InMemorySink, Labels, ObservabilityEvent, ObservabilitySink};
use riftgate_obs::MultiSink;
use std::sync::Arc;

#[test]
fn multi_sink_forwards_each_event_to_each_inner() {
    let a = Arc::new(InMemorySink::new());
    let b = Arc::new(InMemorySink::new());
    let c = Arc::new(InMemorySink::new());
    let multi = MultiSink::new()
        .with(a.clone() as Arc<dyn ObservabilitySink>)
        .with(b.clone() as Arc<dyn ObservabilitySink>)
        .with(c.clone() as Arc<dyn ObservabilitySink>);
    for i in 0..7 {
        multi.publish(ObservabilityEvent::Counter {
            name: "riftgate_test_counter",
            value: i,
            labels: Labels::new(),
        });
    }
    assert_eq!(a.count(), 7);
    assert_eq!(b.count(), 7);
    assert_eq!(c.count(), 7);
}

#[test]
fn multi_sink_with_no_inner_sinks_is_no_op() {
    let multi = MultiSink::new();
    assert!(multi.is_empty());
    multi.publish(ObservabilityEvent::Counter {
        name: "riftgate_test_counter",
        value: 1,
        labels: Labels::new(),
    });
    // No panic, no side effect.
}
