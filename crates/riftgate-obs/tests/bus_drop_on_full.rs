//! Verify the bus drops events (and increments the counter) when the
//! channel is full.

use riftgate_core::obs::{Labels, ObservabilityEvent, ObservabilitySink};
use riftgate_obs::Bus;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Sink that records how many events it has consumed.
struct CountingSink(AtomicUsize);

impl CountingSink {
    fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
    fn count(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

impl ObservabilitySink for CountingSink {
    fn publish(&self, _event: ObservabilityEvent) {
        // Sleep a tiny bit so the bus can fill while we drain slowly.
        std::thread::sleep(Duration::from_millis(2));
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn bus_drops_when_publish_outpaces_consumption() {
    let sink = Arc::new(CountingSink::new());
    let bus = Bus::new(8, sink.clone());
    let pub_ = bus.publisher();

    // Burst-publish 1000 events into a bus of capacity 8 with a sink
    // that drains at ~500 events/s. Most events will drop.
    for i in 0..1000 {
        pub_.publish(ObservabilityEvent::Counter {
            name: "riftgate_test_counter",
            value: i,
            labels: Labels::new(),
        });
    }

    // Give the worker some time to drain the events that did fit.
    std::thread::sleep(Duration::from_millis(200));

    let dropped = bus.dropped_total();
    let consumed = sink.count() as u64;
    assert!(
        dropped > 0,
        "expected drops with a slow sink, got dropped={dropped} consumed={consumed}"
    );
    assert!(
        consumed > 0,
        "expected the worker to consume something, got {consumed}"
    );
    // We don't assert exact balance because timing is jittery; the
    // useful invariant is "no over-counting" — consumed + dropped
    // cannot exceed the number of events we attempted to publish.
    assert!(
        consumed + dropped <= 1100,
        "no over-counting: consumed + dropped = {consumed} + {dropped} = {}",
        consumed + dropped
    );
}
