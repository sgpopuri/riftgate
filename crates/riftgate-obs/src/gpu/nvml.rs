//! NVML-backed GPU-pressure source.
//!
//! This is the feature-gated escape hatch from ADR 0026 for deployments where
//! Riftgate runs on the GPU host. Default builds compile a stub that reports the
//! source as unavailable.

use riftgate_core::BackendId;

/// In-process NVML GPU-pressure source for one backend.
#[derive(Debug, Clone)]
pub struct NvmlSource {
    backend: BackendId,
    device_uuid: String,
}

impl NvmlSource {
    /// Construct an NVML source for a backend and NVIDIA device UUID.
    #[must_use]
    pub fn new(backend: BackendId, device_uuid: impl Into<String>) -> Self {
        Self {
            backend,
            device_uuid: device_uuid.into(),
        }
    }

    /// Backend this source reports pressure for.
    #[must_use]
    pub fn backend(&self) -> BackendId {
        self.backend
    }

    /// NVIDIA device UUID this source polls.
    #[must_use]
    pub fn device_uuid(&self) -> &str {
        &self.device_uuid
    }
}

#[cfg(all(target_os = "linux", feature = "gpu-nvml"))]
mod imp {
    use super::NvmlSource;
    use nvml_wrapper::Nvml;
    use nvml_wrapper::bitmasks::device::ThrottleReasons;
    use nvml_wrapper::enum_wrappers::device::{EccCounter, MemoryError};
    use riftgate_core::{GpuPressure, GpuPressureError, GpuPressureSource, GpuThrottleState};
    use std::time::Instant;

    impl GpuPressureSource for NvmlSource {
        fn poll_once(&self) -> Result<Vec<GpuPressure>, GpuPressureError> {
            let nvml = Nvml::init().map_err(|err| GpuPressureError::Ffi(err.to_string()))?;
            let device = nvml
                .device_by_uuid(self.device_uuid())
                .map_err(|err| GpuPressureError::Ffi(err.to_string()))?;
            let utilization = device
                .utilization_rates()
                .map_err(|err| GpuPressureError::Ffi(err.to_string()))?;
            let memory = device
                .memory_info()
                .map_err(|err| GpuPressureError::Ffi(err.to_string()))?;
            let throttle_reasons = device
                .current_throttle_reasons()
                .map_err(|err| GpuPressureError::Ffi(err.to_string()))?;
            let ecc_errors_total = total_ecc_errors(&device)?;

            let memory_used_pct = if memory.total == 0 {
                0.0
            } else {
                ((memory.used as f64 / memory.total as f64) * 100.0).clamp(0.0, 100.0) as f32
            };

            Ok(vec![GpuPressure {
                backend: self.backend(),
                utilization_pct: (utilization.gpu as f32).clamp(0.0, 100.0),
                memory_used_pct,
                throttle_state: throttle_state_from_reasons(throttle_reasons),
                ecc_errors_total,
                observed_at: Instant::now(),
            }])
        }

        fn name(&self) -> &'static str {
            "nvml"
        }
    }

    fn total_ecc_errors(
        device: &nvml_wrapper::device::Device<'_>,
    ) -> Result<u64, GpuPressureError> {
        let corrected = device
            .total_ecc_errors(MemoryError::Corrected, EccCounter::Volatile)
            .unwrap_or(0);
        let uncorrected = device
            .total_ecc_errors(MemoryError::Uncorrected, EccCounter::Volatile)
            .unwrap_or(0);
        Ok(corrected.saturating_add(uncorrected))
    }

    fn throttle_state_from_reasons(reasons: ThrottleReasons) -> GpuThrottleState {
        if reasons.contains(ThrottleReasons::HW_SLOWDOWN) {
            return GpuThrottleState::HwSlowdown;
        }
        if reasons
            .intersects(ThrottleReasons::SW_THERMAL_SLOWDOWN | ThrottleReasons::HW_THERMAL_SLOWDOWN)
        {
            return GpuThrottleState::Thermal;
        }
        if reasons
            .intersects(ThrottleReasons::SW_POWER_CAP | ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN)
        {
            return GpuThrottleState::Power;
        }
        if reasons.is_empty() || reasons == ThrottleReasons::NONE {
            GpuThrottleState::None
        } else {
            GpuThrottleState::SwLimit
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn throttle_reasons_map_to_stable_state() {
            assert_eq!(
                throttle_state_from_reasons(ThrottleReasons::HW_SLOWDOWN),
                GpuThrottleState::HwSlowdown
            );
            assert_eq!(
                throttle_state_from_reasons(ThrottleReasons::SW_THERMAL_SLOWDOWN),
                GpuThrottleState::Thermal
            );
            assert_eq!(
                throttle_state_from_reasons(ThrottleReasons::SW_POWER_CAP),
                GpuThrottleState::Power
            );
            assert_eq!(
                throttle_state_from_reasons(ThrottleReasons::NONE),
                GpuThrottleState::None
            );
        }
    }
}

#[cfg(not(all(target_os = "linux", feature = "gpu-nvml")))]
impl riftgate_core::GpuPressureSource for NvmlSource {
    fn poll_once(
        &self,
    ) -> Result<Vec<riftgate_core::GpuPressure>, riftgate_core::GpuPressureError> {
        Err(riftgate_core::GpuPressureError::Unavailable(
            "nvml source requires linux and the gpu-nvml feature",
        ))
    }

    fn name(&self) -> &'static str {
        "nvml"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::GpuPressureSource;

    #[test]
    fn source_records_backend_and_uuid() {
        let source = NvmlSource::new(BackendId(11), "GPU-test");
        assert_eq!(source.backend(), BackendId(11));
        assert_eq!(source.device_uuid(), "GPU-test");
        assert_eq!(source.name(), "nvml");
    }
}
