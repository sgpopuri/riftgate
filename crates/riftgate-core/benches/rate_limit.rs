//! Microbenchmarks for [`riftgate_core::rate_limit`] hot-path checks.

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate_core::rate_limit::{
    CostDimension, LimitDecision, RateLimiter, SubjectKey, TokenBucketConfig, TokenBucketLimiter,
};
use riftgate_core::types::{RouteId, TenantId};
use std::hint::black_box;

fn bench_token_bucket_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limit/token_bucket_check");
    for (rate_per_sec, burst) in [(100_000_u32, 100_000_u32), (1_000_000, 1_000_000)] {
        group.bench_with_input(
            BenchmarkId::new("hot_subject", format!("rate{rate_per_sec}_burst{burst}")),
            &(rate_per_sec, burst),
            |b, &(rate_per_sec, burst)| {
                let limiter = TokenBucketLimiter::new(TokenBucketConfig {
                    rate_per_sec,
                    burst,
                    cost: CostDimension::Request,
                    ..TokenBucketConfig::default()
                });
                let subject = SubjectKey::per_route(TenantId(7), RouteId(42));
                b.iter(|| {
                    let decision = limiter.check(&subject, 1);
                    match decision {
                        LimitDecision::Allow => black_box(1_u8),
                        LimitDecision::Deny { .. } => black_box(0_u8),
                    }
                });
            },
        );
    }
    group.finish();
}

mod harness {
    use super::bench_token_bucket_check;
    use criterion::criterion_group;
    criterion_group!(rate_limit, bench_token_bucket_check);
}
criterion_main!(harness::rate_limit);
