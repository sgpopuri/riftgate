//! Hedged-request router — v0.3 decorator over an inner `Router`.
//!
//! Per [ADR 0023](../../../../docs/06-adrs/0023-hedged-requests-p99-triggered.md):
//!
//! - **Dean–Barroso threshold-triggered hedging** at degree=2. On `route`,
//!   the decorator delegates to the inner router to pick the *primary*
//!   backend. If that backend's estimated p95 first-byte latency exceeds
//!   `hedge_min_threshold_ms`, *and* the global hedge budget has spare
//!   capacity, the decorator promotes the decision to
//!   `RoutingDecision::Hedge(vec![primary, secondary])`, picking a
//!   `Closed` secondary distinct from the primary.
//! - **Per-backend P² estimator** (Jain & Chlamtac, 1985) tracks first-byte
//!   latency. State per backend: five quantile markers + counters.
//!   `O(1)` per update; bounded memory.
//! - **Global rate-limit-budget cap**: at most `hedge_max_fraction` of all
//!   routing decisions may be promoted to a hedge.
//!
//! The actual fan-out (dispatching both backends, racing them, cancelling
//! the loser) lives in the request driver (`crates/riftgate/src/proxy.rs`,
//! deferred to a follow-on implementation PR). The router is the policy seam.
//!
//! ## Decision flow
//!
//! ```text
//!   route(req, pool, signals):
//!     primary_decision = inner.route(...)
//!     if primary_decision is not Send(primary):
//!       return primary_decision
//!
//!     q = p95_estimator[primary]
//!     if estimator not warmed up:
//!       return Send(primary)
//!     if q < hedge_min_threshold_ms:
//!       return Send(primary)
//!     if hedge_budget_exhausted:
//!       return Send(primary)
//!
//!     secondary = first Closed backend != primary
//!     if secondary exists:
//!       return Hedge([primary, secondary])
//!     else:
//!       return Send(primary)
//! ```

use core::sync::atomic::{AtomicU64, Ordering};
use riftgate_core::request::Request;
use riftgate_core::router::{
    BackendId, BackendPool, BackendSignals, CircuitState, Outcome, Router, RoutingDecision,
};
use std::collections::HashMap;
use std::sync::Mutex;

/// Configuration for [`HedgedRouter`]. Per [ADR 0023].
#[derive(Debug, Clone, Copy)]
pub struct HedgedConfig {
    /// Quantile each per-backend estimator tracks. Default 0.95.
    pub hedge_after_quantile: f64,
    /// Maximum fraction of routing decisions that may be promoted to a
    /// hedge. Default 0.05.
    pub hedge_max_fraction: f64,
    /// Floor: never hedge if the primary's quantile latency estimate is
    /// below this many milliseconds. Default 50.
    pub hedge_min_threshold_ms: f32,
    /// Minimum number of observations a per-backend estimator must have
    /// digested before its quantile is considered usable. Default 25.
    /// Below this, the estimator is in warm-up; the router does not hedge.
    pub warmup_observations: u32,
}

impl Default for HedgedConfig {
    fn default() -> Self {
        Self {
            hedge_after_quantile: 0.95,
            hedge_max_fraction: 0.05,
            hedge_min_threshold_ms: 50.0,
            warmup_observations: 25,
        }
    }
}

/// P² algorithm (Jain & Chlamtac, 1985) — single-quantile online estimator.
///
/// Tracks the target quantile `p` (default 0.95) over a streaming sequence
/// of observations with O(1) update cost and constant memory (5 markers).
/// Accuracy at p95 is more than sufficient for hedge-trigger purposes; see
/// [ADR 0023 §Notes] for the rationale on P² vs t-digest or HDR histogram
/// in this slot.
#[derive(Debug, Clone)]
struct P2Estimator {
    /// Marker heights `q[0..5]` — the running quantile values for
    /// positions `n[i]`.
    q: [f64; 5],
    /// Actual marker positions `n[0..5]`.
    n: [f64; 5],
    /// Desired marker positions `n_prime[0..5]`.
    n_prime: [f64; 5],
    /// Desired-position increments `dn[0..5]`.
    dn: [f64; 5],
    /// Number of observations digested so far.
    count: u32,
    /// Target quantile p in (0, 1).
    p: f64,
}

impl P2Estimator {
    fn new(p: f64) -> Self {
        Self {
            q: [0.0; 5],
            n: [0.0; 5],
            n_prime: [0.0; 5],
            dn: [0.0, p / 2.0, p, (1.0 + p) / 2.0, 1.0],
            count: 0,
            p,
        }
    }

    fn observe(&mut self, x: f64) {
        if self.count < 5 {
            // Initialization phase: hold the first 5 observations.
            self.q[self.count as usize] = x;
            self.count += 1;
            if self.count == 5 {
                // Sort initial heights.
                self.q
                    .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                self.n = [0.0, 1.0, 2.0, 3.0, 4.0];
                self.n_prime = [0.0, 2.0 * self.p, 4.0 * self.p, 2.0 + 2.0 * self.p, 4.0];
            }
            return;
        }

        // Locate cell k such that q[k] <= x < q[k+1].
        let k = if x < self.q[0] {
            self.q[0] = x;
            0
        } else if x >= self.q[4] {
            self.q[4] = x;
            3
        } else {
            (0..4)
                .find(|&i| self.q[i] <= x && x < self.q[i + 1])
                .unwrap_or(3)
        };

        // Increment positions of markers k+1..4.
        for i in (k + 1)..5 {
            self.n[i] += 1.0;
        }
        for i in 0..5 {
            self.n_prime[i] += self.dn[i];
        }

        // Adjust heights of markers 1..=3.
        for i in 1..4 {
            let d = self.n_prime[i] - self.n[i];
            let np = self.n[i + 1] - self.n[i];
            let nm = self.n[i - 1] - self.n[i];
            if (d >= 1.0 && np > 1.0) || (d <= -1.0 && nm < -1.0) {
                let dsign = d.signum();
                // Parabolic prediction.
                let qp = self.q[i]
                    + dsign / (self.n[i + 1] - self.n[i - 1])
                        * ((self.n[i] - self.n[i - 1] + dsign) * (self.q[i + 1] - self.q[i]) / np
                            + (self.n[i + 1] - self.n[i] - dsign) * (self.q[i] - self.q[i - 1])
                                / nm);
                let new_q = if (self.q[i - 1] < qp) && (qp < self.q[i + 1]) {
                    qp
                } else {
                    // Linear fallback.
                    let neighbour = if dsign > 0.0 { i + 1 } else { i - 1 };
                    self.q[i]
                        + dsign * (self.q[neighbour] - self.q[i]) / (self.n[neighbour] - self.n[i])
                };
                self.q[i] = new_q;
                self.n[i] += dsign;
            }
        }

        self.count = self.count.saturating_add(1);
    }

    fn quantile(&self) -> Option<f64> {
        if self.count < 5 {
            None
        } else {
            Some(self.q[2])
        }
    }

    fn count(&self) -> u32 {
        self.count
    }
}

/// Hedged-request decorator router. See module docs.
pub struct HedgedRouter<R> {
    inner: R,
    cfg: HedgedConfig,
    estimators: Mutex<HashMap<BackendId, P2Estimator>>,
    decisions_total: AtomicU64,
    hedges_total: AtomicU64,
    budget_blocked: AtomicU64,
}

impl<R: core::fmt::Debug> core::fmt::Debug for HedgedRouter<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HedgedRouter")
            .field("inner", &self.inner)
            .field("cfg", &self.cfg)
            .finish()
    }
}

/// Telemetry snapshot for [`HedgedRouter`].
#[derive(Debug, Default, Copy, Clone)]
pub struct HedgeStats {
    /// Total routing decisions observed.
    pub decisions: u64,
    /// Total decisions promoted to a hedge.
    pub hedges_fired: u64,
    /// Hedge candidates blocked by the budget cap.
    pub budget_blocked: u64,
}

impl<R: Router> HedgedRouter<R> {
    /// Wrap `inner` with a `HedgedRouter` using `cfg`.
    #[must_use]
    pub fn new(inner: R, cfg: HedgedConfig) -> Self {
        Self {
            inner,
            cfg,
            estimators: Mutex::new(HashMap::new()),
            decisions_total: AtomicU64::new(0),
            hedges_total: AtomicU64::new(0),
            budget_blocked: AtomicU64::new(0),
        }
    }

    /// Snapshot of hedge statistics. Useful in tests and `/metrics`.
    #[must_use]
    pub fn stats(&self) -> HedgeStats {
        HedgeStats {
            decisions: self.decisions_total.load(Ordering::Relaxed),
            hedges_fired: self.hedges_total.load(Ordering::Relaxed),
            budget_blocked: self.budget_blocked.load(Ordering::Relaxed),
        }
    }

    /// Observed quantile for a backend, in milliseconds, or `None` if the
    /// estimator is still in warm-up.
    pub fn observed_quantile_ms(&self, backend: BackendId) -> Option<f32> {
        let estimators = self.estimators.lock().expect("hedge estimators poisoned");
        estimators
            .get(&backend)
            .and_then(P2Estimator::quantile)
            .map(|q| q as f32)
    }

    fn budget_allows(&self) -> bool {
        let total = self.decisions_total.load(Ordering::Relaxed);
        if total == 0 {
            return true;
        }
        let fired = self.hedges_total.load(Ordering::Relaxed);
        (fired as f64 / total as f64) < self.cfg.hedge_max_fraction
    }

    fn pick_secondary(
        &self,
        primary: BackendId,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> Option<BackendId> {
        // Pick the first `Closed` backend distinct from `primary`. The
        // simplicity is intentional for v0.3; future revisions may prefer
        // the backend with the *lowest* observed quantile.
        for b in pool.iter() {
            if b != primary && matches!(signals.get(b).circuit_state, CircuitState::Closed) {
                return Some(b);
            }
        }
        None
    }
}

impl<R: Router> Router for HedgedRouter<R> {
    fn route(
        &self,
        req: &Request,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> RoutingDecision {
        let decision = self.inner.route(req, pool, signals);
        self.decisions_total.fetch_add(1, Ordering::Relaxed);

        let primary = match decision {
            RoutingDecision::Send(b) => b,
            // Already a hedge (from a deeper decorator) or a reject — pass
            // straight through.
            other => return other,
        };

        let quantile_ms = {
            let estimators = self.estimators.lock().expect("hedge estimators poisoned");
            estimators
                .get(&primary)
                .filter(|e| e.count() >= self.cfg.warmup_observations)
                .and_then(P2Estimator::quantile)
        };

        let Some(q_ms) = quantile_ms else {
            // Warm-up: no hedge.
            return RoutingDecision::Send(primary);
        };
        if (q_ms as f32) < self.cfg.hedge_min_threshold_ms {
            return RoutingDecision::Send(primary);
        }
        if !self.budget_allows() {
            self.budget_blocked.fetch_add(1, Ordering::Relaxed);
            return RoutingDecision::Send(primary);
        }

        match self.pick_secondary(primary, pool, signals) {
            Some(secondary) => {
                self.hedges_total.fetch_add(1, Ordering::Relaxed);
                RoutingDecision::Hedge(vec![primary, secondary])
            }
            None => RoutingDecision::Send(primary),
        }
    }

    fn on_response(&self, decision: &RoutingDecision, outcome: &Outcome) {
        // Update the P² estimator for the backend that handled the
        // request with the wall-clock duration we observed.
        let backend = outcome.backend;
        let ms = outcome.duration.as_secs_f64() * 1000.0;
        let mut estimators = self.estimators.lock().expect("hedge estimators poisoned");
        estimators
            .entry(backend)
            .or_insert_with(|| P2Estimator::new(self.cfg.hedge_after_quantile))
            .observe(ms);
        drop(estimators);
        self.inner.on_response(decision, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::request::{Body, Headers, Method, Request, StatusCode};
    use riftgate_core::types::RequestId;
    use std::time::Duration;

    struct FixedRouter(BackendId);

    impl Router for FixedRouter {
        fn route(
            &self,
            _req: &Request,
            _pool: &BackendPool,
            _signals: &BackendSignals,
        ) -> RoutingDecision {
            RoutingDecision::Send(self.0)
        }
    }

    fn make_req() -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/v1/chat/completions".to_string(),
            headers: Headers::new(),
            body: Body::Bytes(b"hi".to_vec()),
        }
    }

    fn outcome_for(b: BackendId, ms: u64) -> Outcome {
        Outcome {
            backend: b,
            status: Some(StatusCode(200)),
            duration: Duration::from_millis(ms),
            ok: true,
        }
    }

    fn dummy_pool_and_signals(n: usize) -> (BackendPool, BackendSignals) {
        let ids: Vec<BackendId> = (0..n as u16).map(BackendId).collect();
        (BackendPool::from_ids(ids), BackendSignals::default())
    }

    #[test]
    fn warmup_does_not_hedge() {
        let router = HedgedRouter::new(FixedRouter(BackendId(0)), HedgedConfig::default());
        let (pool, signals) = dummy_pool_and_signals(3);
        // No on_response calls yet — estimator empty.
        let dec = router.route(&make_req(), &pool, &signals);
        assert!(matches!(dec, RoutingDecision::Send(b) if b == BackendId(0)));
        assert_eq!(router.stats().hedges_fired, 0);
    }

    #[test]
    fn slow_backend_hedges_after_warmup() {
        let cfg = HedgedConfig {
            hedge_min_threshold_ms: 5.0,
            warmup_observations: 10,
            hedge_max_fraction: 1.0,
            ..HedgedConfig::default()
        };
        let router = HedgedRouter::new(FixedRouter(BackendId(0)), cfg);
        let (pool, signals) = dummy_pool_and_signals(3);
        // Feed observations modelling a slow primary.
        for _ in 0..50 {
            router.on_response(
                &RoutingDecision::Send(BackendId(0)),
                &outcome_for(BackendId(0), 200),
            );
        }
        let dec = router.route(&make_req(), &pool, &signals);
        assert!(matches!(dec, RoutingDecision::Hedge(_)));
        assert!(router.stats().hedges_fired > 0);
    }

    #[test]
    fn fast_backend_does_not_hedge() {
        let cfg = HedgedConfig {
            hedge_min_threshold_ms: 100.0,
            warmup_observations: 10,
            ..HedgedConfig::default()
        };
        let router = HedgedRouter::new(FixedRouter(BackendId(0)), cfg);
        let (pool, signals) = dummy_pool_and_signals(3);
        for _ in 0..50 {
            router.on_response(
                &RoutingDecision::Send(BackendId(0)),
                &outcome_for(BackendId(0), 5),
            );
        }
        let dec = router.route(&make_req(), &pool, &signals);
        assert!(matches!(dec, RoutingDecision::Send(_)));
    }

    #[test]
    fn budget_cap_limits_hedge_fraction() {
        let cfg = HedgedConfig {
            hedge_min_threshold_ms: 1.0,
            warmup_observations: 5,
            hedge_max_fraction: 0.10,
            ..HedgedConfig::default()
        };
        let router = HedgedRouter::new(FixedRouter(BackendId(0)), cfg);
        let (pool, signals) = dummy_pool_and_signals(3);
        for _ in 0..20 {
            router.on_response(
                &RoutingDecision::Send(BackendId(0)),
                &outcome_for(BackendId(0), 500),
            );
        }
        for _ in 0..200 {
            let _ = router.route(&make_req(), &pool, &signals);
        }
        let s = router.stats();
        assert!(
            (s.hedges_fired as f64 / s.decisions as f64) <= 0.11,
            "hedge fraction exceeded budget: {s:?}"
        );
        assert!(s.budget_blocked > 0);
    }

    #[test]
    fn p2_estimator_initialization_phase() {
        let mut est = P2Estimator::new(0.95);
        for v in [10.0, 30.0, 20.0, 50.0].iter().copied() {
            est.observe(v);
        }
        assert!(est.quantile().is_none(), "need 5 samples to bootstrap");
        est.observe(40.0);
        assert!(est.quantile().is_some());
    }

    #[test]
    fn p2_estimator_approximates_p95_of_distribution() {
        let mut est = P2Estimator::new(0.95);
        // Feed 1000 samples uniform on [0, 100]. True p95 = 95.
        for i in 0..1000u32 {
            let v = (i % 100) as f64;
            est.observe(v);
        }
        let q = est.quantile().expect("quantile after 1000 samples");
        assert!(q > 80.0 && q < 100.0, "p95 estimate out of range: {q}");
    }
}
