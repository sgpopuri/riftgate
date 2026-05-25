//! The request handler.
//!
//! This is the core of the v0.1 walking-skeleton binary: every inbound
//! request lands here.
//!
//! ```text
//!     hyper::server         <─── req: Request<Incoming>
//!         │
//!         v
//!     proxy::handle ── /health, /ready short-circuit
//!         │
//!         v   per-request BumpArena (FR-007), per-request RequestId
//!         │
//!         v   parse JSON body if /v1/chat/completions (FR-002)
//!         │
//!         v   router.route(...) -> BackendId (FR-003)
//!         │
//!         v   upstream.request(...).timeout(...) (FR-005 timeout)
//!         │
//!         v   forward response with first-token observation (FR-004 + FR-006)
//!         │
//!         v   spans: received -> queued -> dispatched -> first_token -> completed
//! ```
//!
//! Spans emitted, in order:
//!
//! - [`spans::REQUEST_RECEIVED`](riftgate_obs::spans::REQUEST_RECEIVED)
//! - [`spans::REQUEST_QUEUED`](riftgate_obs::spans::REQUEST_QUEUED)
//! - [`spans::REQUEST_DISPATCHED`](riftgate_obs::spans::REQUEST_DISPATCHED)
//! - [`spans::REQUEST_FIRST_TOKEN`](riftgate_obs::spans::REQUEST_FIRST_TOKEN)
//!   (on first non-empty response data frame)
//! - [`spans::REQUEST_COMPLETED`](riftgate_obs::spans::REQUEST_COMPLETED)
//!   (on body end-of-stream)

use crate::health;
use crate::shutdown::DrainReceiver;
use crate::upstream::{UpstreamClient, UpstreamReqBody};
use bumpalo::Bump;
use bytes::Bytes;
use http::{HeaderName, HeaderValue, Method, Response, StatusCode, Uri};
use http_body::{Body, Frame};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use pin_project::{pin_project, pinned_drop};
use riftgate_config::Config;
use riftgate_core::router::{BackendPool, BackendSignals, Router, RoutingDecision};
use riftgate_core::types::RequestId;
use riftgate_obs::Publisher;
use riftgate_obs::spans;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Per-process state shared across every request.
///
/// Cheap to clone (one `Arc` per field); held by the per-connection
/// service closure.
#[derive(Clone)]
pub struct HandlerState {
    /// Loaded configuration. Hot reload is v0.2+; in v0.1 the binary
    /// holds an `Arc<Config>` for the lifetime of the process.
    pub config: Arc<Config>,
    /// Active router. Trait object so the binary can swap router impls
    /// via configuration in v0.2.
    pub router: Arc<dyn Router>,
    /// Configured backend pool.
    pub pool: Arc<BackendPool>,
    /// Live per-backend signals. v0.1 uses an empty signal set; v0.2
    /// updates it from `Router::on_response`.
    pub signals: Arc<BackendSignals>,
    /// Hyper-rustls upstream client.
    pub upstream: UpstreamClient,
    /// Observability publisher (drop-on-full bus).
    pub publisher: Publisher,
    /// Drain signal — used by the `/ready` handler.
    pub drain: DrainReceiver,
}

/// Body type returned by [`handle`]. Boxed so streaming and
/// non-streaming bodies coexist.
pub type ResponseBody = BoxBody<Bytes, hyper::Error>;

/// Top-level request handler. Always returns `Ok`; per-request errors
/// are encoded as HTTP error responses.
///
/// # Errors
/// Returns [`Infallible`] (alias for `!`); the signature is
/// `Result<_, Infallible>` because hyper's `service_fn` expects a
/// `Result`.
pub async fn handle(
    req: hyper::Request<Incoming>,
    state: HandlerState,
) -> Result<Response<ResponseBody>, Infallible> {
    let request_id = RequestId::next();
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();

    publish_span(
        &state.publisher,
        request_id,
        spans::REQUEST_RECEIVED,
        Duration::ZERO,
    );

    if method == Method::GET {
        if path == "/health" {
            return Ok(into_box(health::health()));
        }
        if path == "/ready" {
            return Ok(into_box(health::ready(&state.drain)));
        }
    }

    // Per-request arena (FR-007). Allocations made through this arena
    // are reclaimed when the handler returns and `_arena` is dropped.
    // We do not yet wire the parser through it; the trait surface is
    // exercised in `riftgate-core::allocator`'s tests.
    let _arena = Bump::with_capacity(8 * 1024);

    // Read the request body. v0.1 does not stream request bodies;
    // chat/completions JSON fits comfortably in memory.
    let (parts, body) = req.into_parts();
    let body_bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                &format!("body read error: {e}"),
            ));
        }
    };

    // FR-002: validate JSON for the chat/completions path.
    if method == Method::POST
        && path == "/v1/chat/completions"
        && !body_bytes.is_empty()
        && serde_json::from_slice::<serde_json::Value>(&body_bytes).is_err()
    {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "request body is not valid JSON",
        ));
    }

    // Routing. The v0.1 binary builds a minimal `riftgate-core` Request
    // for the router; the router only inspects the method and path
    // today, but the trait surface is forward-compatible.
    let decision = state.router.route(
        &build_core_request(request_id, &method, &path),
        state.pool.as_ref(),
        state.signals.as_ref(),
    );

    let backend_id = match decision {
        RoutingDecision::Send(id) => id,
        RoutingDecision::Reject(s) => {
            publish_span(
                &state.publisher,
                request_id,
                spans::REQUEST_REJECTED,
                started.elapsed(),
            );
            let status = StatusCode::from_u16(s.0).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return Ok(error_response(status, "router rejected the request"));
        }
        RoutingDecision::Hedge(_) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "hedged routing lands in v0.3 (see Options 010)",
            ));
        }
    };

    publish_span(
        &state.publisher,
        request_id,
        spans::REQUEST_QUEUED,
        started.elapsed(),
    );

    // Build the upstream request.
    let upstream_uri = match build_upstream_uri(&state.config.backend.url, &path, parts.uri.query())
    {
        Ok(u) => u,
        Err(msg) => {
            return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, &msg));
        }
    };

    let mut upstream_req = hyper::Request::builder()
        .method(method.clone())
        .uri(upstream_uri);

    // Forward inbound headers, except hop-by-hop and Host.
    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name) || name == http::header::HOST {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
    }

    // Inject the configured auth header.
    let auth = state.config.backend.auth_header.expose();
    if !auth.is_empty() {
        if let Ok(v) = HeaderValue::from_str(auth) {
            upstream_req = upstream_req.header(http::header::AUTHORIZATION, v);
        }
    }

    // Identify ourselves. Useful for upstream logs.
    upstream_req = upstream_req.header(
        HeaderName::from_static("x-riftgate-request-id"),
        HeaderValue::from_str(&request_id.to_string()).expect("request id is ASCII-safe"),
    );
    upstream_req = upstream_req.header(
        HeaderName::from_static("x-riftgate-backend"),
        HeaderValue::from_str(&backend_id.to_string()).expect("backend id is ASCII-safe"),
    );

    let upstream_req: hyper::Request<UpstreamReqBody> = upstream_req
        .body(Full::new(body_bytes))
        .expect("upstream request build should never fail");

    publish_span(
        &state.publisher,
        request_id,
        spans::REQUEST_DISPATCHED,
        started.elapsed(),
    );

    // Per-request timeout (FR-005).
    let timeout = Duration::from_millis(u64::from(state.config.backend.timeout_ms));
    let dispatch_started = Instant::now();
    let upstream_resp =
        match tokio::time::timeout(timeout, state.upstream.request(upstream_req)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("upstream error: {e}"),
                ));
            }
            Err(_elapsed) => {
                publish_span(
                    &state.publisher,
                    request_id,
                    spans::REQUEST_COMPLETED,
                    started.elapsed(),
                );
                return Ok(error_response(
                    StatusCode::GATEWAY_TIMEOUT,
                    "upstream did not respond before deadline",
                ));
            }
        };

    let _ = dispatch_started; // bound for clarity / future per-span emission.

    // Build the response back to the client. We forward status and
    // (most) headers verbatim.
    let (resp_parts, resp_body) = upstream_resp.into_parts();
    let mut builder = Response::builder().status(resp_parts.status);
    for (name, value) in resp_parts.headers.iter() {
        if is_hop_by_hop(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder = builder
        .header(
            HeaderName::from_static("x-riftgate-request-id"),
            HeaderValue::from_str(&request_id.to_string()).expect("request id is ASCII-safe"),
        )
        .header(
            HeaderName::from_static("x-riftgate-backend"),
            HeaderValue::from_str(&backend_id.to_string()).expect("backend id is ASCII-safe"),
        );

    let observed = ObservedBody {
        inner: resp_body,
        first_seen: false,
        completed: false,
        publisher: state.publisher.clone(),
        request_id,
        started,
    };

    let body: ResponseBody = observed.boxed();
    Ok(builder
        .body(body)
        .expect("response build should never fail"))
}

/// Build the riftgate-core `Request` that the router consumes. v0.1
/// only inspects method + path; we leave headers and body empty to
/// avoid a redundant copy on the hot path.
fn build_core_request(
    id: RequestId,
    method: &Method,
    path: &str,
) -> riftgate_core::request::Request {
    use riftgate_core::request::{Body as CoreBody, Headers, Method as CoreMethod};
    let m = match *method {
        Method::GET => CoreMethod::Get,
        Method::POST => CoreMethod::Post,
        Method::PUT => CoreMethod::Put,
        Method::DELETE => CoreMethod::Delete,
        Method::OPTIONS => CoreMethod::Options,
        Method::HEAD => CoreMethod::Head,
        ref other => CoreMethod::Other(other.to_string()),
    };
    riftgate_core::request::Request {
        id,
        method: m,
        path: path.to_owned(),
        headers: Headers::new(),
        body: CoreBody::Empty,
    }
}

/// Compose `<backend>/<path>?<query>` into a parsed `Uri`.
fn build_upstream_uri(backend_url: &str, path: &str, query: Option<&str>) -> Result<Uri, String> {
    let trimmed = backend_url.trim_end_matches('/');
    let combined = match query {
        Some(q) => format!("{trimmed}{path}?{q}"),
        None => format!("{trimmed}{path}"),
    };
    combined
        .parse::<Uri>()
        .map_err(|e| format!("invalid upstream URI `{combined}`: {e}"))
}

/// Hop-by-hop headers per RFC 7230 §6.1. We strip these on both
/// inbound (request → upstream) and outbound (upstream → client)
/// because forwarding them across a proxy is incorrect.
fn is_hop_by_hop(name: &http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Convert a `Response<Full<Bytes>>` into the boxed response body type.
fn into_box(resp: Response<Full<Bytes>>) -> Response<ResponseBody> {
    let (parts, body) = resp.into_parts();
    let boxed: ResponseBody = body.map_err(|never: Infallible| match never {}).boxed();
    Response::from_parts(parts, boxed)
}

/// Build a small text/plain error response with the boxed body type.
fn error_response(status: StatusCode, message: &str) -> Response<ResponseBody> {
    let body = Full::new(Bytes::from(format!("{message}\n")))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("x-riftgate-error", "1")
        .body(body)
        .expect("static error body should always build")
}

fn publish_span(publisher: &Publisher, id: RequestId, name: &'static str, duration: Duration) {
    publisher.publish(riftgate_core::obs::ObservabilityEvent::SpanEnd {
        request_id: id,
        name,
        duration,
    });
}

/// Body wrapper that observes the first non-empty data frame and the
/// end-of-stream so the proxy can emit `request.first_token` and
/// `request.completed` spans without buffering the response.
///
/// `request.completed` is emitted in [`PinnedDropImpl`] so the span
/// fires regardless of who terminates the body (full read, client
/// disconnect, error mid-stream).
#[pin_project(PinnedDrop)]
struct ObservedBody {
    #[pin]
    inner: Incoming,
    first_seen: bool,
    completed: bool,
    publisher: Publisher,
    request_id: RequestId,
    started: Instant,
}

impl Body for ObservedBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        let result = this.inner.poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &result {
            if !*this.first_seen {
                if let Some(data) = frame.data_ref() {
                    if !data.is_empty() {
                        *this.first_seen = true;
                        this.publisher
                            .publish(riftgate_core::obs::ObservabilityEvent::SpanEnd {
                                request_id: *this.request_id,
                                name: spans::REQUEST_FIRST_TOKEN,
                                duration: this.started.elapsed(),
                            });
                    }
                }
            }
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

#[pinned_drop]
impl PinnedDrop for ObservedBody {
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();
        if !*this.completed {
            *this.completed = true;
            this.publisher
                .publish(riftgate_core::obs::ObservabilityEvent::SpanEnd {
                    request_id: *this.request_id,
                    name: spans::REQUEST_COMPLETED,
                    duration: this.started.elapsed(),
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_upstream_uri_appends_path() {
        let u = build_upstream_uri("http://api.example.com", "/v1/chat/completions", None).unwrap();
        assert_eq!(u.to_string(), "http://api.example.com/v1/chat/completions");
    }

    #[test]
    fn build_upstream_uri_keeps_query() {
        let u = build_upstream_uri("http://api.example.com/", "/v1/models", Some("filter=gpt"))
            .unwrap();
        assert_eq!(u.to_string(), "http://api.example.com/v1/models?filter=gpt");
    }

    #[test]
    fn build_upstream_uri_rejects_invalid() {
        assert!(build_upstream_uri("not a url", "/x", None).is_err());
    }

    #[test]
    fn hop_by_hop_set_matches_rfc_7230() {
        let h = |s: &str| is_hop_by_hop(&http::HeaderName::from_bytes(s.as_bytes()).unwrap());
        assert!(h("connection"));
        assert!(h("transfer-encoding"));
        assert!(h("upgrade"));
        assert!(!h("content-type"));
        assert!(!h("authorization"));
    }
}
