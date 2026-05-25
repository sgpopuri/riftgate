//! Weighted-random router using the Walker alias method.
//!
//! Per [ADR 0014](../../../../docs/06-adrs/0014-weighted-random-router.md)
//! the v0.2 weighted-routing impl is Vose's alias method (1991), which
//! samples from a discrete distribution in O(1) time after an O(N) table
//! build at config-load time. The table is rebuilt on configuration
//! reload; the steady-state hot path is one PRNG step and one array
//! lookup.
//!
//! Capped at N = 32 eligible backends per the ADR.
//!
//! Backends with `CircuitState::Open` are skipped at route time; if every
//! backend is open the router returns
//! `RoutingDecision::Reject(StatusCode::ServiceUnavailable)`.

use core::sync::atomic::{AtomicU64, Ordering};
use riftgate_core::request::{Request, StatusCode};
use riftgate_core::router::{
    BackendId, BackendPool, BackendSignals, CircuitState, Router, RoutingDecision,
};

/// Maximum number of eligible backends a `WeightedRandomRouter` honors.
/// Per [ADR 0014](../../../../docs/06-adrs/0014-weighted-random-router.md).
pub const MAX_WEIGHTED_BACKENDS: usize = 32;

/// One entry in the Walker alias table.
#[derive(Debug, Copy, Clone)]
struct AliasEntry {
    /// Backend id at this primary slot.
    primary: BackendId,
    /// Backend id at the alias slot (used when the PRNG sample exceeds
    /// `prob_scaled`).
    alias: BackendId,
    /// Probability of picking `primary`, scaled to `[0, ALIAS_SCALE]`.
    prob_scaled: u32,
}

const ALIAS_SCALE: u32 = 1 << 24;

/// Walker alias method router.
///
/// Construct via [`WeightedRandomRouter::new`] with `(BackendId, weight)`
/// pairs. Weights must be positive integers. The router takes O(N) to
/// build the table and O(1) per `route` call.
pub struct WeightedRandomRouter {
    table: Vec<AliasEntry>,
    backends: Vec<BackendId>,
    rng_state: AtomicU64,
}

impl core::fmt::Debug for WeightedRandomRouter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WeightedRandomRouter")
            .field("backends", &self.backends)
            .finish()
    }
}

impl WeightedRandomRouter {
    /// Build a router from `(backend, weight)` pairs.
    ///
    /// # Panics
    /// Panics if `weights` is empty, exceeds [`MAX_WEIGHTED_BACKENDS`],
    /// contains a zero weight, or if the total weight overflows `u64`.
    #[must_use]
    pub fn new(weights: &[(BackendId, u32)]) -> Self {
        Self::with_seed(weights, splitmix64(0xCAFE_BABE_DEAD_BEEF))
    }

    /// Construct with a caller-supplied PRNG seed. Tests use this for
    /// determinism.
    #[must_use]
    pub fn with_seed(weights: &[(BackendId, u32)], seed: u64) -> Self {
        assert!(
            !weights.is_empty(),
            "WeightedRandomRouter requires at least one backend"
        );
        assert!(
            weights.len() <= MAX_WEIGHTED_BACKENDS,
            "WeightedRandomRouter exceeds MAX_WEIGHTED_BACKENDS"
        );
        let n = weights.len();
        let total: u64 = weights
            .iter()
            .map(|(_, w)| {
                assert!(*w > 0, "WeightedRandomRouter requires positive weights");
                u64::from(*w)
            })
            .sum();
        assert!(total > 0, "WeightedRandomRouter total weight must be > 0");

        // Vose's algorithm. Compute per-bucket scaled probability
        // p_i = (weight_i * n) / total, in fixed point.
        let mut scaled: Vec<u64> = weights
            .iter()
            .map(|(_, w)| (u64::from(*w) * (n as u64) * u64::from(ALIAS_SCALE)) / total)
            .collect();

        let mut small: Vec<usize> = Vec::with_capacity(n);
        let mut large: Vec<usize> = Vec::with_capacity(n);
        let scale_u64 = u64::from(ALIAS_SCALE);
        for (i, &s) in scaled.iter().enumerate() {
            if s < scale_u64 {
                small.push(i);
            } else {
                large.push(i);
            }
        }

        let mut table: Vec<AliasEntry> = (0..n)
            .map(|i| AliasEntry {
                primary: weights[i].0,
                alias: weights[i].0,
                prob_scaled: ALIAS_SCALE,
            })
            .collect();

        while let (Some(s), Some(l)) = (small.pop(), large.pop()) {
            table[s] = AliasEntry {
                primary: weights[s].0,
                alias: weights[l].0,
                prob_scaled: scaled[s] as u32,
            };
            let new_large = scaled[l] + scaled[s] - scale_u64;
            scaled[l] = new_large;
            if new_large < scale_u64 {
                small.push(l);
            } else {
                large.push(l);
            }
        }
        // Any leftovers settle at prob = 1.
        for i in large.drain(..).chain(small.drain(..)) {
            table[i] = AliasEntry {
                primary: weights[i].0,
                alias: weights[i].0,
                prob_scaled: ALIAS_SCALE,
            };
        }

        Self {
            table,
            backends: weights.iter().map(|(b, _)| *b).collect(),
            rng_state: AtomicU64::new(seed.max(1)),
        }
    }

    /// Drive the xorshift64* PRNG once.
    fn next_u64(&self) -> u64 {
        // We use atomic CAS only to ensure cross-thread visibility; on
        // contention we accept a small bias toward older samples by
        // retrying with the freshly observed state.
        loop {
            let s = self.rng_state.load(Ordering::Relaxed).max(1);
            let next = xorshift64(s);
            if self
                .rng_state
                .compare_exchange(s, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    fn sample(&self) -> BackendId {
        let r = self.next_u64();
        let n = self.table.len();
        let bucket = (r as usize) % n;
        let coin = ((r >> 32) as u32) & (ALIAS_SCALE - 1);
        let entry = self.table[bucket];
        if coin < entry.prob_scaled {
            entry.primary
        } else {
            entry.alias
        }
    }
}

impl Router for WeightedRandomRouter {
    fn route(
        &self,
        _req: &Request,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> RoutingDecision {
        if pool.is_empty() {
            return RoutingDecision::Reject(StatusCode::BAD_GATEWAY);
        }
        // Try up to N samples; if every draw is on an open circuit,
        // fall through to a linear scan to confirm everyone is open.
        let n = self.backends.len();
        for _ in 0..n {
            let b = self.sample();
            if !matches!(signals.get(b).circuit_state, CircuitState::Open) {
                return RoutingDecision::Send(b);
            }
        }
        // Linear scan as a safety net.
        for b in pool.iter() {
            if !matches!(signals.get(b).circuit_state, CircuitState::Open) {
                return RoutingDecision::Send(b);
            }
        }
        RoutingDecision::Reject(StatusCode::SERVICE_UNAVAILABLE)
    }
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut t = z;
    t = (t ^ (t >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    t = (t ^ (t >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    t ^ (t >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::request::{Body, Headers, Method};
    use riftgate_core::router::BackendSignal;
    use riftgate_core::types::RequestId;
    use std::collections::HashMap;

    fn dummy_request() -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/v1/chat/completions".into(),
            headers: Headers::new(),
            body: Body::Empty,
        }
    }

    #[test]
    fn weights_sample_proportionally() {
        // Backend 0 weight 1, backend 1 weight 3 => 25%/75%.
        let r =
            WeightedRandomRouter::with_seed(&[(BackendId(0), 1), (BackendId(1), 3)], 0xABCD_EF01);
        let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1)]);
        let signals =
            BackendSignals::from_vec(vec![BackendSignal::default(), BackendSignal::default()]);

        let mut counts: HashMap<BackendId, u32> = HashMap::new();
        for _ in 0..10_000 {
            if let RoutingDecision::Send(b) = r.route(&dummy_request(), &pool, &signals) {
                *counts.entry(b).or_default() += 1;
            }
        }
        let c0 = *counts.get(&BackendId(0)).unwrap_or(&0);
        let c1 = *counts.get(&BackendId(1)).unwrap_or(&0);
        // Expect ~2500 / ~7500 with ±10% slack.
        assert!((2000..3000).contains(&c0), "backend 0 count = {c0}");
        assert!((7000..8000).contains(&c1), "backend 1 count = {c1}");
    }

    #[test]
    fn open_backend_is_skipped() {
        let r = WeightedRandomRouter::with_seed(&[(BackendId(0), 1), (BackendId(1), 1)], 42);
        let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1)]);
        let signals = BackendSignals::from_vec(vec![
            BackendSignal {
                circuit_state: CircuitState::Open,
                ..BackendSignal::default()
            },
            BackendSignal::default(),
        ]);
        for _ in 0..50 {
            match r.route(&dummy_request(), &pool, &signals) {
                RoutingDecision::Send(b) => assert_eq!(b, BackendId(1)),
                other => panic!("expected Send(1), got {other:?}"),
            }
        }
    }

    #[test]
    fn all_open_returns_service_unavailable() {
        let r = WeightedRandomRouter::with_seed(&[(BackendId(0), 1), (BackendId(1), 1)], 7);
        let pool = BackendPool::from_ids(vec![BackendId(0), BackendId(1)]);
        let signals = BackendSignals::from_vec(vec![
            BackendSignal {
                circuit_state: CircuitState::Open,
                ..BackendSignal::default()
            },
            BackendSignal {
                circuit_state: CircuitState::Open,
                ..BackendSignal::default()
            },
        ]);
        match r.route(&dummy_request(), &pool, &signals) {
            RoutingDecision::Reject(s) => assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE),
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn empty_pool_returns_bad_gateway() {
        let r = WeightedRandomRouter::with_seed(&[(BackendId(0), 1)], 1);
        let pool = BackendPool::default();
        let signals = BackendSignals::default();
        match r.route(&dummy_request(), &pool, &signals) {
            RoutingDecision::Reject(s) => assert_eq!(s, StatusCode::BAD_GATEWAY),
            other => panic!("expected Reject(BAD_GATEWAY), got {other:?}"),
        }
    }
}
