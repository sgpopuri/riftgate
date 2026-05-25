# ADR 0026. GPU pressure correlation via DCGM exporter HTTP scrape (primary) and NVML in-process FFI (feature-gated escape hatch)

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [028-gpu-pressure-correlation](../05-options/028-gpu-pressure-correlation.md)
> **Deciders:** Sriram Popuri

## Context

[`docs/04-design/lld-routing.md`](../04-design/lld-routing.md) declares `BackendSignals::gpu_pressure: f32` as the read-only routing-input field that drives weighted-random rebalancing under GPU saturation, the `HedgedRouter`'s decision to escalate ([ADR `0023`](0023-hedged-requests-p99-triggered.md)), and joins to per-request OTel spans in `v0.4`. Five candidates were evaluated in [Options `028`](../05-options/028-gpu-pressure-correlation.md): NVML in-process FFI, DCGM exporter HTTP scrape, purpose-built sidecar, eBPF kprobes on NVIDIA driver symbols, and a hybrid. The bedrock constraints: single-binary distribution (`NFR-OPS02`), Riftgate-on-LB topologies (most production deployments are not GPU-co-located), multi-vendor reality (NVIDIA + AMD + Habana + Inferentia in 2026), and pluggability over performance (`AGENTS.md` §5).

## Decision

**`v0.4` adopts the hybrid `§3.5`: a new `GpuPressureSource` trait in `crates/riftgate-core`, with `DcgmScrapeSource` as the default impl (HTTP scrape of NVIDIA's `dcgm-exporter` Prometheus endpoint) and `NvmlSource` as a feature-gated alternative (`gpu-nvml`) for operators on GPU-co-located topologies. Pure sidecar, pure eBPF, and `nvmlDeviceHandle_t`-in-`BackendSignals` are explicitly rejected.**

- New trait `GpuPressureSource` in `crates/riftgate-core/src/gpu.rs` with `fn current(&self, backend: BackendId) -> Option<GpuPressure>`. The richer struct `GpuPressure { utilization_pct, memory_used_pct, throttle_state, ecc_errors_total, observed_at }` lives behind the trait; the routing-hot-path `BackendSignals::gpu_pressure: f32` field remains a derived single-axis summary (`utilization_pct.max(memory_used_pct)`).
- `DcgmScrapeSource` in `crates/riftgate-obs/src/gpu/dcgm.rs` is the default impl. TOML configuration per backend:
  ```toml
  [backends.gpu0.gpu_pressure_source.dcgm]
  endpoint = "http://gpu-node-1.internal:9400/metrics"
  scrape_interval_ms = 5000
  gpu_index = 0
  ```
  Scraping happens in a per-backend background task; results write into `ArcSwap<GpuPressure>` that the routing hot path reads with no lock. The hot path **never blocks on the scrape**.
- `NvmlSource` in `crates/riftgate-obs/src/gpu/nvml.rs` is feature-gated `gpu-nvml`, Linux-only, cargo dependencies follow the `crates/riftgate-io-uring/Cargo.toml` `[target.'cfg(target_os = "linux")'.dependencies]` pattern. Empty-lib elsewhere; macOS and non-NVIDIA Linux compile cleanly with no behavioural change.
- `NoopGpuSource` is the default for backends without an explicit `gpu_pressure_source` block. Returns `None`; routers see `BackendSignals::gpu_pressure = 0.0`.
- Observability join: both impls publish `Histogram` events for `riftgate_backend_gpu_utilization_pct` and `riftgate_backend_gpu_memory_used_pct` per backend on the existing observability bus from [ADR `0011`](0011-otel-default-sink-multisink-fanout.md). The eBPF integration from [ADR `0024`](0024-ebpf-via-aya.md) remains orthogonal — BPF programs can attribute syscall costs to specific GPUs, but they do not produce the primary `BackendSignals::gpu_pressure` value.
- AMD ROCm SMI exporter [Options `028` §6] uses Prometheus scrape format; an operator can point `DcgmScrapeSource` at an AMD exporter endpoint and field translation works for utilization and memory. Documented as a known-working multi-vendor path; not promoted to a separate impl in `v0.4`.
- MIG (Multi-Instance GPU) partitioning: `DcgmScrapeSource` accepts a `mig_uuid` configuration; reads the per-MIG-labelled metric from DCGM. NVML similarly.

## Consequences

- **Positive:**
  - No NVIDIA runtime library in Riftgate's default link line. `cargo build --workspace --all-features` on a developer laptop without `libnvidia-ml.so.1` compiles cleanly.
  - Works on Riftgate-on-LB topologies (most production deployments). DCGM exporter runs on the GPU host; Riftgate reads it over the network at whatever cadence operators configure.
  - Reuses operators' existing DCGM exporter deployments (NVIDIA GPU operator is the canonical Kubernetes pattern). No new prerequisite for operators who already monitor GPUs.
  - The `GpuPressureSource` trait surface absorbs vendor diversity. AMD via Prometheus scrape today; native ROCm SMI / Habana / Inferentia impls become clean future additions behind the same trait without touching `BackendSignals`.
  - Routing hot path is unaffected — every read is from `ArcSwap<GpuPressure>` (one atomic load), staleness is documented, no I/O on the hot path.
  - `nvmlDeviceHandle_t` and other vendor-specific types stay out of `riftgate-core`'s public surface, preserving the "pluggability over performance" invariant.
- **Negative / accepted tradeoffs:**
  - Update cadence at the DCGM-default 1-second scrape interval is bounded by network jitter; routers see GPU pressure with documented staleness. The pull-vs-push tradeoff is real; operators with sub-second routing requirements can tighten the interval (DCGM down to 100 ms) or escalate to the deferred sidecar option.
  - Two impls to maintain in `v0.4`: DCGM and NVML. The trait abstraction is the right cost to pay for topology flexibility.
  - DCGM exporter is a soft prerequisite for GPU-aware routing on NVIDIA hosts. Operators who do not deploy it get `NoopGpuSource` and the field stays at zero.
  - Per-process GPU memory accounting (uniquely available via NVML) is only accessible behind the `gpu-nvml` feature gate. Production deployments on the LB tier lose this fidelity. Acceptable given the topology constraint.
- **Future work this enables:**
  - Vendor-specific impls behind the same trait: `RocmSmiSource` for AMD, `HabanaExporterSource` for Habana, etc. Each is a self-contained impl with no `BackendSignals` schema change.
  - Sidecar option (Options `028` §3.3) becomes the long-term multi-vendor strategy for `v1.0+` when operator demand justifies a second Riftgate-maintained binary.
  - The routing-input integration is straightforward to extend: weighted-random rebalancing, hedged-router escalation triggers, KV-aware routing fallback when an Open backend signals high pressure.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship a GPU-pressure sidecar binary in `v0.4`.
  - Riftgate will not link against `libnvidia-ml.so.1` in the default build.
  - Riftgate will not derive GPU pressure from eBPF kprobes on NVIDIA driver symbols (Options `028` §3.4 is rejected).
  - `BackendSignals` will not gain vendor-specific fields. New signals go behind the `GpuPressureSource` trait or stay in observability-only metrics.

## Compliance

- `crates/riftgate-core/src/gpu.rs` declares the trait and the `GpuPressure` struct.
- `crates/riftgate-obs/src/gpu/dcgm.rs` implements `DcgmScrapeSource`; `crates/riftgate-obs/src/gpu/nvml.rs` implements `NvmlSource` behind `cfg(all(target_os = "linux", feature = "gpu-nvml"))`.
- `crates/riftgate-obs/tests/dcgm_scrape.rs` asserts correct parsing of DCGM exporter Prometheus output against a fixture file checked into the repo.
- `crates/riftgate-obs/tests/dcgm_scrape_failure.rs` asserts graceful behaviour under scrape failure (timeout, partial read, malformed metrics): the last-known `GpuPressure` remains valid for the documented staleness window, then transitions to `None`.
- `crates/riftgate-router/tests/gpu_pressure_routing.rs` asserts that `WeightedRandomRouter` correctly down-weights backends with high `BackendSignals::gpu_pressure`.
- The trait surface in `riftgate-core` is part of the v0.1 frozen surface; changes require a new ADR. Adding new impls (vendor-specific) does **not** require a new ADR. Changing the `BackendSignals::gpu_pressure` field type **does**.

## Notes

- The choice to keep `BackendSignals::gpu_pressure` as a single-axis `f32` rather than promoting `GpuPressure` directly is deliberate. Routers want a scalar; promoting the struct would require every router to handle multi-vendor field heterogeneity. The trait absorbs that on the producer side instead.
- The DCGM-exporter scrape interval is the load-bearing tunable for operators. 5 seconds (the default) is appropriate for SLO-grade rebalancing; 1 second is appropriate for fast-routing decisions; 100 ms is available with higher exporter overhead. We document all three points in [`RUNBOOK.md`](../../RUNBOOK.md).
- The decision to *not* introduce a vendor-discriminator field on `GpuPressure` (e.g. `vendor: GpuVendor`) is intentional. Routers should not branch on vendor; if a vendor-specific signal matters, it lives in a vendor-specific trait method, not in the canonical struct.
- The relationship to [ADR `0024`](0024-ebpf-via-aya.md) is one of orthogonality: BPF programs can supplement GPU-pressure data with kernel-side syscall-attribution signals, but the primary signal source for `BackendSignals::gpu_pressure` is the GPU-management surface (DCGM/NVML/ROCm SMI), not the kernel.
- Reading from existing observability infrastructure (DCGM exporter) rather than re-implementing GPU telemetry in Riftgate is the right discipline. We are not building a GPU monitoring product; we are building a gateway that consumes existing GPU monitoring well.
