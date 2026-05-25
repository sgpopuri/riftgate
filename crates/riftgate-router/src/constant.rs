//! Constant router. FR-X02 second impl alongside [`crate::RoundRobinRouter`].
//!
//! Always returns the same backend id. Useful as a starting point for
//! tests of caller code that needs a `Router` but doesn't need to
//! exercise routing logic.

use riftgate_core::request::Request;
use riftgate_core::router::{BackendId, BackendPool, BackendSignals, Router, RoutingDecision};

/// Always-the-same-backend router.
///
/// Construct with the desired `BackendId`; subsequent `route` calls
/// always return [`RoutingDecision::Send`] of that id, even if the
/// backend pool is empty (the bin is responsible for ensuring the id
/// resolves).
#[derive(Debug, Copy, Clone)]
pub struct ConstantRouter(pub BackendId);

impl ConstantRouter {
    /// Construct a `ConstantRouter` that always picks `id`.
    pub const fn new(id: BackendId) -> Self {
        Self(id)
    }
}

impl Router for ConstantRouter {
    fn route(
        &self,
        _req: &Request,
        _pool: &BackendPool,
        _signals: &BackendSignals,
    ) -> RoutingDecision {
        RoutingDecision::Send(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::request::{Body, Headers, Method};
    use riftgate_core::types::RequestId;

    fn dummy_request() -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/".into(),
            headers: Headers::new(),
            body: Body::Empty,
        }
    }

    #[test]
    fn always_returns_constructed_id() {
        let r = ConstantRouter::new(BackendId(7));
        let pool = BackendPool::new();
        let signals = BackendSignals::new();
        for _ in 0..10 {
            match r.route(&dummy_request(), &pool, &signals) {
                RoutingDecision::Send(b) => assert_eq!(b, BackendId(7)),
                other => panic!("ConstantRouter should always Send, got {other:?}"),
            }
        }
    }
}
