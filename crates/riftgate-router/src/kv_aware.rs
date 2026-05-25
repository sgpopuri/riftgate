//! KV-cache-aware router — v0.3 decorator over an inner `Router`.
//!
//! Per [ADR 0022](../../../../docs/06-adrs/0022-kv-aware-routing-prefix-trie.md)
//! and [Options 025 §3.A.1](../../../../docs/05-options/025-v03-routing-strategies.md):
//!
//! - Maintains an in-tree prefix trie keyed by chunked `xxHash3-64` hashes
//!   of the request's prompt bytes.
//! - On `route`: hashes the request body in `prefix_chunk_bytes`-sized
//!   chunks, walks the trie to the longest prefix that already names a
//!   backend, and returns `Send(backend)` if that backend is still
//!   `CircuitState::Closed`. Otherwise falls back to the inner router and
//!   *records* the inner decision into the trie so the next request with
//!   the same prefix benefits from the just-warmed KV cache.
//! - Capacity-bounded: when the trie holds more than `max_trie_entries`
//!   backend-bearing nodes, the trie is cleared in O(1) (full-flush
//!   eviction). True LRU eviction is a known follow-up; the v0.3 design
//!   trades that complexity for simplicity per [ADR 0022 §Notes].
//!
//! **Threading.** The trie sits behind a single `RwLock<PrefixTrie>`. Reads
//! (`route` hot path) take the read lock briefly; writes (insertion on a
//! routing miss, full-flush on overflow) take the write lock briefly.
//! Contention is documented in the LLD.
//!
//! ## Decorator data flow
//!
//! ```text
//!   route(req, pool, signals):
//!     if body.len() < min_prefix_bytes_to_route:
//!         return inner.route(...)
//!
//!     hashes  = chunked_xxh3(body, prefix_chunk_bytes)
//!     hit     = trie.longest_match(hashes)
//!
//!     if let Some(b) = hit:
//!         if signals[b].circuit_state == Closed:
//!             return Send(b)             // KV-cache fast path
//!
//!     // Miss or breaker-rejected: defer to inner.
//!     dec = inner.route(req, pool, signals)
//!     if let Send(b) = dec:
//!         trie.insert(hashes, b)
//!     dec
//! ```

use core::sync::atomic::{AtomicU64, Ordering};
use riftgate_core::request::{Body, Request};
use riftgate_core::router::{
    BackendId, BackendPool, BackendSignals, CircuitState, Outcome, Router, RoutingDecision,
};
use std::collections::HashMap;
use std::sync::RwLock;
use xxhash_rust::xxh3::xxh3_64;

/// Configuration knobs for [`KvAwareRouter`]. Per [ADR 0022].
#[derive(Debug, Clone, Copy)]
pub struct KvAwareConfig {
    /// Bytes per trie level. Default 64.
    pub prefix_chunk_bytes: usize,
    /// Maximum number of backend-bearing nodes the trie holds before a
    /// full-flush eviction. Default 100 000.
    pub max_trie_entries: usize,
    /// Requests with bodies shorter than this fall through to the inner
    /// router without consulting the trie. Default 256.
    pub min_prefix_bytes_to_route: usize,
}

impl Default for KvAwareConfig {
    fn default() -> Self {
        Self {
            prefix_chunk_bytes: 64,
            max_trie_entries: 100_000,
            min_prefix_bytes_to_route: 256,
        }
    }
}

/// Node in the prefix trie. `backend` is `Some` at any prefix endpoint
/// recorded by a prior routing decision; the longest-match walk terminates
/// at the deepest `Some` it reaches.
#[derive(Debug, Default)]
struct TrieNode {
    backend: Option<BackendId>,
    children: HashMap<u64, Box<TrieNode>>,
}

/// In-tree prefix trie keyed by chunked xxHash3-64 hashes.
#[derive(Debug)]
struct PrefixTrie {
    root: TrieNode,
    /// Count of `backend`-bearing nodes. Bounded by `cap`; on overflow the
    /// trie is fully cleared.
    count: usize,
    cap: usize,
}

impl PrefixTrie {
    fn new(cap: usize) -> Self {
        Self {
            root: TrieNode::default(),
            count: 0,
            cap,
        }
    }

    fn insert(&mut self, hashes: &[u64], backend: BackendId) {
        if hashes.is_empty() {
            return;
        }
        if self.count >= self.cap {
            // Full-flush eviction per ADR 0022. Documented simplification;
            // a true LRU would touch and evict per-node.
            self.root = TrieNode::default();
            self.count = 0;
        }
        let mut node = &mut self.root;
        for &h in hashes {
            node = node.children.entry(h).or_default();
        }
        if node.backend.is_none() {
            self.count += 1;
        }
        node.backend = Some(backend);
    }

    fn longest_match(&self, hashes: &[u64]) -> Option<BackendId> {
        let mut node = &self.root;
        let mut best: Option<BackendId> = None;
        for &h in hashes {
            match node.children.get(&h) {
                Some(next) => {
                    node = next;
                    if node.backend.is_some() {
                        best = node.backend;
                    }
                }
                None => break,
            }
        }
        best
    }

    fn entry_count(&self) -> usize {
        self.count
    }
}

/// KV-cache-aware decorator router. See module docs and ADR 0022.
pub struct KvAwareRouter<R> {
    inner: R,
    cfg: KvAwareConfig,
    trie: RwLock<PrefixTrie>,
    /// Telemetry counters surfaced via [`KvAwareRouter::stats`].
    hits: AtomicU64,
    misses: AtomicU64,
    breaker_rejections: AtomicU64,
}

impl<R: core::fmt::Debug> core::fmt::Debug for KvAwareRouter<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KvAwareRouter")
            .field("inner", &self.inner)
            .field("cfg", &self.cfg)
            .finish()
    }
}

/// Hit / miss counters for [`KvAwareRouter`]. Stable for observability.
#[derive(Debug, Default, Copy, Clone)]
pub struct KvAwareStats {
    /// Times the trie returned a `Closed` backend on `route`.
    pub hits: u64,
    /// Times the trie did not name a backend (or the body was too short).
    pub misses: u64,
    /// Times the trie named a backend whose breaker was open. Falls
    /// through to inner.
    pub breaker_rejections: u64,
    /// Current backend-bearing-node count in the trie.
    pub entries: usize,
}

impl<R: Router> KvAwareRouter<R> {
    /// Wrap `inner` with a `KvAwareRouter` using `cfg`.
    #[must_use]
    pub fn new(inner: R, cfg: KvAwareConfig) -> Self {
        Self {
            inner,
            trie: RwLock::new(PrefixTrie::new(cfg.max_trie_entries)),
            cfg,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            breaker_rejections: AtomicU64::new(0),
        }
    }

    /// Snapshot of routing-decision statistics. Useful in tests and in
    /// `/metrics`-style introspection.
    #[must_use]
    pub fn stats(&self) -> KvAwareStats {
        let trie = self.trie.read().expect("kv-aware trie poisoned");
        KvAwareStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            breaker_rejections: self.breaker_rejections.load(Ordering::Relaxed),
            entries: trie.entry_count(),
        }
    }

    fn body_hashes(&self, body: &[u8]) -> Vec<u64> {
        let chunk = self.cfg.prefix_chunk_bytes.max(1);
        body.chunks(chunk).map(xxh3_64).collect()
    }
}

impl<R: Router> Router for KvAwareRouter<R> {
    fn route(
        &self,
        req: &Request,
        pool: &BackendPool,
        signals: &BackendSignals,
    ) -> RoutingDecision {
        let body_bytes: &[u8] = match &req.body {
            Body::Empty => &[],
            Body::Bytes(v) => v.as_slice(),
        };
        if body_bytes.len() < self.cfg.min_prefix_bytes_to_route {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return self.inner.route(req, pool, signals);
        }
        let hashes = self.body_hashes(body_bytes);

        // Read-side: check the trie under a read lock.
        let hit = {
            let trie = self.trie.read().expect("kv-aware trie poisoned");
            trie.longest_match(&hashes)
        };

        if let Some(b) = hit {
            if matches!(signals.get(b).circuit_state, CircuitState::Closed) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return RoutingDecision::Send(b);
            }
            self.breaker_rejections.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }

        // Miss or breaker-rejected: defer to inner and record the inner
        // decision for the next request with the same prefix.
        let decision = self.inner.route(req, pool, signals);
        if let RoutingDecision::Send(b) = decision {
            let mut trie = self.trie.write().expect("kv-aware trie poisoned");
            trie.insert(&hashes, b);
        }
        decision
    }

    fn on_response(&self, decision: &RoutingDecision, outcome: &Outcome) {
        self.inner.on_response(decision, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::request::{Body, Headers, Method, Request};
    use riftgate_core::types::RequestId;

    /// Inner router that always returns a fixed backend. Lets us seed the
    /// trie deterministically.
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

    fn make_req(body: Vec<u8>) -> Request {
        Request {
            id: RequestId(1),
            method: Method::Post,
            path: "/v1/chat/completions".to_string(),
            headers: Headers::new(),
            body: Body::Bytes(body),
        }
    }

    fn long_body(prefix: u8) -> Vec<u8> {
        // 300 bytes > default min_prefix_bytes_to_route (256).
        let mut body = vec![prefix; 300];
        body[5] = b':';
        body
    }

    #[test]
    fn body_shorter_than_min_bypasses_trie() {
        let router = KvAwareRouter::new(FixedRouter(BackendId(7)), KvAwareConfig::default());
        let pool = BackendPool::from_ids(vec![BackendId(7)]);
        let signals = BackendSignals::new();
        let dec = router.route(&make_req(b"hi".to_vec()), &pool, &signals);
        assert!(matches!(dec, RoutingDecision::Send(b) if b == BackendId(7)));
        let stats = router.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 0);
    }

    #[test]
    fn first_call_misses_and_inserts_into_trie() {
        let router = KvAwareRouter::new(FixedRouter(BackendId(3)), KvAwareConfig::default());
        let pool = BackendPool::from_ids(vec![BackendId(3)]);
        let signals = BackendSignals::new();
        let dec = router.route(&make_req(long_body(b'a')), &pool, &signals);
        assert!(matches!(dec, RoutingDecision::Send(b) if b == BackendId(3)));
        let stats = router.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
        assert!(stats.entries > 0);
    }

    #[test]
    fn second_call_with_same_prefix_hits_trie() {
        let router = KvAwareRouter::new(FixedRouter(BackendId(3)), KvAwareConfig::default());
        let pool = BackendPool::from_ids(vec![BackendId(3)]);
        let signals = BackendSignals::new();
        let body = long_body(b'a');
        let _ = router.route(&make_req(body.clone()), &pool, &signals);
        let _ = router.route(&make_req(body), &pool, &signals);
        let stats = router.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn breaker_open_falls_back_to_inner() {
        use riftgate_core::router::BackendSignal;
        let router = KvAwareRouter::new(FixedRouter(BackendId(3)), KvAwareConfig::default());
        let pool = BackendPool::from_ids(vec![BackendId(3)]);
        let body = long_body(b'b');
        // First call seeds.
        let signals = BackendSignals::new();
        let _ = router.route(&make_req(body.clone()), &pool, &signals);
        // Second call with breaker Open for backend 3 must skip the
        // trie hit and consult inner.
        let bsig_open = BackendSignal {
            circuit_state: CircuitState::Open,
            ..BackendSignal::default()
        };
        let signals_open = BackendSignals::from_vec(vec![
            BackendSignal::default(),
            BackendSignal::default(),
            BackendSignal::default(),
            bsig_open,
        ]);
        let _ = router.route(&make_req(body), &pool, &signals_open);
        let stats = router.stats();
        assert_eq!(stats.breaker_rejections, 1);
    }

    #[test]
    fn capacity_overflow_clears_trie() {
        let cfg = KvAwareConfig {
            max_trie_entries: 4,
            ..Default::default()
        };
        let router = KvAwareRouter::new(FixedRouter(BackendId(0)), cfg);
        let pool = BackendPool::from_ids(vec![BackendId(0)]);
        let signals = BackendSignals::new();
        for i in 0u8..8 {
            let _ = router.route(&make_req(long_body(b'a' + i)), &pool, &signals);
        }
        assert!(router.stats().entries <= 4);
    }
}
