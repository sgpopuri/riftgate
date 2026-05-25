//! End-to-end throughput benchmark: real hyper client -> riftgate
//! gateway -> mock upstream that returns a canned JSON response.
//!
//! This is the only v0.1 benchmark that exercises the entire request
//! path. It is deliberately conservative: a single Tokio runtime with
//! a single in-flight client request, so the measurement reports
//! end-to-end latency rather than peak throughput.
//!
//! Use as a regression gate: large jumps in mean latency between
//! `cargo bench --bench end_to_end` runs indicate a meaningful
//! pipeline regression.

use bytes::Bytes;
use criterion::{Criterion, criterion_main};
use http::{Method, Request, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
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

const SAMPLE_RESPONSE: &[u8] = br#"{"id":"chatcmpl-test","object":"chat.completion","model":"gpt-4o-mini","choices":[{"message":{"role":"assistant","content":"hello back"}}]}"#;

async fn upstream_router(
    _req: Request<Incoming>,
) -> Result<http::Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let body = Full::new(Bytes::from_static(SAMPLE_RESPONSE))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Ok(http::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(body)
        .unwrap())
}

async fn spawn_upstream() -> SocketAddr {
    let listener = TcpListener::bind("localhost:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let svc = service_fn(upstream_router);
                let _ = server_http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    });
    addr
}

async fn spawn_gateway(upstream: SocketAddr) -> SocketAddr {
    let mut config = riftgate_config::Config::default();
    config.server.listen_addr = "localhost:0".parse().unwrap();
    config.backend.url = format!("http://{upstream}");
    config.backend.timeout_ms = 5_000;
    config.obs.bus_capacity = 4096;
    let config = Arc::new(config);

    let sink = Arc::new(riftgate_core::obs::InMemorySink::new());
    let sink_dyn: Arc<dyn riftgate_core::obs::ObservabilitySink> = sink;
    let bus = Bus::new(config.obs.bus_capacity, sink_dyn);
    let publisher = bus.publisher();
    Box::leak(Box::new(bus));

    let upstream_client = gw_upstream::build_client();
    let pool = Arc::new(BackendPool::from_ids(vec![BackendId(0)]));
    let signals = Arc::new(BackendSignals::new());
    let router: Arc<dyn riftgate_core::router::Router> = Arc::new(RoundRobinRouter::new());

    let (drain_tx, drain_rx) = shutdown::channel();
    // Hold the drain sender for the lifetime of the bench process so the
    // accept loop does not see the sender dropped (which it treats as a
    // drain signal — and which would close the listener before warmup
    // connects).
    Box::leak(Box::new(drain_tx));
    let state = proxy::HandlerState {
        config: config.clone(),
        router,
        pool,
        signals,
        upstream: upstream_client,
        publisher,
        drain: drain_rx.clone(),
    };

    let listener = server::bind(config.server.listen_addr).await.unwrap();
    let bound = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = server::accept_loop(listener, state, drain_rx, Duration::from_secs(1)).await;
    });
    bound
}

fn bench_end_to_end(c: &mut Criterion) {
    // Multi-thread runtime so the spawned accept loop runs on a worker
    // independent of the one driving the iter loop.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let (gateway_addr, body_bytes) = runtime.block_on(async {
        let upstream = spawn_upstream().await;
        let gateway = spawn_gateway(upstream).await;
        // Warm one request so the connection pool is established.
        let client: Client<hyper_util::client::legacy::connect::HttpConnector, Empty<Bytes>> =
            Client::builder(TokioExecutor::new()).build_http();
        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("http://{gateway}/v1/models"))
            .body(Empty::new())
            .unwrap();
        let resp = client.request(req).await.expect("warmup");
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = resp.into_body().collect().await.unwrap();
        (gateway, SAMPLE_RESPONSE.len() as u64)
    });

    let client: Client<hyper_util::client::legacy::connect::HttpConnector, Full<Bytes>> =
        runtime.block_on(async { Client::builder(TokioExecutor::new()).build_http() });

    let mut group = c.benchmark_group("end_to_end");
    group.throughput(criterion::Throughput::Bytes(body_bytes));
    group.bench_function("post_chat_completions", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = Request::builder()
                .method(Method::POST)
                .uri(format!("http://{gateway_addr}/v1/chat/completions"))
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from_static(
                    br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#,
                )))
                .unwrap();
            let resp = client.request(req).await.expect("request gateway");
            assert_eq!(resp.status(), StatusCode::OK);
            let _ = resp.into_body().collect().await.unwrap();
        });
    });
    group.finish();
}

mod harness {
    use super::bench_end_to_end;
    use criterion::criterion_group;
    criterion_group!(end_to_end, bench_end_to_end);
}
criterion_main!(harness::end_to_end);
