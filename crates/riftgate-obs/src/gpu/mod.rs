//! GPU-pressure sources for the observability plane.
//!
//! Per [ADR 0026](../../../docs/06-adrs/0026-gpu-pressure-via-dcgm-exporter.md),
//! `riftgate-obs` owns the concrete `GpuPressureSource` implementations while
//! `riftgate-core` owns the trait surface.

mod dcgm;
mod nvml;

pub use dcgm::DcgmScrapeSource;
pub use nvml::NvmlSource;
