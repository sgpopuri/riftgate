//! End-to-end tests for the riftgate v0.1 binary.
//!
//! Each test stands up:
//!
//! 1. A mock upstream HTTP server (a tiny hyper service) that returns
//!    canned responses (JSON or SSE).
//! 2. A real `riftgate` data-plane stack: the same `HandlerState` the
//!    binary builds in `main.rs`, wired against the mock upstream.
//! 3. A hyper client that hits the gateway over a real TCP socket.
//!
//! The tests assert the FR-001..FR-008 acceptance criteria. We do not
//! launch the `riftgate` binary as a subprocess; we exercise the
//! library modules directly so an `InMemorySink` can capture spans.

use arc_swap::ArcSwap;
use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1 as server_http1;
use hyper::service::service_fn;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use riftgate::{proxy, server, shutdown, upstream as gw_upstream};
use riftgate_core::router::{BackendId, BackendPool, BackendSignals};
use riftgate_obs::Bus;
use riftgate_router::RoundRobinRouter;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

type UpstreamRespBody = BoxBody<Bytes, Infallible>;

/// Mock upstream router. Returns:
///
/// - `POST /v1/chat/completions` with `"stream":true` -> SSE stream
/// - `POST /v1/chat/completions` (no stream) -> JSON
/// - `GET /v1/models` -> JSON list
/// - everything else -> 404
async fn upstream_router(
    req: Request<hyper::body::Incoming>,
) -> Result<http::Response<UpstreamRespBody>, Infallible> {
    let path = req.uri().path().to_owned();
    let method = req.method().clone();

    if method == http::Method::POST && path == "/v1/chat/completions" {
        let body = req
            .collect()
            .await
            .map(|c| c.to_bytes())
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        let is_stream = v
            .get("stream")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if is_stream {
            let chunks: Vec<Bytes> = vec![
                Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"),
                Bytes::from_static(
                    b"data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
                ),
                Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n\n"),
                Bytes::from_static(b"data: [DONE]\n\n"),
            ];
            let stream = futures_util::stream::iter(
                chunks
                    .into_iter()
                    .map(|b| Ok::<_, Infallible>(Frame::data(b))),
            );
            let body = StreamBody::new(stream).boxed();
            return Ok(http::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(body)
                .unwrap());
        }

        let body = Bytes::from_static(
            br#"{"id":"chatcmpl-test","object":"chat.completion","model":"gpt-4o-mini","choices":[{"message":{"role":"assistant","content":"hello back"}}]}"#,
        );
        let body = Full::new(body)
            .map_err(|never: Infallible| match never {})
            .boxed();
        return Ok(http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(body)
            .unwrap());
    }

    if method == http::Method::GET && path == "/v1/models" {
        let body = Bytes::from_static(br#"{"data":[{"id":"gpt-4o-mini"}]}"#);
        let body = Full::new(body)
            .map_err(|never: Infallible| match never {})
            .boxed();
        return Ok(http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(body)
            .unwrap());
    }

    let body = Full::new(Bytes::from_static(b"not found"))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Ok(http::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(body)
        .unwrap())
}

/// Spawn the mock upstream and return its address + a stop signal.
async fn spawn_mock_upstream() -> (SocketAddr, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let addr = listener.local_addr().expect("local_addr");
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let io = TokioIo::new(stream);
                            tokio::spawn(async move {
                                let svc = service_fn(upstream_router);
                                let _ = server_http1::Builder::new()
                                    .serve_connection(io, svc)
                                    .await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    (addr, stop_tx)
}

/// Stand up a riftgate gateway against the supplied upstream. Returns
/// the gateway's bound address and the in-memory observability sink.
async fn spawn_riftgate(
    upstream_addr: SocketAddr,
    backend_timeout_ms: u32,
) -> (
    SocketAddr,
    Arc<riftgate_core::obs::InMemorySink>,
    shutdown::DrainSender,
) {
    let mut config = riftgate_config::Config::default();
    config.server.listen_addr = "127.0.0.1:0".parse().unwrap();
    config.backend.url = format!("http://{upstream_addr}");
    config.backend.timeout_ms = backend_timeout_ms;
    config.obs.bus_capacity = 256;
    let config = Arc::new(config);

    let sink = Arc::new(riftgate_core::obs::InMemorySink::new());
    let sink_dyn: Arc<dyn riftgate_core::obs::ObservabilitySink> = sink.clone();
    let bus = Bus::new(config.obs.bus_capacity, sink_dyn);
    let publisher = bus.publisher();
    // Hold the bus for the lifetime of the test process by leaking it;
    // the worker thread keeps draining events into the sink.
    Box::leak(Box::new(bus));

    let upstream_client = gw_upstream::build_client();
    let pool = Arc::new(BackendPool::from_ids(vec![BackendId(0)]));
    let signals = Arc::new(ArcSwap::from_pointee(BackendSignals::new()));
    let router: Arc<dyn riftgate_core::router::Router> = Arc::new(RoundRobinRouter::new());

    let (drain_tx, drain_rx) = shutdown::channel();
    let state = proxy::HandlerState {
        config: config.clone(),
        router,
        pool,
        signals,
        upstream: upstream_client,
        publisher,
        drain: drain_rx.clone(),
        mcp_broker: None,
        tenant_resolver: std::sync::Arc::new(riftgate_core::tenant::HeaderTenantResolver),
    };

    let listener = server::bind(config.server.listen_addr)
        .await
        .expect("gateway bind");
    let bound = listener.local_addr().expect("gateway local_addr");
    tokio::spawn(async move {
        let _ = server::accept_loop(listener, state, drain_rx, Duration::from_secs(1)).await;
    });

    (bound, sink, drain_tx)
}

fn http_client() -> Client<hyper_util::client::legacy::connect::HttpConnector, Full<Bytes>> {
    Client::builder(TokioExecutor::new()).build_http()
}

fn empty_client() -> Client<hyper_util::client::legacy::connect::HttpConnector, Empty<Bytes>> {
    Client::builder(TokioExecutor::new()).build_http()
}

async fn read_body(resp: http::Response<hyper::body::Incoming>) -> (http::response::Parts, Bytes) {
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.expect("collect body").to_bytes();
    (parts, bytes)
}

#[tokio::test]
async fn fr_001_accepts_http1_requests_on_configured_port() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = empty_client();
    let req = Request::builder()
        .method("GET")
        .uri(format!("http://{gateway}/health"))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::OK);
    let (_p, body) = read_body(resp).await;
    assert_eq!(&body[..], b"OK\n");
}

#[tokio::test]
async fn fr_002_rejects_malformed_json_on_chat_completions() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = http_client();
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://{gateway}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(b"{ this is not json }")))
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fr_002_accepts_well_formed_chat_completions_request() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = http_client();
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://{gateway}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(
            br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#,
        )))
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::OK);
    let (parts, body) = read_body(resp).await;
    assert_eq!(
        parts
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("application/json")
    );
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "hello back");
}

#[tokio::test]
async fn fr_003_forwards_to_configured_upstream() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = empty_client();
    let req = Request::builder()
        .method("GET")
        .uri(format!("http://{gateway}/v1/models"))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::OK);
    let (parts, body) = read_body(resp).await;
    let request_id = parts.headers.get("x-riftgate-request-id");
    assert!(
        request_id.is_some(),
        "gateway should set x-riftgate-request-id"
    );
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["data"][0]["id"], "gpt-4o-mini");
}

#[tokio::test]
async fn fr_004_streams_sse_response_chunks() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = http_client();
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://{gateway}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(
            br#"{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        )))
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("text/event-stream")
    );
    let (_parts, body) = read_body(resp).await;
    let s = std::str::from_utf8(&body).expect("utf8");
    assert!(s.contains("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}"));
    assert!(s.contains("data: [DONE]"));
}

#[tokio::test]
async fn fr_005_invalid_config_fails_loudly() {
    // Exercise the config layer directly: an empty backend URL is
    // rejected by the validator.
    let env = riftgate_config::Env::new();
    let err = riftgate_config::load(None, &env).expect_err("default config has empty backend URL");
    let msg = err
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        msg.to_lowercase().contains("backend"),
        "expected the validator to mention backend; got: {msg}"
    );
}

#[tokio::test]
async fn fr_006_emits_canonical_span_sequence() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = http_client();
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://{gateway}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(
            br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#,
        )))
        .unwrap();
    let _resp = client.request(req).await.expect("request gateway");

    // The bus drains asynchronously; collect spans for up to 1 s.
    let mut span_names: Vec<&'static str> = Vec::new();
    for _ in 0..50 {
        for e in sink.drain() {
            if let riftgate_core::obs::ObservabilityEvent::SpanEnd { name, .. } = e {
                span_names.push(name);
            }
        }
        let saw_all = [
            riftgate_obs::spans::REQUEST_RECEIVED,
            riftgate_obs::spans::REQUEST_QUEUED,
            riftgate_obs::spans::REQUEST_DISPATCHED,
            riftgate_obs::spans::REQUEST_COMPLETED,
        ]
        .iter()
        .all(|n| span_names.contains(n));
        if saw_all {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for expected in [
        riftgate_obs::spans::REQUEST_RECEIVED,
        riftgate_obs::spans::REQUEST_QUEUED,
        riftgate_obs::spans::REQUEST_DISPATCHED,
        riftgate_obs::spans::REQUEST_COMPLETED,
    ] {
        assert!(
            span_names.contains(&expected),
            "missing span `{expected}` in observed sequence: {span_names:?}"
        );
    }
}

#[tokio::test]
async fn fr_007_arena_per_request_does_not_grow_unboundedly() {
    // The arena is constructed per-request and dropped at the end of
    // the handler; from the outside we assert "many requests succeed
    // and the gateway stays responsive". A direct memory probe lives
    // in the v0.1 benchmarks (Phase J).
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = http_client();
    for _ in 0..200 {
        let req = Request::builder()
            .method("POST")
            .uri(format!("http://{gateway}/v1/chat/completions"))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(
                br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#,
            )))
            .unwrap();
        let resp = client.request(req).await.expect("request gateway");
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = read_body(resp).await;
    }
}

#[tokio::test]
async fn fr_008_binary_heap_timer_handles_many_concurrent_timers() {
    use riftgate_core::timers::{BinaryHeapTimers, TimerSubsystem};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    let mut t = BinaryHeapTimers::new();
    let now = Instant::now();
    let fired_count = Arc::new(AtomicUsize::new(0));
    for i in 0..100_000u64 {
        let counter = fired_count.clone();
        let _ = t.schedule(
            now + Duration::from_secs(3600 + i),
            Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        );
    }
    let pre_tick_len = t.len();
    let started = Instant::now();
    t.tick(now);
    let elapsed = started.elapsed();
    assert_eq!(
        fired_count.load(Ordering::Relaxed),
        0,
        "no timers should fire when every deadline is in the future"
    );
    assert_eq!(t.len(), pre_tick_len);
    // O(log n) on 100k timers is microseconds; 50 ms upper bound is
    // generous and stays robust on slow CI.
    assert!(
        elapsed < Duration::from_millis(50),
        "tick on 100k timers took {elapsed:?}; expected <50 ms"
    );
}

#[tokio::test]
async fn ready_flips_to_503_during_drain() {
    let (upstream, _stop) = spawn_mock_upstream().await;
    let (gateway, _sink, drain_tx) = spawn_riftgate(upstream, 5_000).await;

    let client = empty_client();
    let req = Request::builder()
        .method("GET")
        .uri(format!("http://{gateway}/ready"))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = client.request(req).await.expect("ready before drain");
    assert_eq!(resp.status(), StatusCode::OK);

    shutdown::begin_drain(&drain_tx);

    let req = Request::builder()
        .method("GET")
        .uri(format!("http://{gateway}/ready"))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = client.request(req).await.expect("ready after drain");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn upstream_timeout_returns_504() {
    // An upstream that accepts but never responds.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((_stream, _)) => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    Err(_) => break,
                }
            }
        }
    });

    let (gateway, _sink, _drain_tx) = spawn_riftgate(upstream_addr, 200).await;

    let client = http_client();
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://{gateway}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(
            br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#,
        )))
        .unwrap();
    let resp = client.request(req).await.expect("request gateway");
    assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);

    let _ = stop_tx.send(());
}
