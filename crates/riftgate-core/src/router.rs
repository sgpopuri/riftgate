//! Router trait + supporting types.
//!
//! Concrete router impls live in `crates/riftgate-router`:
//!
//! - `RoundRobinRouter` (`v0.1`)
//! - `WeightedRandomRouter` (`v0.2`)
//! - `KvAwareRouter` (`v0.3`)
//! - `HedgedRouter` (`v0.3`)
//!
//! See [`docs/04-design/lld-routing.md`](../../../docs/04-design/lld-routing.md)
//! for the design rationale and the data-structures discussion.

use crate::request::{Request, Response, StatusCode};
use crate::types::TenantId;
use core::fmt;
use std::time::Duration;

/// Per-backend identifier.
///
/// Backends are loaded from configuration and assigned ids in the order
/// they appear. Used by every routing decision and every per-backend
/// metric label.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct BackendId(pub u16);

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "backend-{}", self.0)
    }
}

/// Circuit-breaker state for a single backend.
///
/// The full circuit-breaker decision lands in `v0.2` per Options 011; this
/// enum exists in `v0.1` so the `Router` trait shape is stable.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum CircuitState {
    /// Backend is healthy; routing decisions may select it freely.
    #[default]
    Closed,
    /// Backend is unhealthy; routing decisions skip it.
    Open,
    /// A fraction of requests are routed to it as a probe.
    HalfOpen,
}

/// Health and pressure signals for a single backend.
#[derive(Debug, Clone)]
pub struct BackendSignal {
    /// Circuit-breaker state. `Closed` is healthy.
    pub circuit_state: CircuitState,
    /// GPU pressure observed for this backend, normalized to `0.0..=1.0`.
    /// `None` if the backend does not surface this signal (no NVML / DCGM
    /// integration). New in `v0.4`; kept on the v0.1 type so the trait
    /// shape is stable.
    pub gpu_pressure: Option<f32>,
    /// Recent observed P99 latency in milliseconds. Updated by the routing
    /// crate on each `on_response` call.
    pub recent_p99_ms: f32,
}

impl Default for BackendSignal {
    fn default() -> Self {
        Self {
            circuit_state: CircuitState::Closed,
            gpu_pressure: None,
            recent_p99_ms: 0.0,
        }
    }
}

/// Snapshot of all per-backend signals. Built by the routing crate at
/// request entry; passed read-only to the router.
#[derive(Debug, Default, Clone)]
pub struct BackendSignals {
    signals: Vec<BackendSignal>,
}

impl BackendSignals {
    /// Construct an empty `BackendSignals`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from a `Vec` of signals indexed by `BackendId`.
    pub fn from_vec(signals: Vec<BackendSignal>) -> Self {
        Self { signals }
    }

    /// Return the signal for the given backend, or a default if out of
    /// range.
    pub fn get(&self, id: BackendId) -> BackendSignal {
        self.signals.get(id.0 as usize).cloned().unwrap_or_default()
    }

    /// Number of backends represented.
    pub fn len(&self) -> usize {
        self.signals.len()
    }

    /// `true` if no backends are represented.
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }
}

/// Snapshot of the configured backend pool.
///
/// Immutable after construction; the routing crate rebuilds the pool when
/// configuration changes. Routers consult the pool by `BackendId` and emit
/// a [`RoutingDecision`] referencing one or more.
#[derive(Debug, Default, Clone)]
pub struct BackendPool {
    backends: Vec<BackendId>,
}

impl BackendPool {
    /// Construct an empty `BackendPool`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from a `Vec` of backend ids.
    pub fn from_ids(backends: Vec<BackendId>) -> Self {
        Self { backends }
    }

    /// Iterator over backend ids, in their configured order.
    pub fn iter(&self) -> impl Iterator<Item = BackendId> + '_ {
        self.backends.iter().copied()
    }

    /// Number of backends in the pool.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// `true` if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Return the backend at the given index, modulo the pool length.
    /// Useful for round-robin and weighted-random impls.
    pub fn get_modulo(&self, index: usize) -> Option<BackendId> {
        if self.backends.is_empty() {
            None
        } else {
            Some(self.backends[index % self.backends.len()])
        }
    }
}

/// Outcome of a [`Router::route`] call.
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    /// Send the request to the named backend.
    Send(BackendId),
    /// Race the request against multiple backends; the first to respond
    /// wins, the loser is cancelled mid-stream. Lands in the `HedgedRouter`
    /// in `v0.3`.
    Hedge(Vec<BackendId>),
    /// Reject the request with the given status. Used by routers that
    /// implement admission control (e.g. tenant gating).
    Reject(StatusCode),
}

/// Outcome of an upstream call, fed back to the router via
/// [`Router::on_response`].
///
/// Distinct from [`crate::request::Outcome`] in that this carries
/// router-relevant signals only.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// Backend that handled the request.
    pub backend: BackendId,
    /// HTTP status returned, if any.
    pub status: Option<StatusCode>,
    /// Wall-clock duration of the upstream call.
    pub duration: Duration,
    /// `true` if the upstream call succeeded end-to-end.
    pub ok: bool,
}

/// Router trait.
///
/// **`Send + Sync`** because routers are shared across shards via `Arc`.
///
/// **Pure functions of `(request, pool, signals)`.** Side effects only via
/// `on_response`. This separation makes routers testable in isolation.
///
/// Trait object safety: yes (no generics, no associated types).
pub trait Router: Send + Sync {
    /// Decide which backend(s) handle this request.
    fn route(&self, req: &Request, pool: &BackendPool, signals: &BackendSignals)
    -> RoutingDecision;

    /// Optional callback fired after the upstream returns. Routers that
    /// maintain state (hedged-router, EWMA-aware router) update it here.
    /// Default: no-op.
    fn on_response(&self, _decision: &RoutingDecision, _outcome: &Outcome) {}

    /// Optional callback for tenant-gating routers; default: no-op.
    fn on_tenant(&self, _tenant: TenantId, _resp: &Response) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_pool_modulo() {
        let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1), BackendId(2)]);
        assert_eq!(pool.get_modulo(0), Some(BackendId(0)));
        assert_eq!(pool.get_modulo(4), Some(BackendId(1)));
        assert_eq!(pool.get_modulo(99), Some(BackendId(0)));
    }

    #[test]
    fn empty_pool_returns_none() {
        let pool = BackendPool::new();
        assert_eq!(pool.get_modulo(0), None);
    }

    #[test]
    fn signals_default_when_out_of_range() {
        let s = BackendSignals::new();
        assert_eq!(s.get(BackendId(0)).circuit_state, CircuitState::Closed);
    }
}
