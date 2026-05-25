//! Statistical fairness tests for `RoundRobinRouter`.
//!
//! 3000 requests over 3 backends should produce ~1000 per backend.
//! Allow a tolerance of ±10 — the cursor uses `Relaxed` ordering and
//! perfect monotonicity is not required, but in this single-threaded
//! sequential test the count should be exactly 1000.
//!
//! A second test exercises the same router from multiple threads and
//! asserts that the per-backend count is still within ±10% of perfect.

use riftgate_core::request::{Body, Headers, Method, Request};
use riftgate_core::router::{BackendId, BackendPool, BackendSignals, Router, RoutingDecision};
use riftgate_core::types::RequestId;
use riftgate_router::RoundRobinRouter;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

fn dummy_request() -> Request {
    Request {
        id: RequestId(1),
        method: Method::Post,
        path: "/v1/chat/completions".into(),
        headers: Headers::new(),
        body: Body::Empty,
    }
}

#[test]
fn single_threaded_perfect_distribution() {
    let r = RoundRobinRouter::new();
    let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1), BackendId(2)]);
    let signals = BackendSignals::new();
    let mut counts: HashMap<BackendId, usize> = HashMap::new();
    for _ in 0..3000 {
        if let RoutingDecision::Send(b) = r.route(&dummy_request(), &pool, &signals) {
            *counts.entry(b).or_insert(0) += 1;
        }
    }
    for id in [BackendId(0), BackendId(1), BackendId(2)] {
        assert_eq!(
            counts.get(&id).copied().unwrap_or(0),
            1000,
            "single-threaded round-robin should hit each backend exactly 1000/3000 times"
        );
    }
}

#[test]
fn multi_threaded_fair_distribution() {
    let r = Arc::new(RoundRobinRouter::new());
    let pool = Arc::new(BackendPool::from_ids(vec![
        BackendId(0),
        BackendId(1),
        BackendId(2),
    ]));
    let signals = Arc::new(BackendSignals::new());
    let counts: Arc<[AtomicUsize; 3]> = Arc::new([
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ]);

    let n_threads = 4;
    let per_thread = 750;
    let total = n_threads * per_thread;

    let mut handles = Vec::new();
    for _ in 0..n_threads {
        let r = Arc::clone(&r);
        let pool = Arc::clone(&pool);
        let signals = Arc::clone(&signals);
        let counts = Arc::clone(&counts);
        handles.push(thread::spawn(move || {
            for _ in 0..per_thread {
                if let RoutingDecision::Send(b) = r.route(&dummy_request(), &pool, &signals) {
                    counts[b.0 as usize].fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let expected = total / 3;
    let tolerance = (expected as f64 * 0.1) as usize;
    for (i, c) in counts.iter().enumerate() {
        let observed = c.load(Ordering::Relaxed);
        let diff = observed.abs_diff(expected);
        assert!(
            diff <= tolerance,
            "backend {i} count {observed} drifted from expected {expected} by {diff} (>{tolerance})"
        );
    }
}
