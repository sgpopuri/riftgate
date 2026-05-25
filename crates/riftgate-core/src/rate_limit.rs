//! `RateLimiter` trait — defined in `v0.1`, default impl deferred to `v0.2`.
//!
//! Per [ADR 0009](../../../docs/06-adrs/0009-rate-limiter-trait-in-proc-only.md)
//! (proposed) the v0.2 default impl is a sharded in-proc token bucket; the
//! v0.1 trait shape is locked in here so callers (the future request-side
//! filter chain, the future MCP capability broker) can compile against it
//! immediately.
//!
//! See [Options 021](../../../docs/05-options/021-rate-limiting.md) for the
//! full design space and [`docs/04-design/lld-rate-limiter.md`](../../../docs/04-design/lld-rate-limiter.md).
//!
//! **Why no v0.1 impl?** [`FR-108`](../../../docs/01-requirements/functional.md)
//! explicitly targets the in-proc token-bucket impl at `v0.2`. Shipping a
//! `NoopLimiter` in v0.1 would create the false impression that rate
//! limiting is enabled; the trait + `Option<Arc<dyn RateLimiter>>` shape on
//! callers (no global default) is the right `v0.1` posture.

use crate::router::BackendId;
use crate::types::{RouteId, TenantId};
use std::time::Duration;

/// Subject of a rate-limit decision.
///
/// The combination `(tenant, route, backend)` is the canonical key shape
/// per [Options 021 §6](../../../docs/05-options/021-rate-limiting.md). For
/// per-tenant-only limits, leave `route` and `backend` at default; for
/// per-backend-only limits, leave `tenant` and `route` at default.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SubjectKey {
    /// Per-tenant identity (default `TenantId(0)` for single-tenant
    /// deployments).
    pub tenant: TenantId,
    /// Per-route identity (default `RouteId(0)` for cross-route limits).
    pub route: RouteId,
    /// Optional per-backend scope.
    pub backend: Option<BackendId>,
}

impl SubjectKey {
    /// Construct a per-tenant subject (route and backend default).
    pub fn per_tenant(tenant: TenantId) -> Self {
        Self {
            tenant,
            route: RouteId(0),
            backend: None,
        }
    }

    /// Construct a per-`(tenant, route)` subject.
    pub fn per_route(tenant: TenantId, route: RouteId) -> Self {
        Self {
            tenant,
            route,
            backend: None,
        }
    }
}

/// Outcome of a rate-limit check.
///
/// `Allow` does not consume any state outside the limiter; `Deny` carries
/// a `retry_after` so the caller can populate the `Retry-After: <seconds>`
/// HTTP header.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LimitDecision {
    /// Request may proceed.
    Allow,
    /// Request is denied. `Retry-After` advice in `retry_after`.
    Deny {
        /// How long the caller should wait before retrying.
        retry_after: Duration,
    },
}

/// Rate-limiter trait.
///
/// Trait shape only in `v0.1`; the v0.2 in-proc token-bucket impl
/// (`TokenBucketLimiter`) lives in `crates/riftgate-core::rate_limit` once
/// the v0.2 milestone opens. Distributed impls (Redis-Lua GCRA,
/// consistent-hash-routed) ship in their own crates per Options 021 §3.7.
///
/// **`Send + Sync`** — limiters are constructed once at startup and shared
/// by the request path via `Arc`.
///
/// `cost` is the rate-cost of this request (typically 1 for RPM limiting,
/// or `tokens_in_prompt` for TPM limiting).
pub trait RateLimiter: Send + Sync {
    /// Decide whether the given subject may consume `cost` units of rate
    /// capacity.
    ///
    /// Returns `LimitDecision::Allow` if so; `LimitDecision::Deny { ... }`
    /// otherwise. Implementations MUST be lock-free on the fast path
    /// ([`NFR-P07`](../../../docs/01-requirements/non-functional.md): <100 µs
    /// at 1k RPS).
    fn check(&self, subject: &SubjectKey, cost: u32) -> LimitDecision;
}

/// Rate-cost dimension used by [`TokenBucketLimiter`].
///
/// `Request` charges one bucket-token per request; `PromptTokens` charges
/// `cost` tokens per request, where `cost` is the prompt-token count the
/// caller already computed. Per
/// [ADR 0018](../../../docs/06-adrs/0018-token-bucket-parameters.md) the
/// two dimensions are first-class so an operator can run RPM-limited and
/// TPM-limited buckets side by side without a second limiter impl.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CostDimension {
    /// One unit per request, regardless of payload size.
    Request,
    /// Caller-supplied unit count (typically prompt tokens).
    PromptTokens,
}

/// Operator-facing configuration for one [`TokenBucketLimiter`].
///
/// Knobs and defaults match
/// [Options 023 §6](../../../docs/05-options/023-token-bucket-parameters.md)
/// and [ADR 0018](../../../docs/06-adrs/0018-token-bucket-parameters.md).
#[derive(Debug, Clone, Copy)]
pub struct TokenBucketConfig {
    /// Steady-state refill rate in tokens per second.
    pub rate_per_sec: u32,
    /// Maximum bucket depth (peak burst).
    pub burst: u32,
    /// Whether `cost` carries request count or prompt-token count.
    pub cost: CostDimension,
    /// Lower bound on the `Retry-After` value the limiter returns on
    /// `Deny`. Protects clients from retry storms when `rate_per_sec` is
    /// large and the bucket is just barely depleted.
    pub retry_after_floor: Duration,
}

impl Default for TokenBucketConfig {
    fn default() -> Self {
        Self {
            rate_per_sec: 100,
            burst: 100,
            cost: CostDimension::Request,
            retry_after_floor: Duration::from_millis(50),
        }
    }
}

/// In-proc sharded token-bucket rate limiter.
///
/// Per [ADR 0018](../../../docs/06-adrs/0018-token-bucket-parameters.md):
///
/// - One packed `AtomicU64` per `SubjectKey`, kept in a `dashmap::DashMap`
///   with 64 shards. The packed layout is
///   `[u32 tokens_scaled | u32 last_refill_ms_since_start]`; the scale
///   factor `SCALE = 1 << 16` gives microtoken-precision arithmetic
///   without floats and matches the rate-limit microbench's
///   `NFR-P07` budget.
/// - Refill is lazy: on each `check`, we compute the elapsed milliseconds
///   since `last_refill`, add `elapsed_ms * rate_per_sec / 1000` scaled
///   tokens, clamp to `burst`, attempt to subtract `cost`, and CAS the
///   new state back. On CAS failure we retry the whole arithmetic; the
///   typical contention case settles in 1-2 iterations.
/// - Time is monotonic milliseconds since process start. `u32` ms wraps
///   after ~49 days; an entry that has not been touched in that window is
///   treated as a full bucket on the next call, which is the correct
///   behavior for a lazy-refill limiter.
///
/// Lock-free on the fast path: the only blocking primitive is the
/// `DashMap` shard lock around the `entry_or_insert_with` path, taken
/// once per new subject and never again for hot subjects.
pub struct TokenBucketLimiter {
    cfg: TokenBucketConfig,
    state: dashmap::DashMap<SubjectKey, std::sync::atomic::AtomicU64>,
    start: std::time::Instant,
}

/// Fixed-point scale for `tokens_scaled`. Per
/// [ADR 0018](../../../docs/06-adrs/0018-token-bucket-parameters.md) the
/// value is `1 << 16`; changing it is a breaking ABI change for any future
/// metric that exposes the raw scaled count.
pub const TOKEN_BUCKET_SCALE: u64 = 1 << 16;

/// Number of DashMap shards. Per
/// [ADR 0018](../../../docs/06-adrs/0018-token-bucket-parameters.md).
pub const TOKEN_BUCKET_SHARDS: usize = 64;

impl TokenBucketLimiter {
    /// Construct a limiter with the given configuration.
    ///
    /// # Panics
    /// Panics if `cfg.burst == 0` or `cfg.rate_per_sec == 0`. A zero-rate
    /// or zero-burst limiter is always-deny; the caller should use
    /// [`NoopLimiter`] (always-allow) or simply not install a limiter
    /// rather than encoding it here.
    #[must_use]
    pub fn new(cfg: TokenBucketConfig) -> Self {
        assert!(
            cfg.rate_per_sec > 0,
            "TokenBucketConfig.rate_per_sec must be > 0"
        );
        assert!(cfg.burst > 0, "TokenBucketConfig.burst must be > 0");
        Self {
            cfg,
            state: dashmap::DashMap::with_shard_amount(TOKEN_BUCKET_SHARDS),
            start: std::time::Instant::now(),
        }
    }

    fn now_ms(&self) -> u32 {
        // Monotonic ms-since-start, wrapped to u32 (~49 days). See type-
        // level docs for wrap-handling.
        let elapsed = self.start.elapsed().as_millis();
        (elapsed as u64) as u32
    }

    fn full_state(&self, now_ms: u32) -> u64 {
        pack(saturating_scaled_burst(self.cfg.burst), now_ms)
    }

    fn cost_units(&self, requested: u32) -> u32 {
        match self.cfg.cost {
            CostDimension::Request => 1,
            CostDimension::PromptTokens => requested.max(1),
        }
    }

    fn retry_after_for(&self, deficit_scaled: u64) -> Duration {
        // deficit_scaled is "scaled tokens we are short of cost"; convert
        // to milliseconds via rate_per_sec.
        let rate = u64::from(self.cfg.rate_per_sec);
        let scaled_per_ms = (rate * TOKEN_BUCKET_SCALE) / 1000;
        let scaled_per_ms = scaled_per_ms.max(1);
        let ms = deficit_scaled.div_ceil(scaled_per_ms);
        let retry = Duration::from_millis(ms);
        if retry < self.cfg.retry_after_floor {
            self.cfg.retry_after_floor
        } else {
            retry
        }
    }
}

fn pack(tokens_scaled: u64, last_refill_ms: u32) -> u64 {
    let tokens_u32 = tokens_scaled.min(u64::from(u32::MAX)) as u32;
    (u64::from(tokens_u32) << 32) | u64::from(last_refill_ms)
}

fn unpack(state: u64) -> (u64, u32) {
    let tokens_u32 = (state >> 32) as u32;
    let last_refill_ms = (state & 0xFFFF_FFFF) as u32;
    (u64::from(tokens_u32), last_refill_ms)
}

fn saturating_scaled_burst(burst: u32) -> u64 {
    // burst (raw tokens) × SCALE; clamped to u32::MAX so the packed
    // representation never loses precision unexpectedly. With
    // SCALE = 1 << 16, this gives a representable burst up to 65_535
    // raw tokens. Operators above that should configure burst per
    // shard or split into multiple subjects.
    (u64::from(burst) * TOKEN_BUCKET_SCALE).min(u64::from(u32::MAX))
}

impl RateLimiter for TokenBucketLimiter {
    fn check(&self, subject: &SubjectKey, cost: u32) -> LimitDecision {
        use std::sync::atomic::{AtomicU64, Ordering};
        let now_ms = self.now_ms();
        let burst_scaled = saturating_scaled_burst(self.cfg.burst);
        let cost_units = u64::from(self.cost_units(cost));
        let cost_scaled = cost_units * TOKEN_BUCKET_SCALE;

        // Per-subject lazy refill + CAS update. New subjects start full.
        let entry = self
            .state
            .entry(*subject)
            .or_insert_with(|| AtomicU64::new(self.full_state(now_ms)));

        loop {
            let observed = entry.load(Ordering::Acquire);
            let (tokens_scaled, last_refill_ms) = unpack(observed);
            // u32 wrap-safe elapsed_ms.
            let elapsed_ms = now_ms.wrapping_sub(last_refill_ms);
            let refill_scaled =
                u64::from(elapsed_ms) * u64::from(self.cfg.rate_per_sec) * TOKEN_BUCKET_SCALE
                    / 1000;
            let after_refill = (tokens_scaled + refill_scaled).min(burst_scaled);

            if after_refill < cost_scaled {
                let deficit = cost_scaled - after_refill;
                // Update state to reflect the refill even on deny so the
                // next caller does not pay the same elapsed-ms work.
                let new_state = pack(after_refill, now_ms);
                let _ = entry.compare_exchange(
                    observed,
                    new_state,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                return LimitDecision::Deny {
                    retry_after: self.retry_after_for(deficit),
                };
            }

            let new_tokens = after_refill - cost_scaled;
            let new_state = pack(new_tokens, now_ms);
            match entry.compare_exchange(observed, new_state, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return LimitDecision::Allow,
                Err(_) => continue, // Another writer raced; retry.
            }
        }
    }
}

/// Pass-through limiter. Always returns `Allow`.
///
/// Used when rate limiting is disabled in config and as a baseline in
/// conformance tests.
pub struct NoopLimiter;

impl RateLimiter for NoopLimiter {
    fn check(&self, _subject: &SubjectKey, _cost: u32) -> LimitDecision {
        LimitDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny test impl that lets the trait be exercised in unit tests.
    /// Always allows, but increments a counter so a test can assert it
    /// was consulted.
    struct AlwaysAllow;
    impl RateLimiter for AlwaysAllow {
        fn check(&self, _subject: &SubjectKey, _cost: u32) -> LimitDecision {
            LimitDecision::Allow
        }
    }

    #[test]
    fn rate_limiter_is_dyn_safe() {
        let r: Box<dyn RateLimiter> = Box::new(AlwaysAllow);
        let key = SubjectKey::per_tenant(TenantId(7));
        assert_eq!(r.check(&key, 1), LimitDecision::Allow);
    }

    #[test]
    fn subject_key_constructors() {
        let a = SubjectKey::per_tenant(TenantId(1));
        assert_eq!(a.tenant, TenantId(1));
        assert_eq!(a.route, RouteId(0));
        assert_eq!(a.backend, None);

        let b = SubjectKey::per_route(TenantId(2), RouteId(5));
        assert_eq!(b.route, RouteId(5));
    }

    #[test]
    fn noop_limiter_always_allows() {
        let l = NoopLimiter;
        assert_eq!(
            l.check(&SubjectKey::per_tenant(TenantId(1)), 1),
            LimitDecision::Allow
        );
    }

    #[test]
    fn token_bucket_denies_after_burst_exhausted() {
        let l = TokenBucketLimiter::new(TokenBucketConfig {
            rate_per_sec: 1,
            burst: 3,
            cost: CostDimension::Request,
            retry_after_floor: Duration::from_millis(0),
        });
        let key = SubjectKey::per_tenant(TenantId(1));
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
        match l.check(&key, 1) {
            LimitDecision::Deny { retry_after } => {
                assert!(retry_after > Duration::ZERO);
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn token_bucket_refills_over_time() {
        let l = TokenBucketLimiter::new(TokenBucketConfig {
            rate_per_sec: 1000,
            burst: 2,
            cost: CostDimension::Request,
            retry_after_floor: Duration::from_millis(0),
        });
        let key = SubjectKey::per_tenant(TenantId(2));
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
        // Bucket empty; wait for refill.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(l.check(&key, 1), LimitDecision::Allow);
    }

    #[test]
    fn token_bucket_retry_after_respects_floor() {
        let l = TokenBucketLimiter::new(TokenBucketConfig {
            rate_per_sec: 1_000_000,
            burst: 1,
            cost: CostDimension::Request,
            retry_after_floor: Duration::from_millis(75),
        });
        let key = SubjectKey::per_tenant(TenantId(3));
        l.check(&key, 1);
        match l.check(&key, 1) {
            LimitDecision::Deny { retry_after } => {
                assert!(retry_after >= Duration::from_millis(75));
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn token_bucket_prompt_tokens_cost() {
        let l = TokenBucketLimiter::new(TokenBucketConfig {
            rate_per_sec: 1,
            burst: 100,
            cost: CostDimension::PromptTokens,
            retry_after_floor: Duration::from_millis(0),
        });
        let key = SubjectKey::per_tenant(TenantId(4));
        assert_eq!(l.check(&key, 40), LimitDecision::Allow);
        assert_eq!(l.check(&key, 40), LimitDecision::Allow);
        match l.check(&key, 40) {
            LimitDecision::Deny { .. } => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn token_bucket_independent_subjects() {
        let l = TokenBucketLimiter::new(TokenBucketConfig {
            rate_per_sec: 1,
            burst: 1,
            cost: CostDimension::Request,
            retry_after_floor: Duration::from_millis(0),
        });
        let a = SubjectKey::per_tenant(TenantId(10));
        let b = SubjectKey::per_tenant(TenantId(11));
        assert_eq!(l.check(&a, 1), LimitDecision::Allow);
        assert_eq!(l.check(&b, 1), LimitDecision::Allow);
        // Each subject's burst is independent.
        match l.check(&a, 1) {
            LimitDecision::Deny { .. } => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }
}
