//! Backpressure trait + `HighWaterPolicy` impl.
//!
//! Per [Options 012](../../../docs/05-options/012-backpressure.md) and
//! [ADR 0017](../../../docs/06-adrs/0017-drop-newest-503-backpressure.md):
//! the gateway sheds load by **dropping the newest** request with a
//! `503 Service Unavailable` and a `Retry-After` header once the shared
//! queue crosses a configurable high-water mark. Below a (lower)
//! low-water mark the gateway resumes accepting new requests. The
//! hysteresis avoids a thrashing pattern where one rejected request
//! immediately frees a slot that the next request claims.
//!
//! This module exposes:
//!
//! - [`DenialReason`] — the shared vocabulary used by the rate limiter,
//!   the breaker, and this policy so OTel counters and `Retry-After`
//!   propagation have one structured cause label across the three
//!   protection primitives.
//! - [`AdmissionDecision`] — the per-call output.
//! - [`BackpressurePolicy`] — the trait.
//! - [`HighWaterPolicy`] — the v0.2 high/low-water implementation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Why a request was denied. Shared across the three v0.2 protection
/// primitives so the OTel counter `riftgate.requests.rejected{reason=…}`
/// is unified.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum DenialReason {
    /// Rate limiter rejected the request.
    RateLimit,
    /// Queue depth exceeded the configured high-water mark.
    QueueFull,
    /// Every eligible backend's circuit breaker was `Open`.
    CircuitOpen,
}

impl DenialReason {
    /// Static label suitable for an OTel attribute or a Prometheus label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::QueueFull => "queue_full",
            Self::CircuitOpen => "circuit_open",
        }
    }
}

/// Output of a [`BackpressurePolicy::on_enqueue`] check.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AdmissionDecision {
    /// Admit the request onto the shard queue.
    Admit,
    /// Reject with `503 Service Unavailable`. The caller propagates
    /// `retry_after` into the response's `Retry-After` header.
    Reject {
        /// Cause label.
        reason: DenialReason,
        /// `Retry-After` advice.
        retry_after: Duration,
    },
}

/// Trait shape: given the current queue depth, return an
/// [`AdmissionDecision`].
///
/// Implementations are pure functions of `(depth, internal state)`; the
/// caller is the only mutator of the queue itself. Trait-object-safe.
pub trait BackpressurePolicy: Send + Sync {
    /// Evaluate the policy for one inbound request.
    fn on_enqueue(&self, depth: usize) -> AdmissionDecision;
}

/// High/low-water-mark backpressure policy with hysteresis.
///
/// State machine:
///
/// - **Admitting** (default). `on_enqueue` returns [`AdmissionDecision::Admit`]
///   until `depth >= high_water`. On crossing, the policy flips to
///   `Shedding`.
/// - **Shedding**. `on_enqueue` returns [`AdmissionDecision::Reject`] with
///   `retry_after` until `depth <= low_water`. On crossing, the policy
///   flips back to `Admitting`.
///
/// `low_water < high_water` is enforced at construction.
pub struct HighWaterPolicy {
    high_water: usize,
    low_water: usize,
    retry_after: Duration,
    shedding: AtomicBool,
}

impl HighWaterPolicy {
    /// Construct a new policy.
    ///
    /// # Panics
    /// Panics if `low_water >= high_water` (hysteresis collapses to zero
    /// otherwise) or if either is zero.
    #[must_use]
    pub fn new(high_water: usize, low_water: usize, retry_after: Duration) -> Self {
        assert!(high_water > 0, "high_water must be > 0");
        assert!(low_water > 0, "low_water must be > 0");
        assert!(
            low_water < high_water,
            "low_water must be strictly less than high_water"
        );
        Self {
            high_water,
            low_water,
            retry_after,
            shedding: AtomicBool::new(false),
        }
    }

    /// `true` while the policy is rejecting requests. Test helper.
    #[must_use]
    pub fn is_shedding(&self) -> bool {
        self.shedding.load(Ordering::Acquire)
    }
}

impl BackpressurePolicy for HighWaterPolicy {
    fn on_enqueue(&self, depth: usize) -> AdmissionDecision {
        let shedding = self.shedding.load(Ordering::Acquire);
        let new_shedding = if shedding {
            depth > self.low_water
        } else {
            depth >= self.high_water
        };
        if new_shedding != shedding {
            self.shedding.store(new_shedding, Ordering::Release);
        }
        if new_shedding {
            AdmissionDecision::Reject {
                reason: DenialReason::QueueFull,
                retry_after: self.retry_after,
            }
        } else {
            AdmissionDecision::Admit
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_below_high_water() {
        let p = HighWaterPolicy::new(100, 80, Duration::from_millis(50));
        assert_eq!(p.on_enqueue(0), AdmissionDecision::Admit);
        assert_eq!(p.on_enqueue(50), AdmissionDecision::Admit);
        assert_eq!(p.on_enqueue(99), AdmissionDecision::Admit);
        assert!(!p.is_shedding());
    }

    #[test]
    fn rejects_at_or_above_high_water() {
        let p = HighWaterPolicy::new(10, 5, Duration::from_millis(25));
        match p.on_enqueue(10) {
            AdmissionDecision::Reject {
                reason,
                retry_after,
            } => {
                assert_eq!(reason, DenialReason::QueueFull);
                assert_eq!(retry_after, Duration::from_millis(25));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
        assert!(p.is_shedding());
    }

    #[test]
    fn hysteresis_resumes_at_low_water_only() {
        let p = HighWaterPolicy::new(10, 5, Duration::from_millis(10));
        // Climb to high-water and flip to shedding.
        assert_eq!(p.on_enqueue(10), make_reject());
        assert!(p.is_shedding());
        // Drop to between low and high; still shedding.
        assert_eq!(p.on_enqueue(9), make_reject());
        assert_eq!(p.on_enqueue(6), make_reject());
        // Drop to low-water; resume.
        assert_eq!(p.on_enqueue(5), AdmissionDecision::Admit);
        assert!(!p.is_shedding());
    }

    fn make_reject() -> AdmissionDecision {
        AdmissionDecision::Reject {
            reason: DenialReason::QueueFull,
            retry_after: Duration::from_millis(10),
        }
    }

    #[test]
    #[should_panic(expected = "low_water must be strictly less than high_water")]
    fn rejects_inverted_water_marks() {
        let _ = HighWaterPolicy::new(10, 10, Duration::from_millis(1));
    }

    #[test]
    fn denial_reason_labels_are_stable() {
        assert_eq!(DenialReason::RateLimit.as_str(), "rate_limit");
        assert_eq!(DenialReason::QueueFull.as_str(), "queue_full");
        assert_eq!(DenialReason::CircuitOpen.as_str(), "circuit_open");
    }
}
