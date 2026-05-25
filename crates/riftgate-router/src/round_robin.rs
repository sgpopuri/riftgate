//! Round-robin router.
//!
//! Maintains a single `AtomicUsize` cursor; on each `route` call, the
//! cursor is fetched-and-incremented (`Relaxed` ordering) and used as
//! the index into the [`BackendPool`]
//! modulo the pool size.
//!
//! ```text
//!   pool: [b0, b1, b2]
//!   cursor: 0 -> route returns b0, cursor -> 1
//!   cursor: 1 -> route returns b1, cursor -> 2
//!   cursor: 2 -> route returns b2, cursor -> 3
//!   cursor: 3 -> route returns b0, cursor -> 4   (3 % 3 == 0)
//! ```
//!
//! `Relaxed` ordering is sufficient: perfect monotonicity is not
//! required, only fair distribution over time. The statistical fairness
//! test in `tests/fairness.rs` asserts the distribution stays within
//! ±10 of perfect across 3000 requests on 3 backends.

use core::sync::atomic::{AtomicUsize, Ordering};
use riftgate_core::request::{Request, StatusCode};
use riftgate_core::router::{BackendPool, BackendSignals, Router, RoutingDecision};

/// Round-robin router. See module-level docs for details.
#[derive(Debug, Default)]
pub struct RoundRobinRouter {
    cursor: AtomicUsize,
}

impl RoundRobinRouter {
    /// Construct a new `RoundRobinRouter` with the cursor starting at 0.
    pub const fn new() -> Self {
        Self {
            cursor: AtomicUsize::new(0),
        }
    }
}

impl Router for RoundRobinRouter {
    fn route(
        &self,
        _req: &Request,
        pool: &BackendPool,
        _signals: &BackendSignals,
    ) -> RoutingDecision {
        if pool.is_empty() {
            return RoutingDecision::Reject(StatusCode::BAD_GATEWAY);
        }
        // fetch_add wraps on overflow which is fine (modulo pool length
        // gives the same result regardless of cursor's absolute value).
        let cursor = self.cursor.fetch_add(1, Ordering::Relaxed);
        // `pool.is_empty()` was checked above; `get_modulo` returns
        // `Some` for any non-empty pool.
        let backend = pool
            .get_modulo(cursor)
            .expect("get_modulo on a non-empty pool always returns Some");
        RoutingDecision::Send(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::request::{Body, Headers, Method};
    use riftgate_core::router::BackendId;
    use riftgate_core::types::RequestId;

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
    fn empty_pool_rejects_with_bad_gateway() {
        let r = RoundRobinRouter::new();
        let pool = BackendPool::new();
        let signals = BackendSignals::new();
        match r.route(&dummy_request(), &pool, &signals) {
            RoutingDecision::Reject(s) => assert_eq!(s, StatusCode::BAD_GATEWAY),
            other => panic!("expected Reject(BAD_GATEWAY), got {other:?}"),
        }
    }

    #[test]
    fn picks_each_backend_in_order() {
        let r = RoundRobinRouter::new();
        let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1), BackendId(2)]);
        let signals = BackendSignals::new();
        let mut sequence = Vec::new();
        for _ in 0..6 {
            if let RoutingDecision::Send(b) = r.route(&dummy_request(), &pool, &signals) {
                sequence.push(b);
            }
        }
        assert_eq!(
            sequence,
            vec![
                BackendId(0),
                BackendId(1),
                BackendId(2),
                BackendId(0),
                BackendId(1),
                BackendId(2),
            ]
        );
    }
}
