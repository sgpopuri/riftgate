//! Live backend-signal snapshot helpers.
//!
//! v0.4 runtime integration updates `BackendSignals` from background
//! `GpuPressureSource` polls while request routing reads a lock-free snapshot.

use arc_swap::ArcSwap;
use riftgate_core::gpu::GpuPressure;
use riftgate_core::router::{BackendId, BackendSignal, BackendSignals};
use std::sync::Arc;

/// Apply one poll cycle of GPU-pressure observations to the live signal snapshot.
///
/// Existing non-GPU fields (circuit state, latency hints) are preserved.
pub fn apply_gpu_pressure_updates(store: &ArcSwap<BackendSignals>, samples: &[GpuPressure]) {
    if samples.is_empty() {
        return;
    }

    let current = store.load_full();
    let max_backend = samples
        .iter()
        .map(|s| s.backend.0 as usize)
        .max()
        .unwrap_or(0);
    let mut next: Vec<BackendSignal> = (0..=max_backend.max(current.len().saturating_sub(1)))
        .map(|idx| current.get(BackendId(idx as u16)))
        .collect();

    for sample in samples {
        let idx = sample.backend.0 as usize;
        if idx >= next.len() {
            next.resize(idx + 1, BackendSignal::default());
        }
        next[idx].gpu_pressure = Some(sample.scalar_pressure());
    }

    store.store(Arc::new(BackendSignals::from_vec(next)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::gpu::GpuThrottleState;
    use riftgate_core::router::CircuitState;
    use std::time::Instant;

    fn sample(backend: BackendId, util: f32, mem: f32) -> GpuPressure {
        GpuPressure {
            backend,
            utilization_pct: util,
            memory_used_pct: mem,
            throttle_state: GpuThrottleState::None,
            ecc_errors_total: 0,
            observed_at: Instant::now(),
        }
    }

    #[test]
    fn apply_updates_sets_gpu_pressure_scalar() {
        let store = ArcSwap::from_pointee(BackendSignals::from_vec(vec![BackendSignal::default()]));
        apply_gpu_pressure_updates(&store, &[sample(BackendId(0), 70.0, 30.0)]);

        let snapshot = store.load();
        assert_eq!(snapshot.get(BackendId(0)).gpu_pressure, Some(0.70));
    }

    #[test]
    fn apply_updates_preserves_other_signal_fields() {
        let initial = BackendSignals::from_vec(vec![BackendSignal {
            circuit_state: CircuitState::Open,
            gpu_pressure: None,
            recent_p99_ms: 123.0,
        }]);
        let store = ArcSwap::from_pointee(initial);

        apply_gpu_pressure_updates(&store, &[sample(BackendId(0), 20.0, 40.0)]);

        let snapshot = store.load();
        let signal = snapshot.get(BackendId(0));
        assert_eq!(signal.circuit_state, CircuitState::Open);
        assert_eq!(signal.recent_p99_ms, 123.0);
        assert_eq!(signal.gpu_pressure, Some(0.40));
    }

    #[test]
    fn apply_updates_expands_snapshot_for_new_backend() {
        let store = ArcSwap::from_pointee(BackendSignals::new());
        apply_gpu_pressure_updates(&store, &[sample(BackendId(2), 90.0, 10.0)]);

        let snapshot = store.load();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot.get(BackendId(2)).gpu_pressure, Some(0.90));
    }
}
