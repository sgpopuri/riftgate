//! Native filter-chain executor.

use riftgate_core::{Filter, FilterAction, Request, Response};
use std::sync::Arc;

/// In-order filter-chain executor.
///
/// The chain holds an immutable `Vec<Arc<dyn Filter>>`. Adding or removing
/// filters requires rebuilding the chain (which is a config-reload-time
/// activity, not a hot-path activity). Cloning a `FilterChain` is cheap —
/// it clones the inner `Arc` slice only.
///
/// **Trait shape:** `FilterChain` itself implements
/// [`riftgate_core::Filter`], so a single `Arc<FilterChain>` can be passed
/// to any code that expects a single `Filter`.
#[derive(Clone, Default)]
pub struct FilterChain {
    filters: Vec<Arc<dyn Filter>>,
}

impl core::fmt::Debug for FilterChain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterChain")
            .field("filter_count", &self.filters.len())
            .finish()
    }
}

impl FilterChain {
    /// Construct an empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// Construct a chain from an existing vector.
    #[must_use]
    pub fn from_filters(filters: Vec<Arc<dyn Filter>>) -> Self {
        Self { filters }
    }

    /// Append a filter to the end of the chain. Returns the new length.
    pub fn push(&mut self, filter: Arc<dyn Filter>) -> usize {
        self.filters.push(filter);
        self.filters.len()
    }

    /// Number of filters in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// `true` if the chain is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Filter for FilterChain {
    fn on_request(&self, req: &mut Request) -> FilterAction {
        for f in &self.filters {
            match f.on_request(req) {
                FilterAction::Continue => continue,
                terminate @ FilterAction::Terminate(_) => return terminate,
            }
        }
        FilterAction::Continue
    }

    fn on_response(&self, resp: &mut Response) -> FilterAction {
        // Reverse order on the response side — canonical filter-chain
        // shape; matches Envoy / Linkerd / Spin.
        for f in self.filters.iter().rev() {
            match f.on_response(resp) {
                FilterAction::Continue => continue,
                terminate @ FilterAction::Terminate(_) => return terminate,
            }
        }
        FilterAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::types::RequestId;
    use riftgate_core::{Body, Headers, IdentityFilter, LoggingFilter, Method, StatusCode};

    fn dummy_request() -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/v1/chat/completions".to_string(),
            headers: Headers::new(),
            body: Body::Bytes(b"hello".to_vec()),
        }
    }

    fn dummy_response() -> Response {
        Response {
            id: RequestId(1),
            status: StatusCode(200),
            headers: Headers::new(),
            body: Body::Bytes(b"world".to_vec()),
        }
    }

    /// Counts how many times on_request / on_response are called and in
    /// what order across multiple filter instances.
    #[derive(Debug, Default)]
    struct Tracer {
        label: &'static str,
        log: std::sync::Mutex<Vec<&'static str>>,
    }

    impl Filter for Tracer {
        fn on_request(&self, _req: &mut Request) -> FilterAction {
            self.log.lock().expect("tracer log").push(self.label);
            FilterAction::Continue
        }
        fn on_response(&self, _resp: &mut Response) -> FilterAction {
            self.log.lock().expect("tracer log").push(self.label);
            FilterAction::Continue
        }
    }

    #[test]
    fn empty_chain_passes_through() {
        let chain = FilterChain::new();
        let mut req = dummy_request();
        assert_eq!(chain.on_request(&mut req), FilterAction::Continue);
        let mut resp = dummy_response();
        assert_eq!(chain.on_response(&mut resp), FilterAction::Continue);
    }

    #[test]
    fn chain_runs_identity_and_logging_filters() {
        let chain = FilterChain::from_filters(vec![
            Arc::new(IdentityFilter::new()),
            Arc::new(LoggingFilter::new()),
        ]);
        assert_eq!(chain.len(), 2);
        let mut req = dummy_request();
        assert_eq!(chain.on_request(&mut req), FilterAction::Continue);
    }

    #[test]
    fn chain_terminates_short_circuits() {
        struct Reject;
        impl Filter for Reject {
            fn on_request(&self, _req: &mut Request) -> FilterAction {
                FilterAction::Terminate(StatusCode(403))
            }
        }
        let chain = FilterChain::from_filters(vec![Arc::new(Reject)]);
        let mut req = dummy_request();
        assert_eq!(
            chain.on_request(&mut req),
            FilterAction::Terminate(StatusCode(403))
        );
    }

    #[test]
    fn response_order_is_reversed() {
        let t1 = Arc::new(Tracer {
            label: "t1",
            log: Default::default(),
        });
        let t2 = Arc::new(Tracer {
            label: "t2",
            log: Default::default(),
        });
        let chain = FilterChain::from_filters(vec![t1.clone(), t2.clone()]);
        let mut req = dummy_request();
        let mut resp = dummy_response();
        let _ = chain.on_request(&mut req);
        let _ = chain.on_response(&mut resp);
        // Each tracer logs into its own buffer; the response-side traversal
        // calls them in reverse order, so t2 logs its response visit first
        // and t1 second. Concatenating both buffers gives the full path.
        let req_visits: Vec<&'static str> = t1.log.lock().unwrap().to_vec();
        assert_eq!(req_visits, vec!["t1", "t1"]); // on_request then on_response
        let req_visits2: Vec<&'static str> = t2.log.lock().unwrap().to_vec();
        assert_eq!(req_visits2, vec!["t2", "t2"]);
    }
}
