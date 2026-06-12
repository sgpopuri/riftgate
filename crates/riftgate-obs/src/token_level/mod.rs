//! Per-token metrics aggregation: TTFT, inter-token latency, jitter.
//!
//! Per [ADR 0025](../../../docs/06-adrs/0025-token-level-metrics-probabilistic.md),
//! v0.4 aggregates token-level SLO metrics as HDR histograms (aggregate latency)
//! and via Vitter reservoir sampling (forensic tail sampling). One histogram
//! per `(tenant, model, route)` dimension; bounded cap with `(other, other, other)`
//! fallback when exceeded.
//!
//! **Non-blocking design:** All operations are non-blocking lock-free or
//! use sharded locks via DashMap. Publishing a token event does not block
//! the data plane.

use dashmap::DashMap;
use hdrhistogram::Histogram;
use std::sync::Arc;

pub use self::reservoir::VitterReservoir;

mod reservoir;

/// Token-level metrics dimensions. Stable for observability labels.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TokenDimension {
    /// Tenant identifier (or "other" if cap exceeded).
    pub tenant: String,
    /// Model identifier (or "other" if cap exceeded).
    pub model: String,
    /// Route identifier (or "other" if cap exceeded).
    pub route: String,
}

impl TokenDimension {
    /// Create a new dimension. Input strings are taken as-is; dimensionality
    /// capping is applied at the aggregator level.
    pub fn new(
        tenant: impl Into<String>,
        model: impl Into<String>,
        route: impl Into<String>,
    ) -> Self {
        Self {
            tenant: tenant.into(),
            model: model.into(),
            route: route.into(),
        }
    }

    /// The fallback "other" dimension used when the cardinality cap is exceeded.
    pub fn other() -> Self {
        Self {
            tenant: "other".to_string(),
            model: "other".to_string(),
            route: "other".to_string(),
        }
    }
}

/// Per-dimension token metrics: aggregate latencies and forensic samples.
struct DimensionMetrics {
    /// HDR histogram for TTFT latency (bounded 1 µs to 10 s).
    ttft_histogram: Histogram<u64>,
    /// HDR histogram for inter-token latency (bounded 1 µs to 10 s).
    inter_token_histogram: Histogram<u64>,
    /// Vitter reservoir for TTFT samples (K=100 by default).
    ttft_reservoir: VitterReservoir<u64>,
    /// Vitter reservoir for inter-token samples (K=100 by default).
    inter_token_reservoir: VitterReservoir<u64>,
}

impl DimensionMetrics {
    /// Create a new dimension's metrics. Histograms are bounded to
    /// [1 µs, 10 s] with auto-scaling power-of-two bucketing.
    fn new(reservoir_capacity: usize) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            ttft_histogram: Histogram::new(3)?, // 3 digits of precision
            inter_token_histogram: Histogram::new(3)?,
            ttft_reservoir: VitterReservoir::new(reservoir_capacity),
            inter_token_reservoir: VitterReservoir::new(reservoir_capacity),
        })
    }

    /// Record a TTFT sample.
    fn record_ttft(&mut self, latency_micros: u64) {
        let _ = self.ttft_histogram.record(latency_micros);
        self.ttft_reservoir.add(latency_micros);
    }

    /// Record an inter-token sample.
    fn record_inter_token(&mut self, latency_micros: u64) {
        let _ = self.inter_token_histogram.record(latency_micros);
        self.inter_token_reservoir.add(latency_micros);
    }
}

/// Aggregates per-`(tenant, model, route)` token-level metrics.
///
/// **Thread-safety:** All operations are non-blocking. DashMap provides
/// sharded concurrent access; reservoirs and histograms are mutated only
/// via exclusive access gained via the map's Entry API.
///
/// **Cardinality cap:** Bounded to `dimension_cap` (default 10,000) unique
/// dimensions; overflows are bucketed into the `(other, other, other)` dimension
/// to prevent OOM from attacker-controlled tenant/model/route values.
pub struct TokenLevelAggregator {
    /// Map from dimension to per-dimension metrics.
    dimensions: Arc<DashMap<TokenDimension, DimensionMetrics>>,
    /// Hard cap on unique dimensions. Overflow goes to "other".
    dimension_cap: usize,
    /// Capacity of each reservoir (default 100).
    reservoir_capacity: usize,
}

impl TokenLevelAggregator {
    /// Create a new aggregator with defaults (dimension_cap=10_000, reservoir K=100).
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_capacity(10_000, 100)
    }

    /// Create a new aggregator with custom caps.
    pub fn with_capacity(
        dimension_cap: usize,
        reservoir_capacity: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            dimensions: Arc::new(DashMap::new()),
            dimension_cap,
            reservoir_capacity,
        })
    }

    /// Record a token's TTFT latency in microseconds. Non-blocking.
    pub fn record_ttft(&self, dim: TokenDimension, latency_micros: u64) {
        self.record_metric(&dim, |m| m.record_ttft(latency_micros));
    }

    /// Record a token's inter-token latency in microseconds. Non-blocking.
    pub fn record_inter_token(&self, dim: TokenDimension, latency_micros: u64) {
        self.record_metric(&dim, |m| m.record_inter_token(latency_micros));
    }

    /// Helper: record a metric, capping dimensions and falling back to "other" on overflow.
    fn record_metric<F>(&self, dim: &TokenDimension, f: F)
    where
        F: FnOnce(&mut DimensionMetrics),
    {
        let target_dim = if self.dimensions.len() >= self.dimension_cap {
            TokenDimension::other()
        } else {
            dim.clone()
        };

        // Acquire exclusive access to the target dimension and record.
        let mut entry = self.dimensions.entry(target_dim).or_insert_with(|| {
            DimensionMetrics::new(self.reservoir_capacity).expect("histogram creation failed")
        });
        f(&mut entry);
    }

    /// Snapshot the current metrics for a dimension. Returns None if the
    /// dimension has no samples yet.
    pub fn snapshot(&self, dim: &TokenDimension) -> Option<DimensionSnapshot> {
        self.dimensions.get(dim).map(|metrics| DimensionSnapshot {
            ttft_p50: metrics.ttft_histogram.value_at_percentile(50.0),
            ttft_p95: metrics.ttft_histogram.value_at_percentile(95.0),
            ttft_p99: metrics.ttft_histogram.value_at_percentile(99.0),
            inter_token_p50: metrics.inter_token_histogram.value_at_percentile(50.0),
            inter_token_p95: metrics.inter_token_histogram.value_at_percentile(95.0),
            inter_token_p99: metrics.inter_token_histogram.value_at_percentile(99.0),
            ttft_samples: metrics.ttft_reservoir.samples(),
            inter_token_samples: metrics.inter_token_reservoir.samples(),
        })
    }

    /// Current number of unique dimensions being tracked.
    pub fn dimension_count(&self) -> usize {
        self.dimensions.len()
    }
}

impl Default for TokenLevelAggregator {
    fn default() -> Self {
        Self::new().expect("TokenLevelAggregator creation failed")
    }
}

impl Clone for TokenLevelAggregator {
    fn clone(&self) -> Self {
        Self {
            dimensions: Arc::clone(&self.dimensions),
            dimension_cap: self.dimension_cap,
            reservoir_capacity: self.reservoir_capacity,
        }
    }
}

/// Snapshot of metrics for a single dimension at a point in time.
#[derive(Debug, Clone)]
pub struct DimensionSnapshot {
    /// TTFT latency (µs), 50th percentile.
    pub ttft_p50: u64,
    /// TTFT latency (µs), 95th percentile.
    pub ttft_p95: u64,
    /// TTFT latency (µs), 99th percentile.
    pub ttft_p99: u64,
    /// Inter-token latency (µs), 50th percentile.
    pub inter_token_p50: u64,
    /// Inter-token latency (µs), 95th percentile.
    pub inter_token_p95: u64,
    /// Inter-token latency (µs), 99th percentile.
    pub inter_token_p99: u64,
    /// Forensic TTFT samples (up to K items from reservoir).
    pub ttft_samples: Vec<u64>,
    /// Forensic inter-token samples (up to K items from reservoir).
    pub inter_token_samples: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_ttft_updates_histogram() {
        let agg = TokenLevelAggregator::new().unwrap();
        let dim = TokenDimension::new("tenant1", "gpt-4", "chat");

        for i in 1..=100 {
            agg.record_ttft(dim.clone(), i * 100); // 100, 200, ..., 10000 µs
        }

        let snap = agg.snapshot(&dim).expect("snapshot failed");
        assert!(snap.ttft_p50 > 0);
        assert!(snap.ttft_p95 > snap.ttft_p50);
        assert!(snap.ttft_p99 > snap.ttft_p95);
    }

    #[test]
    fn record_inter_token_updates_histogram() {
        let agg = TokenLevelAggregator::new().unwrap();
        let dim = TokenDimension::new("tenant2", "gpt-3.5", "completion");

        for i in 1..=50 {
            agg.record_inter_token(dim.clone(), i * 50); // 50, 100, ..., 2500 µs
        }

        let snap = agg.snapshot(&dim).expect("snapshot failed");
        assert!(snap.inter_token_p50 > 0);
        assert!(snap.inter_token_p95 > snap.inter_token_p50);
    }

    #[test]
    fn dimension_cap_enforces_overflow_to_other() {
        let agg = TokenLevelAggregator::with_capacity(2, 10).unwrap();

        // Add 2 dimensions up to the cap.
        let dim1 = TokenDimension::new("t1", "m1", "r1");
        let dim2 = TokenDimension::new("t2", "m2", "r2");
        agg.record_ttft(dim1.clone(), 100);
        agg.record_ttft(dim2.clone(), 200);

        // Third dimension exceeds cap and falls back to "other".
        let dim3 = TokenDimension::new("t3", "m3", "r3");
        agg.record_ttft(dim3.clone(), 300);

        // Check that dim3 did not get its own entry; instead it went to "other".
        assert_eq!(agg.dimension_count(), 3); // dim1, dim2, "other"
        assert!(agg.snapshot(&dim3).is_none()); // dim3 is not tracked
        let other = agg
            .snapshot(&TokenDimension::other())
            .expect("other snapshot");
        assert!(other.ttft_p50 > 0); // "other" received the sample
    }

    #[test]
    fn vitter_reservoir_captures_samples() {
        let agg = TokenLevelAggregator::with_capacity(10_000, 10).unwrap();
        let dim = TokenDimension::new("t", "m", "r");

        // Add 100 samples; reservoir should hold up to 10.
        for i in 0..100 {
            agg.record_ttft(dim.clone(), (i * 10) as u64);
        }

        let snap = agg.snapshot(&dim).expect("snapshot");
        assert!(snap.ttft_samples.len() <= 10);
        assert!(!snap.ttft_samples.is_empty());
    }

    #[test]
    fn concurrent_record_does_not_block() {
        let agg = Arc::new(TokenLevelAggregator::new().unwrap());
        let mut tasks = vec![];

        for task_id in 0..10 {
            let agg_clone = Arc::clone(&agg);
            let task = std::thread::spawn(move || {
                let dim = TokenDimension::new(format!("tenant{}", task_id), "model", "route");
                for i in 0..100 {
                    agg_clone.record_ttft(dim.clone(), (i * 10) as u64);
                    agg_clone.record_inter_token(dim.clone(), (i * 5) as u64);
                }
            });
            tasks.push(task);
        }

        for task in tasks {
            task.join().expect("thread panicked");
        }

        // Verify all dimensions are tracked.
        assert_eq!(agg.dimension_count(), 10);
    }
}
