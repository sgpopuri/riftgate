//! Per-backend 3-state circuit breaker, packaged as a `Router` decorator.
//!
//! Per [ADR 0016](../../../../docs/06-adrs/0016-three-state-circuit-breaker.md):
//!
//! - One breaker state machine per backend (`Closed` -> `Open` -> `HalfOpen`).
//! - `Closed` is the healthy default. Counts consecutive failures; trips to
//!   `Open` when `failure_threshold` consecutive failures hit.
//! - `Open` rejects every routing decision for `reset_timeout`, then moves
//!   to `HalfOpen`.
//! - `HalfOpen` admits at most `half_open_max` probe requests in flight.
//!   The first probe success closes the breaker; any failure re-opens it.
//!
//! Failure dimension (per the ADR): 5xx responses, 408, 504, and any IO
//! timeout count as failure. 4xx responses other than 408/429 do *not*
//! count: a 400 Bad Request is a client problem, not a backend problem.
//! 429 from a backend is treated as backend-pressure and does count.
//!
//! Composes with any other `Router` impl by wrapping it: the decorator
//! injects synthesized signals into the downstream `route` call so the
//! inner router naturally avoids `Open` backends. On `on_response`, the
//! decorator updates breaker state and forwards to the inner router.

use core::sync::atomic::{AtomicU64, Ordering};
use riftgate_core::request::{Request, StatusCode};
use riftgate_core::router::{
    BackendId, BackendPool, BackendSignal, BackendSignals, CircuitState, Outcome, Router,
    RoutingDecision,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuration for a [`CircuitBreakerArbiter`].
#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerConfig {
    /// Consecutive failure count that trips `Closed` -> `Open`.
    pub failure_threshold: u32,
    /// Duration the breaker stays `Open` before transitioning to
    /// `HalfOpen`.
    pub reset_timeout: Duration,
    /// Maximum number of concurrent probes admitted while `HalfOpen`.
    pub half_open_max: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            half_open_max: 1,
        }
    }
}

/// Packed per-backend state: `[u32 state_tag | u32 consecutive_failures]`.
/// `state_tag` is `0 = Closed`, `1 = Open`, `2 = HalfOpen`. The
/// `transitioned_at` instant is kept in a sibling mutex-protected map
/// keyed by backend; transitions are cold relative to `route`.
fn pack_state(tag: u32, failures: u32) -> u64 {
    (u64::from(tag) << 32) | u64::from(failures)
}

fn unpack_state(s: u64) -> (u32, u32) {
    ((s >> 32) as u32, (s & 0xFFFF_FFFF) as u32)
}

const TAG_CLOSED: u32 = 0;
const TAG_OPEN: u32 = 1;
const TAG_HALF_OPEN: u32 = 2;

#[derive(Debug)]
struct BackendBreaker {
    state: AtomicU64,
    half_open_inflight: AtomicU64,
}

impl BackendBreaker {
    fn new() -> Self {
        Self {
            state: AtomicU64::new(pack_state(TAG_CLOSED, 0)),
            half_open_inflight: AtomicU64::new(0),
        }
    }
}

/// `Router` decorator that enforces a per-backend 3-state circuit breaker.
///
/// Wraps any inner [`Router`] impl. The decorator owns the breaker state;
/// the inner router observes the breaker-derived [`CircuitState`] via the
/// `signals` passed into [`Router::route`], so a `WeightedRandomRouter` or
/// future `KvAwareRouter` does not need to know about the breaker.
pub struct CircuitBreakerArbiter<R> {
    inner: R,
    cfg: CircuitBreakerConfig,
    breakers: Mutex<HashMap<BackendId, BackendBreaker>>,
    transitioned_at: Mutex<HashMap<BackendId, Instant>>,
}

impl<R: core::fmt::Debug> core::fmt::Debug for CircuitBreakerArbiter<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CircuitBreakerArbiter")
            .field("inner", &self.inner)
            .field("cfg", &self.cfg)
            .finish()
    }
}

impl<R: Router> CircuitBreakerArbiter<R> {
    /// Wrap `inner` with a circuit breaker using `cfg`.
    #[must_use]
    pub fn new(inner: R, cfg: CircuitBreakerConfig) -> Self {
        Self {
            inner,
            cfg,
            breakers: Mutex::new(HashMap::new()),
            transitioned_at: Mutex::new(HashMap::new()),
        }
    }

    /// Current state of the breaker for `backend`. Test helper.
    #[must_use]
    pub fn state_of(&self, backend: BackendId) -> CircuitState {
        let breakers = self.breakers.lock().expect("breakers mutex");
        match breakers.get(&backend) {
            Some(b) => tag_to_state(unpack_state(b.state.load(Ordering::Acquire)).0),
            None => CircuitState::Closed,
        }
    }

    fn ensure_backend(&self, backend: BackendId) {
        let mut breakers = self.breakers.lock().expect("breakers mutex");
        breakers.entry(backend).or_insert_with(BackendBreaker::new);
    }

    fn maybe_transition_open_to_halfopen(&self, backend: BackendId) {
        let now = Instant::now();
        let mut transitioned = self.transitioned_at.lock().expect("transitioned mutex");
        let breakers = self.breakers.lock().expect("breakers mutex");
        if let Some(b) = breakers.get(&backend) {
            let (tag, _) = unpack_state(b.state.load(Ordering::Acquire));
            if tag == TAG_OPEN {
                if let Some(&t) = transitioned.get(&backend) {
                    if now.duration_since(t) >= self.cfg.reset_timeout {
                        b.state
                            .store(pack_state(TAG_HALF_OPEN, 0), Ordering::Release);
                        b.half_open_inflight.store(0, Ordering::Release);
                        transitioned.insert(backend, now);
                    }
                }
            }
        }
    }

    fn build_signals(&self, pool: &BackendPool, signals: &BackendSignals) -> BackendSignals {
        let max_id = pool.iter().map(|b| b.0 as usize).max().unwrap_or(0);
        let mut out: Vec<BackendSignal> = (0..=max_id).map(|_| BackendSignal::default()).collect();
        // Preserve any non-breaker signals from the caller.
        for b in pool.iter() {
            out[b.0 as usize] = signals.get(b);
        }
        let breakers = self.breakers.lock().expect("breakers mutex");
        for b in pool.iter() {
            if let Some(br) = breakers.get(&b) {
                let (tag, _) = unpack_state(br.state.load(Ordering::Acquire));
                out[b.0 as usize].circuit_state = tag_to_state(tag);
                if tag == TAG_HALF_OPEN
                    && br.half_open_inflight.load(Ordering::Acquire)
                        >= u64::from(self.cfg.half_open_max)
                {
                    // Probe budget exhausted: treat as Open for this
                    // routing call.
                    out[b.0 as usize].circuit_state = CircuitState::Open;
                }
            }
        }
        BackendSignals::from_vec(out)
    }

    fn record_outcome(&self, outcome: &Outcome) {
        let backend = outcome.backend;
        self.ensure_backend(backend);
        let counts_as_failure = !outcome.ok || outcome.status.is_some_and(counts_as_failure);
        let breakers = self.breakers.lock().expect("breakers mutex");
        let Some(br) = breakers.get(&backend) else {
            return;
        };
        let mut transitioned = self.transitioned_at.lock().expect("transitioned mutex");
        let now = Instant::now();
        let (tag, failures) = unpack_state(br.state.load(Ordering::Acquire));
        match (tag, counts_as_failure) {
            (TAG_CLOSED, true) => {
                let next = failures + 1;
                if next >= self.cfg.failure_threshold {
                    br.state.store(pack_state(TAG_OPEN, 0), Ordering::Release);
                    transitioned.insert(backend, now);
                } else {
                    br.state
                        .store(pack_state(TAG_CLOSED, next), Ordering::Release);
                }
            }
            (TAG_CLOSED, false) if failures > 0 => {
                br.state.store(pack_state(TAG_CLOSED, 0), Ordering::Release);
            }
            (TAG_HALF_OPEN, true) => {
                br.state.store(pack_state(TAG_OPEN, 0), Ordering::Release);
                br.half_open_inflight.store(0, Ordering::Release);
                transitioned.insert(backend, now);
            }
            (TAG_HALF_OPEN, false) => {
                br.state.store(pack_state(TAG_CLOSED, 0), Ordering::Release);
                br.half_open_inflight.store(0, Ordering::Release);
            }
            // Outcomes that arrive while Open are observed but do not
            // mutate state — they likely came from the inner router's
            // synthesized reject path.
            _ => {}
        }
    }
}

fn counts_as_failure(s: StatusCode) -> bool {
    // 5xx are always failures. 408 Request Timeout and 504 Gateway
    // Timeout and 429 (backend-pressure) count. Other 4xx are client
    // problems and do not.
    if s.0 >= 500 {
        return true;
    }
    matches!(s.0, 408 | 429)
}

fn tag_to_state(tag: u32) -> CircuitState {
    match tag {
        TAG_OPEN => CircuitState::Open,
        TAG_HALF_OPEN => CircuitState::HalfOpen,
        _ => CircuitState::Closed,
    }
}

impl<R: Router> Router for CircuitBreakerArbiter<R> {
    fn route(
        &self,
        req: &Request,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> RoutingDecision {
        for b in pool.iter() {
            self.ensure_backend(b);
            self.maybe_transition_open_to_halfopen(b);
        }
        let merged = self.build_signals(pool, signals);
        let decision = self.inner.route(req, pool, &merged);
        if let RoutingDecision::Send(backend) = &decision {
            // Count a half-open probe.
            let breakers = self.breakers.lock().expect("breakers mutex");
            if let Some(br) = breakers.get(backend) {
                let (tag, _) = unpack_state(br.state.load(Ordering::Acquire));
                if tag == TAG_HALF_OPEN {
                    br.half_open_inflight.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
        decision
    }

    fn on_response(&self, decision: &RoutingDecision, outcome: &Outcome) {
        self.record_outcome(outcome);
        self.inner.on_response(decision, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstantRouter;
    use riftgate_core::request::{Body, Headers, Method};
    use riftgate_core::types::RequestId;

    fn dummy_request() -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/v1/chat/completions".into(),
            headers: Headers::new(),
            body: Body::Empty,
        }
    }

    fn fail_outcome(b: BackendId) -> Outcome {
        Outcome {
            backend: b,
            status: Some(StatusCode(503)),
            duration: Duration::from_millis(1),
            ok: false,
        }
    }

    fn ok_outcome(b: BackendId) -> Outcome {
        Outcome {
            backend: b,
            status: Some(StatusCode::OK),
            duration: Duration::from_millis(1),
            ok: true,
        }
    }

    #[test]
    fn trips_open_after_threshold_failures() {
        let inner = ConstantRouter::new(BackendId(0));
        let cb = CircuitBreakerArbiter::new(
            inner,
            CircuitBreakerConfig {
                failure_threshold: 3,
                reset_timeout: Duration::from_millis(50),
                half_open_max: 1,
            },
        );
        let backend = BackendId(0);
        assert_eq!(cb.state_of(backend), CircuitState::Closed);
        for _ in 0..3 {
            cb.on_response(&RoutingDecision::Send(backend), &fail_outcome(backend));
        }
        assert_eq!(cb.state_of(backend), CircuitState::Open);
    }

    #[test]
    fn successes_reset_failure_count() {
        let cb = CircuitBreakerArbiter::new(
            ConstantRouter::new(BackendId(0)),
            CircuitBreakerConfig {
                failure_threshold: 3,
                reset_timeout: Duration::from_secs(1),
                half_open_max: 1,
            },
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &ok_outcome(BackendId(0)),
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        // Still closed: success reset the count.
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::Closed);
    }

    #[test]
    fn half_open_then_close_on_success() {
        let cb = CircuitBreakerArbiter::new(
            ConstantRouter::new(BackendId(0)),
            CircuitBreakerConfig {
                failure_threshold: 2,
                reset_timeout: Duration::from_millis(10),
                half_open_max: 1,
            },
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::Open);
        std::thread::sleep(Duration::from_millis(20));
        // Calling route triggers the open->half_open transition check.
        let pool = BackendPool::from_ids(vec![BackendId(0)]);
        let signals = BackendSignals::from_vec(vec![BackendSignal::default()]);
        let _ = cb.route(&dummy_request(), &pool, &signals);
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::HalfOpen);
        // Success while half-open closes.
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &ok_outcome(BackendId(0)),
        );
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::Closed);
    }

    #[test]
    fn half_open_failure_reopens() {
        let cb = CircuitBreakerArbiter::new(
            ConstantRouter::new(BackendId(0)),
            CircuitBreakerConfig {
                failure_threshold: 2,
                reset_timeout: Duration::from_millis(5),
                half_open_max: 1,
            },
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        std::thread::sleep(Duration::from_millis(15));
        let pool = BackendPool::from_ids(vec![BackendId(0)]);
        let signals = BackendSignals::from_vec(vec![BackendSignal::default()]);
        let _ = cb.route(&dummy_request(), &pool, &signals);
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::HalfOpen);
        cb.on_response(
            &RoutingDecision::Send(BackendId(0)),
            &fail_outcome(BackendId(0)),
        );
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::Open);
    }

    #[test]
    fn four_hundred_does_not_count_as_failure() {
        let cb = CircuitBreakerArbiter::new(
            ConstantRouter::new(BackendId(0)),
            CircuitBreakerConfig {
                failure_threshold: 2,
                reset_timeout: Duration::from_secs(1),
                half_open_max: 1,
            },
        );
        let bad_request = Outcome {
            backend: BackendId(0),
            status: Some(StatusCode::BAD_REQUEST),
            duration: Duration::from_millis(1),
            ok: true,
        };
        for _ in 0..10 {
            cb.on_response(&RoutingDecision::Send(BackendId(0)), &bad_request);
        }
        assert_eq!(cb.state_of(BackendId(0)), CircuitState::Closed);
    }
}
