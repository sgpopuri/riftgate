//! Microbenchmarks for routing hot-path decisions.

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate_core::request::{Body, Headers, Method, Request};
use riftgate_core::router::{BackendId, BackendPool, BackendSignal, BackendSignals, Router};
use riftgate_core::types::RequestId;
use riftgate_router::{CircuitBreakerArbiter, CircuitBreakerConfig, WeightedRandomRouter};
use std::hint::black_box;

fn make_req() -> Request {
    Request {
        id: RequestId(1),
        method: Method::Post,
        path: "/v1/chat/completions".into(),
        headers: Headers::new(),
        body: Body::Empty,
    }
}

fn bench_weighted_route(c: &mut Criterion) {
    let mut group = c.benchmark_group("routing/weighted");
    let req = make_req();
    for n in [2_u16, 8_u16, 16_u16] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut weights = Vec::with_capacity(n as usize);
            for i in 0..n {
                weights.push((BackendId(i), 1_u32 + u32::from(i % 3)));
            }
            let pool = BackendPool::from_ids(weights.iter().map(|(b, _)| *b).collect());
            let signals = BackendSignals::from_vec(vec![BackendSignal::default(); n as usize]);
            let router = WeightedRandomRouter::with_seed(&weights, 0xA11CE);
            b.iter(|| {
                black_box(router.route(&req, &pool, &signals));
            });
        });
    }
    group.finish();
}

fn bench_breaker_weighted_route(c: &mut Criterion) {
    let mut group = c.benchmark_group("routing/circuit_breaker_weighted");
    let req = make_req();
    for n in [2_u16, 8_u16, 16_u16] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut weights = Vec::with_capacity(n as usize);
            for i in 0..n {
                weights.push((BackendId(i), 1_u32 + u32::from(i % 3)));
            }
            let pool = BackendPool::from_ids(weights.iter().map(|(b, _)| *b).collect());
            let signals = BackendSignals::from_vec(vec![BackendSignal::default(); n as usize]);
            let inner = WeightedRandomRouter::with_seed(&weights, 0xBADC0DE);
            let router = CircuitBreakerArbiter::new(inner, CircuitBreakerConfig::default());
            b.iter(|| {
                black_box(router.route(&req, &pool, &signals));
            });
        });
    }
    group.finish();
}

mod harness {
    use super::{bench_breaker_weighted_route, bench_weighted_route};
    use criterion::criterion_group;
    criterion_group!(routing, bench_weighted_route, bench_breaker_weighted_route);
}
criterion_main!(harness::routing);
