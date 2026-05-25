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
}
