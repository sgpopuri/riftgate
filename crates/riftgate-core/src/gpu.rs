//! GPU pressure trait surface — v0.4.
//!
//! Per [Options `028`](../../../docs/05-options/028-gpu-pressure-correlation.md)
//! and [ADR `0026`](../../../docs/06-adrs/0026-gpu-pressure-via-dcgm-exporter.md):
//!
//! - Riftgate models per-backend GPU pressure as a *signal* that the router
//!   can fold into its decision. The default [`Router::route`] consumer is
//!   [`crate::router::BackendSignals::gpu_pressure`], which the v0.4 routing
//!   crate populates from the trait below.
//! - Two impls ship in `crates/riftgate-obs`:
//!   - `DcgmScrapeSource` — scrapes the NVIDIA `dcgm-exporter` Prometheus
//!     endpoint at operator-configured cadence (default 5 s). Default for
//!     LB-topology deployments.
//!   - `NvmlSource` — in-process NVML FFI via `nvml-wrapper` behind the
//!     `gpu-nvml` feature. For GPU-co-located deployments.
//! - One trait-stable null impl ([`NoopGpuSource`]) lives here so the trait
//!   surface has the required FR-X02 second impl even when neither concrete
//!   sink crate is built.
//!
//! The router never blocks waiting on GPU telemetry. The poll is a
//! background task; the routing hot path reads a `BackendSignals` snapshot
//! that the poll task refreshes via `arc-swap`. Staleness is the cost of
//! decoupling, and is documented in the LLD.

use crate::router::BackendId;
use core::fmt;
use std::time::Instant;

/// Single observation of GPU pressure for a specific backend at a specific
/// instant.
///
/// All numeric fields are bounded:
///
/// - `utilization_pct` and `memory_used_pct` are normalized to `0.0..=100.0`.
/// - `throttle_state` is a typed enum (no free-form strings).
/// - `ecc_errors_total` is a monotonic counter; consumers diff against the
///   previous sample to detect new errors.
#[derive(Debug, Clone)]
pub struct GpuPressure {
    /// Which backend this observation belongs to.
    pub backend: BackendId,
    /// Compute utilization, percent. `0.0..=100.0`.
    pub utilization_pct: f32,
    /// Framebuffer memory in use, percent. `0.0..=100.0`.
    pub memory_used_pct: f32,
    /// Current throttle state reported by the GPU.
    pub throttle_state: GpuThrottleState,
    /// Total ECC errors observed for this GPU since power-on. Monotonic.
    pub ecc_errors_total: u64,
    /// When this observation was taken.
    pub observed_at: Instant,
}

impl GpuPressure {
    /// Reduce the observation to the scalar `0.0..=1.0` axis the router's
    /// [`BackendSignal::gpu_pressure`](crate::router::BackendSignal::gpu_pressure)
    /// expects. The reduction is `max(utilization, memory_used)` clamped
    /// into `[0, 1]`, which biases the router away from a GPU that is hot
    /// on either compute or memory.
    #[must_use]
    pub fn scalar_pressure(&self) -> f32 {
        let m = self.utilization_pct.max(self.memory_used_pct);
        (m / 100.0).clamp(0.0, 1.0)
    }
}

/// Throttling reason reported by the GPU. Bounded enum so observability
/// labels stay cardinality-safe.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum GpuThrottleState {
    /// Not throttled.
    #[default]
    None,
    /// Throttled by software limit (power cap, clock cap, MIG quota).
    SwLimit,
    /// Throttled because the GPU is thermally constrained.
    Thermal,
    /// Throttled because the host's power supply is saturated.
    Power,
    /// Hardware slowdown asserted (worst-case, indicates a fault).
    HwSlowdown,
}

impl GpuThrottleState {
    /// Wire-format string. Stable for observability labels.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SwLimit => "sw_limit",
            Self::Thermal => "thermal",
            Self::Power => "power",
            Self::HwSlowdown => "hw_slowdown",
        }
    }
}

impl fmt::Display for GpuThrottleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// GPU-pressure source trait.
///
/// One impl per topology (DCGM exporter for LB-topology, NVML for
/// GPU-co-located). Riftgate composes the impl behind a poll task in
/// `riftgate-obs`; the router reads the resulting `BackendSignals` snapshot
/// from `arc-swap`.
///
/// **`Send + Sync`** because the same instance is shared across the poll
/// task and any introspection RPC (`/gpu/snapshot`, when it lands).
///
/// **Trait object safety**: yes (no generics, no associated types).
pub trait GpuPressureSource: Send + Sync {
    /// Poll the source once. Implementations may block on IO; callers run
    /// `poll_once` on a dedicated tokio task and never on the routing hot
    /// path.
    ///
    /// Returns one observation per backend the source knows about. The
    /// returned `Vec` may be empty (no GPU backends configured); the poll
    /// task treats that as "no signal change".
    ///
    /// # Errors
    /// Returns `Err` on scrape / FFI failure. Callers log and back off
    /// per [`docs/04-design/lld-observability.md`](../../../docs/04-design/lld-observability.md).
    fn poll_once(&self) -> Result<Vec<GpuPressure>, GpuPressureError>;

    /// Human-readable name for the source ("dcgm-exporter", "nvml",
    /// "noop"). Used by the poll-task log line so operators can correlate
    /// which source is active.
    fn name(&self) -> &'static str;
}

/// Errors reported by a [`GpuPressureSource`] poll. Typed (not strings) so
/// operators can branch on the reason in the poll-task back-off logic.
#[derive(Debug, thiserror::Error)]
pub enum GpuPressureError {
    /// Scrape endpoint refused the connection or returned a non-2xx status.
    #[error("gpu pressure scrape failed: {0}")]
    ScrapeFailed(String),
    /// Underlying FFI / driver returned an error (NVML / DCGM API call).
    #[error("gpu pressure ffi error: {0}")]
    Ffi(String),
    /// Source returned data that does not match the expected schema
    /// (e.g. a Prometheus metric name we don't recognize, or a value out
    /// of the documented range).
    #[error("gpu pressure parse error: {0}")]
    Parse(String),
    /// Source is correctly compiled in but disabled at runtime (feature
    /// off, capability missing, no GPUs detected).
    #[error("gpu pressure source unavailable: {0}")]
    Unavailable(&'static str),
}

/// Null impl. Always returns an empty observation vector and the name
/// `"noop"`.
///
/// Default `GpuPressureSource` when the operator did not configure a
/// `gpu_pressure_source` block. Useful for the FR-X02 second impl, for the
/// unit-test suite, and for running Riftgate on non-NVIDIA Linux or on
/// macOS where DCGM and NVML do not exist.
#[derive(Debug, Default, Copy, Clone)]
pub struct NoopGpuSource;

impl NoopGpuSource {
    /// Construct a `NoopGpuSource`. Zero-cost.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl GpuPressureSource for NoopGpuSource {
    fn poll_once(&self) -> Result<Vec<GpuPressure>, GpuPressureError> {
        Ok(Vec::new())
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

/// Static-table impl used by tests and by the routing crate's unit suite
/// to drive the router under deterministic GPU pressure values.
///
/// FR-X02 second impl alongside [`NoopGpuSource`]. Holds a `Vec<GpuPressure>`
/// snapshot built at construction and returns a clone of it on every
/// `poll_once`. Not for production use.
#[derive(Debug, Clone)]
pub struct StaticGpuSource {
    snapshot: Vec<GpuPressure>,
}

impl StaticGpuSource {
    /// Construct from a precomputed snapshot. Tests use this to model a
    /// hot GPU (high utilization), a cold GPU (zero), or a throttled GPU.
    #[must_use]
    pub fn new(snapshot: Vec<GpuPressure>) -> Self {
        Self { snapshot }
    }
}

impl GpuPressureSource for StaticGpuSource {
    fn poll_once(&self) -> Result<Vec<GpuPressure>, GpuPressureError> {
        Ok(self.snapshot.clone())
    }

    fn name(&self) -> &'static str {
        "static"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(util: f32, mem: f32) -> GpuPressure {
        GpuPressure {
            backend: BackendId(0),
            utilization_pct: util,
            memory_used_pct: mem,
            throttle_state: GpuThrottleState::None,
            ecc_errors_total: 0,
            observed_at: Instant::now(),
        }
    }

    #[test]
    fn scalar_pressure_takes_max_of_axes() {
        assert!((sample(70.0, 30.0).scalar_pressure() - 0.70).abs() < 1e-6);
        assert!((sample(30.0, 90.0).scalar_pressure() - 0.90).abs() < 1e-6);
    }

    #[test]
    fn scalar_pressure_clamps_to_unit_range() {
        assert_eq!(sample(120.0, 100.0).scalar_pressure(), 1.0);
        assert_eq!(sample(-10.0, -5.0).scalar_pressure(), 0.0);
    }

    #[test]
    fn noop_source_is_empty() {
        let s = NoopGpuSource::new();
        assert_eq!(s.poll_once().unwrap().len(), 0);
        assert_eq!(s.name(), "noop");
    }

    #[test]
    fn static_source_round_trips_snapshot() {
        let snap = vec![sample(50.0, 60.0), sample(80.0, 20.0)];
        let s = StaticGpuSource::new(snap);
        let out = s.poll_once().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(s.name(), "static");
    }

    #[test]
    fn throttle_state_strings_are_stable() {
        assert_eq!(GpuThrottleState::None.as_str(), "none");
        assert_eq!(GpuThrottleState::Thermal.as_str(), "thermal");
        assert_eq!(GpuThrottleState::HwSlowdown.as_str(), "hw_slowdown");
    }
}
