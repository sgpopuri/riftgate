//! Bounded MPSC bus between the data plane and the sinks.
//!
//! - [`Publisher`] is held by the data plane; [`Publisher::publish`] is
//!   non-blocking and drops on full, incrementing
//!   `riftgate_observability_dropped_total`.
//! - [`Bus`] owns the channel and a worker thread that drains events to
//!   a single [`ObservabilitySink`]. To fan out to multiple sinks,
//!   construct a [`crate::MultiSink`] and pass it as the `sink`.
//!
//! The drop-on-full discipline is enforced at the publisher, not at the
//! sink: even if the sink hangs forever, the data plane never blocks.

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use riftgate_core::obs::{ObservabilityEvent, ObservabilitySink};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

/// Data-plane handle to the bus. Cheap to clone (one `Arc`).
#[derive(Clone)]
pub struct Publisher {
    sender: Sender<ObservabilityEvent>,
    dropped: Arc<AtomicU64>,
}

impl Publisher {
    /// Publish an event. Non-blocking; drops on full.
    ///
    /// This is the **only** path by which the data plane emits
    /// observability events. The contract is:
    ///
    /// - This call never blocks.
    /// - This call never fails (no error variant).
    /// - On full or disconnected channel, the event is dropped and the
    ///   `dropped_total` counter is incremented.
    pub fn publish(&self, event: ObservabilityEvent) {
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Snapshot the total number of events dropped at the bus since
    /// startup. Exposed as `riftgate_observability_dropped_total`.
    pub fn dropped_total(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Owner of the bus channel and the per-bus worker thread.
///
/// On `Bus::new(capacity, sink)`:
///
/// 1. A bounded channel of `capacity` events is created.
/// 2. A worker thread is spawned that reads events one at a time and
///    calls `sink.publish(event)` for each.
/// 3. A [`Publisher`] handle is constructed for the data plane.
///
/// On drop, the channel sender (held by the `Publisher` and its
/// clones) goes out of scope; the worker exits when the receiver returns
/// `Err(RecvError)`.
pub struct Bus {
    publisher: Publisher,
    /// Held to keep the worker alive for the lifetime of the bus.
    /// Joined on drop via the implicit `JoinHandle` drop (which detaches
    /// the thread; we don't block shutdown on it).
    _worker: Option<JoinHandle<()>>,
}

impl Bus {
    /// Construct a new `Bus` of the given capacity, draining events to
    /// `sink`.
    ///
    /// `sink` is shared via `Arc` so the worker thread can move it
    /// without taking ownership of the caller's handle.
    pub fn new(capacity: usize, sink: Arc<dyn ObservabilitySink>) -> Self {
        let (sender, receiver) = bounded::<ObservabilityEvent>(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let publisher = Publisher {
            sender,
            dropped: Arc::clone(&dropped),
        };
        let worker = thread::Builder::new()
            .name("riftgate-obs-worker".into())
            .spawn(move || drain(receiver, sink))
            .expect("could not spawn riftgate-obs worker thread");
        Self {
            publisher,
            _worker: Some(worker),
        }
    }

    /// Borrow a clone-cheap [`Publisher`] handle for the data plane.
    pub fn publisher(&self) -> Publisher {
        self.publisher.clone()
    }

    /// Snapshot the dropped-events counter.
    pub fn dropped_total(&self) -> u64 {
        self.publisher.dropped_total()
    }
}

fn drain(receiver: Receiver<ObservabilityEvent>, sink: Arc<dyn ObservabilitySink>) {
    while let Ok(event) = receiver.recv() {
        sink.publish(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::obs::{InMemorySink, Labels};
    use std::time::Duration;

    #[test]
    fn publish_round_trip_to_sink() {
        let sink = Arc::new(InMemorySink::new());
        let bus = Bus::new(16, sink.clone());
        let pub_ = bus.publisher();
        for i in 0..5 {
            pub_.publish(ObservabilityEvent::Counter {
                name: "riftgate_test_counter",
                value: i,
                labels: Labels::new(),
            });
        }
        // Give the worker a moment to drain.
        for _ in 0..50 {
            if sink.count() == 5 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sink.count(), 5);
        assert_eq!(bus.dropped_total(), 0);
    }

    /// A blocking sink that holds onto the worker so the bus channel
    /// fills up. Used to verify drop-on-full.
    struct BlockingSink {
        block: std::sync::Mutex<()>,
    }

    impl BlockingSink {
        fn new() -> Self {
            Self {
                block: std::sync::Mutex::new(()),
            }
        }
    }

    impl ObservabilitySink for BlockingSink {
        fn publish(&self, _event: ObservabilityEvent) {
            // Acquire the mutex held by the test to block forever.
            let _g = self.block.lock();
            // Once we got it, block "forever" (long enough for the test
            // to fill the bus).
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    #[test]
    fn bus_drops_when_channel_full() {
        let sink = Arc::new(BlockingSink::new());
        // Hold the mutex so the worker blocks on its first publish.
        let _hold = sink.block.lock().unwrap();
        let bus = Bus::new(4, sink.clone());
        let pub_ = bus.publisher();

        // Push 4 events: the first one pulls into the worker (blocked),
        // 3 more fit in the channel. The 5th drops.
        for i in 0..1000 {
            pub_.publish(ObservabilityEvent::Counter {
                name: "riftgate_test_counter",
                value: i,
                labels: Labels::new(),
            });
        }
        // Some events were dropped (we sent 1000 into a bus of capacity 4
        // with a blocked worker).
        assert!(
            bus.dropped_total() > 0,
            "expected at least one dropped event, got {}",
            bus.dropped_total()
        );
    }
}
