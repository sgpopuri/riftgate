//! Filter trait + `IdentityFilter` and `LoggingFilter` impls.
//!
//! Filters transform requests on the way in and responses on the way out.
//! The full WASM filter chain (per [Options 016 — extension mechanism], to
//! be authored when v0.3 lands) consumes this trait via `wasmtime`; native
//! filters compile against this trait directly.
//!
//! See [`docs/03-architecture/extension-plane.md`](../../../docs/03-architecture/extension-plane.md)
//! for the extension contract.

use crate::request::{Request, Response, StatusCode};

/// Outcome of a filter call.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FilterAction {
    /// Continue processing; the (possibly modified) request or response
    /// flows downstream unchanged in shape.
    Continue,
    /// Short-circuit: terminate this request with the given status. Useful
    /// for PII redaction filters that detect a forbidden pattern, or for
    /// cost-guard filters that reject before dispatch.
    Terminate(StatusCode),
}

/// Filter trait.
///
/// Filters see typed `Request` and `Response` objects (not raw bytes); the
/// filter chain executor (lands in `riftgate-filter` in `v0.3`) drives the
/// chain in order on the request side and reverse order on the response
/// side.
///
/// **`Send + Sync`** because filter chains are shared across shards via
/// `Arc`. Filters that need per-request state allocate it from the
/// per-request arena and pass it via the `Request` / `Response` types
/// rather than holding it in `&self`.
///
/// Trait object safety: yes.
pub trait Filter: Send + Sync {
    /// Inspect or modify a request before it is dispatched.
    fn on_request(&self, req: &mut Request) -> FilterAction;

    /// Inspect or modify a response before it is sent to the client.
    ///
    /// Default: pass-through. Filters that only care about requests can
    /// skip implementing this method.
    fn on_response(&self, _resp: &mut Response) -> FilterAction {
        FilterAction::Continue
    }
}

/// Pass-through filter that does nothing.
///
/// Useful as a default in tests and as the v0.1 placeholder when no filter
/// chain is configured.
#[derive(Debug, Default, Copy, Clone)]
pub struct IdentityFilter;

impl IdentityFilter {
    /// Construct an `IdentityFilter`. Zero cost.
    pub const fn new() -> Self {
        Self
    }
}

impl Filter for IdentityFilter {
    fn on_request(&self, _req: &mut Request) -> FilterAction {
        FilterAction::Continue
    }
}

/// Filter that logs every request and response at `tracing::debug` level.
///
/// FR-X02 second impl alongside [`IdentityFilter`]. Useful as a development
/// aid and as a smoke test that the filter chain executor is wiring filters
/// in the right order.
#[derive(Debug, Default, Copy, Clone)]
pub struct LoggingFilter;

impl LoggingFilter {
    /// Construct a `LoggingFilter`. Zero cost.
    pub const fn new() -> Self {
        Self
    }
}

impl Filter for LoggingFilter {
    fn on_request(&self, req: &mut Request) -> FilterAction {
        tracing::debug!(
            request_id = %req.id,
            method = ?req.method,
            path = %req.path,
            header_count = req.headers.len(),
            body_bytes = req.body.len(),
            "filter: request"
        );
        FilterAction::Continue
    }

    fn on_response(&self, resp: &mut Response) -> FilterAction {
        tracing::debug!(
            request_id = %resp.id,
            status = resp.status.0,
            header_count = resp.headers.len(),
            body_bytes = resp.body.len(),
            "filter: response"
        );
        FilterAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Body, Headers, Method};
    use crate::types::RequestId;

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
    fn identity_filter_continues() {
        let f = IdentityFilter::new();
        let mut req = dummy_request();
        assert_eq!(f.on_request(&mut req), FilterAction::Continue);
    }

    #[test]
    fn filter_is_dyn_safe() {
        let _f: Box<dyn Filter> = Box::new(IdentityFilter::new());
        let _g: Box<dyn Filter> = Box::new(LoggingFilter::new());
    }
}
