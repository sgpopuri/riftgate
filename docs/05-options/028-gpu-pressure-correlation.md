# 028. GPU pressure correlation: how Riftgate reads DCGM/NVML signals and merges them into `BackendSignals::gpu_pressure`

> **Status:** recommended
> **Foundational topics:** NVIDIA DCGM (Data Center GPU Manager) exporter and HTTP scraping, NVML (NVIDIA Management Library) in-process FFI, sidecar / ambassador deployment patterns, observability-versus-routing-signal decoupling, multi-vendor GPU telemetry surfaces (AMD ROCm SMI, Intel Habana, MIG partitioning)
> **Related options:** [010](010-routing-strategy.md), [013](013-observability-sink.md), [014](014-ebpf-integration.md), [025](025-v03-routing-strategies.md), [027](027-token-level-metrics.md)
> **Related ADR:** [ADR 0026](../06-adrs/0026-gpu-pressure-via-dcgm-exporter.md)

## 1. The decision in one sentence

> Pick the signal source and integration topology for backend GPU pressure — the field `BackendSignals::gpu_pressure` that routers read and that the observability plane joins to per-request spans.

## 2. Context — what forces this decision

[`docs/04-design/lld-routing.md`](../04-design/lld-routing.md) declares `BackendSignals` as the read-only signal channel routers consume on every `route()` call. `gpu_pressure` is the field that drives weighted-random rebalancing when GPUs are saturated, that feeds the v0.3 `HedgedRouter` ([ADR `0023`](../06-adrs/0023-hedged-requests-p99-triggered.md))'s decision to escalate, and that — per the [observability-plane document](../03-architecture/observability-plane.md) — is one of the three things `v0.4`'s eBPF programs were meant to surface.

The bedrock question is whose telemetry we believe, in what format, and with what coupling cost. The candidates differ in three dimensions:

1. **Where the signal originates.** NVIDIA's official surfaces are NVML (per-host C library, runs in-process) and DCGM (NVIDIA's monitoring daemon with an exporter exposing Prometheus-format metrics over HTTP).
2. **Where the reading happens.** In-process FFI from Riftgate, out-of-process scrape against a DCGM exporter, sidecar-injected envelope, or eBPF kprobing of NVIDIA driver entry points.
3. **What cadence the signal updates.** Per-routing-decision (sub-millisecond), per-second, per-10-seconds, per-event-only.

Three concrete forces:

- **`AGENTS.md` §5 ("Pluggability over performance").** Whatever we pick, the field type `BackendSignals::gpu_pressure: f32` must remain stable. Future GPU vendors (AMD ROCm via `rocm-smi`, Intel via Habana exporters, MIG-partitioned slices) must be representable behind the same field. We do not bake `nvmlDeviceHandle_t` into the routing API.
- **Single-binary distribution (`NFR-OPS02`).** Linking against NVML pulls in NVIDIA's proprietary CUDA dynamic libraries (`libnvidia-ml.so.1`) at runtime. On hosts without an NVIDIA GPU, the linker either fails at startup or we ship a stub. Either decision is an operator-visible quirk.
- **Multi-vendor reality.** The 2026 LLM-inference fleet is no longer NVIDIA-only. AMD MI300X and MI325X are in production at multiple cloud providers; Habana Gaudi-2 / Gaudi-3 deployments exist; Inferentia / Trainium on AWS are present. A signal source that *only* works for NVIDIA forecloses on `v1.0+` multi-vendor reach.

The `v0.4` milestone needs *a* working signal source for NVIDIA backends (the dominant case), with a clear extension story for the rest.

## 3. Candidates

### 3.1. NVML in-process FFI

**What it is.** Link against `libnvidia-ml.so.1` at runtime via the `nvml-wrapper` Rust crate. Each Riftgate process directly queries device utilization, memory pressure, throttle reasons, and per-process GPU memory usage via NVML calls. Updates can be on-demand (sub-millisecond per call) or polled in a background task.

**Why it's interesting.** Lowest latency from query to answer. No external process or network hop. Per-process GPU memory accounting is uniquely available through NVML (DCGM exposes it only via host-level aggregates). The Rust binding is well-maintained.

**Where it falls short.** Couples Riftgate's runtime to NVIDIA's proprietary library. `cargo build` on a developer laptop without `libnvidia-ml` either fails or requires a feature gate. Most production gateways run on the *load-balancer* side of the architecture, not co-located with the GPU host; Riftgate-on-the-LB-host cannot read GPUs it doesn't have. NVIDIA-only by definition. The library has historical ABI quirks (driver version mismatches surface as `NVML_ERROR_LIB_RM_VERSION_MISMATCH`); production deployments need careful library-version pinning.

**Real-world systems that use it.** Triton Inference Server, NVIDIA's internal tooling, GPU-resident services like Determined AI's agent. Almost always running *on the GPU host*, not on a remote gateway.

### 3.2. DCGM exporter HTTP scrape

**What it is.** Operators run NVIDIA's `dcgm-exporter` (a container) as a DaemonSet on every GPU node; it exposes a Prometheus-format `/metrics` endpoint with per-GPU and per-MIG metrics. Riftgate scrapes the endpoint at a configurable cadence (default 5 s) per backend, parses the relevant metrics, and updates `BackendSignals::gpu_pressure` for that backend.

**Why it's interesting.** Decoupled: no NVIDIA library in Riftgate's link line. DCGM is NVIDIA's *production-recommended* surface for monitoring — the same telemetry that fleet management at NVIDIA, AWS, GCP, and Azure consume. Prometheus format is universal; if an operator already runs DCGM exporter (most do), Riftgate consumes the same data their fleet does. Multi-tenant by design (DCGM handles per-MIG partitioning). The HTTP-scrape model treats GPU pressure as a *signal* (read-only, with documented staleness), not as a runtime dependency.

**Where it falls short.** Update cadence is bounded by scrape interval; 1-second granularity is the realistic floor (DCGM's own collection cycle is 1 s by default, configurable down to 100 ms with higher exporter overhead). Adds an HTTP-client dependency and a per-backend scrape task to Riftgate. Requires operators to run `dcgm-exporter` somewhere reachable — production sites already do, but it's a documented prerequisite. The HTTP path is one more thing that can fail (timeouts, partial reads, scrape lag during GPU-driver hangs).

**Real-world systems that use it.** Most production Kubernetes inference deployments. The official NVIDIA GPU operator ships DCGM exporter. Grafana dashboards for GPU monitoring assume this surface. AWS EKS NVIDIA add-on uses it.

### 3.3. Sidecar / ambassador exposing a Riftgate-internal protocol

**What it is.** A purpose-built sidecar process — written in Rust, deployed alongside each GPU backend — speaks NVML on the GPU host and exposes a small gRPC or HTTP/2 surface specific to Riftgate. The sidecar handles vendor multiplexing (NVML on NVIDIA hosts, `rocm-smi` on AMD hosts) and presents a unified `BackendPressureReport` to Riftgate. Riftgate subscribes once per backend; the sidecar streams updates.

**Why it's interesting.** Best long-term abstraction: Riftgate sees a vendor-neutral, Riftgate-shaped surface. The vendor-coupling cost lives in the sidecar, not in the gateway. Push semantics (sidecar streams to gateway) bound latency better than pull (HTTP scrape). The sidecar is a natural place to bundle DCGM, NVML, ROCm SMI, Habana exporter behind one wire-format.

**Where it falls short.** Requires Riftgate to ship and maintain a second binary (`riftgate-gpu-sidecar`) — a milestone-by-itself, and a thing operators must deploy alongside the gateway. The maintenance cost is real: NVIDIA driver changes, AMD ROCm minor revisions, kernel-level GPU partitioning changes all flow through this sidecar. Production sites *already* run DCGM exporter; asking them to *also* run a Riftgate-specific sidecar is a deployment-burden regression.

**Real-world systems that use it.** Service-mesh sidecars (Istio, Linkerd) provide the canonical pattern. No GPU-specific production sidecar exists at this exact shape in the open-source ecosystem.

### 3.4. eBPF kprobes on NVIDIA driver entry points

**What it is.** Use Aya BPF programs (per [Options `014`](014-ebpf-integration.md)) to attach kprobes on NVIDIA driver functions (e.g. `nvidia_open`, `nvidia_ioctl`, kernel-side `__nv_NvKmsIoctl`) and infer GPU pressure from observed syscall rates and latencies. Pure observation, no NVML/DCGM dependency.

**Why it's interesting.** Zero NVIDIA-library coupling. Joins naturally to per-request spans (the eBPF side already runs in `v0.4` for other reasons per Options `014`). Captures *real* per-process pressure — the syscall-rate signal is observed from inside the kernel and reflects exactly what the GPU is being asked to do.

**Where it falls short.** Inference from syscall rates to "GPU memory pressure" or "throttle state" is *indirect*. Throttle and ECC error states are *not* visible at the syscall boundary; they live inside the GPU driver's memory and require the driver's own management interface to surface. NVIDIA's proprietary driver does not expose stable internal symbols (the `nvidia_*` kernel module symbols are subject to change between driver versions), so kprobing them is a non-CO-RE story — every driver minor bump risks breaking the probes. The kernel community has, multiple times, documented friction between BPF and the NVIDIA proprietary driver.

**Real-world systems that use it.** Research papers and one-off SRE investigation tools. No production gateway uses BPF as its primary GPU-pressure signal.

### 3.5. Hybrid — DCGM scrape primary, NVML in-process secondary behind a feature gate

**What it is.** `§3.2`'s DCGM HTTP scrape is the default and the documented path. NVML in-process FFI is available behind a `gpu-nvml` feature gate for operators who run Riftgate *on the GPU host itself* (gateway-co-located deployments) and want sub-second granularity. Both implement the same `GpuPressureSource` trait that `BackendSignals` consumes; operators pick one per backend in TOML.

**Why it's interesting.** Gives operators the choice that matches their topology. Decoupled deployments (Riftgate on LB tier, GPUs elsewhere) get DCGM. Co-located deployments (Riftgate on the inference node) get NVML's lower latency. The trait surface absorbs vendor diversity.

**Where it falls short.** Two code paths to maintain in `v0.4`. The NVML path needs the feature-gated empty-lib-elsewhere pattern (per `crates/riftgate-io-uring/Cargo.toml`'s shape). The trait abstraction is real maintenance work — but it is also exactly the "pluggability over performance" discipline `AGENTS.md` §5 requires.

## 4. Tradeoff matrix

| Property | 3.1 NVML | 3.2 DCGM scrape | 3.3 Sidecar | 3.4 eBPF | 3.5 Hybrid | Why it matters |
|----------|----------|-----------------|--------------|----------|-------------|----------------|
| NVIDIA-only coupling | yes (hard link) | yes (data semantics) | abstracted by sidecar | yes (driver symbols) | yes default; hybrid escape | Multi-vendor reality of 2026. |
| Linker/runtime dependency on `libnvidia-ml.so` | yes (hard) | no | only inside sidecar | no | only behind feature gate | `NFR-OPS02` single-binary impact. |
| Works on Riftgate-on-LB topology (no co-located GPU) | no | yes | yes | no | yes (when DCGM picked) | Most production gateways are not GPU-co-located. |
| Update cadence | sub-ms on demand | ~1 s (scrape) | streaming push | event-driven | both | Routing decisions don't need sub-ms; 1 s is fine. |
| Multi-vendor extension story | NVIDIA only | NVIDIA today; AMD via ROCm SMI exporter | clean (sidecar absorbs) | NVIDIA-driver-specific | clean (per backend) | `v1.0+` reach. |
| Signal fidelity (throttle state, ECC errors) | yes | yes (DCGM exposes) | yes (via NVML) | no (inferred) | yes | Routing wants accurate "is this GPU sick?" signal. |
| Adds a new Riftgate-maintained binary | no | no | yes (sidecar) | no | no | Maintenance burden. |
| Familiarity to NVIDIA-using operators | medium | very high | low | low | high | Adoption friction. |
| Coupling to the eBPF substrate decision (Options `014`) | none | none | none | hard | optional | Decoupling decisions is good. |
| Per-MIG partition support | yes | yes | yes (via NVML) | partial | yes | Production GPU partitioning matters. |
| Per-process GPU memory accounting | yes (NVML-only) | no (host-aggregate only) | yes (via NVML) | no | yes (when NVML picked) | Some advanced routers want this. |
| `v0.4` implementation cost | medium | low | very high | high | medium-high | Milestone scope. |

## 5. Foundational principles

The pattern this decision instantiates is the *external signal vs in-process signal* split that runs through every distributed-systems telemetry decision. In-process signals (NVML FFI, eBPF) are low-latency but couple the consumer to the producer's runtime; external signals (DCGM exporter, sidecar) decouple at the cost of staleness and an additional network hop. The standard reference is Hohpe and Woolf's *Enterprise Integration Patterns* [1] (sidecar / ambassador chapter) and the more recent Microsoft *Cloud Design Patterns* [2] on the ambassador pattern.

NVIDIA's specific telemetry surface stack is documented end-to-end in NVIDIA's developer documentation [3, 4, 5]. NVML [4] is the in-process C API for "fetch one fact about one GPU now"; DCGM [5] is the daemon that aggregates NVML across hosts, exposes a gRPC API, ships a Prometheus exporter [6], and handles policy concerns (health checks, ECC error accounting, driver-version compatibility). The Kubernetes ecosystem standardised on DCGM exporter for GPU monitoring via the NVIDIA GPU operator [7]; this is the path with the strongest production track record and the broadest tooling support.

The multi-vendor abstraction principle traces to OpenTelemetry's vendor-neutral telemetry stance: capture the *semantics* (utilization, memory pressure, throttle state) not the *source*. AMD's ROCm SMI exporter [8] presents the same Prometheus metric shape (different metric names, identical semantic categories); Habana's `hl-smi` similarly. A signal-source decision that picks a *vendor* forecloses on multi-vendor; a decision that picks a *protocol* (Prometheus scrape) absorbs the vendor diversity at the configuration layer.

The routing-versus-observability decoupling principle is articulated by the routing LLD itself [9]: routers consume read-only signals via `BackendSignals`; they do not perform any I/O on the routing hot path. Whatever signal source we pick must publish into `BackendSignals` *out-of-band* (a background task on the gateway, with documented staleness), never inline with `route()`. This is the same pattern circuit-breaker state already follows.

The observability-versus-routing separation has a second face: GPU pressure shows up in *both* surfaces. Routers want it as a `f32` field on `BackendSignals`; OTel/Prometheus operators want it as a histogram in dashboards. The signal source must feed both. DCGM exporter already feeds the OTel/Prometheus side (operators scrape it directly); Riftgate's incremental work is the *consumption* into `BackendSignals`, not the production of the signal itself.

## 6. Recommendation

**Adopt `§3.5` — DCGM exporter scrape as the default `GpuPressureSource` impl, with NVML in-process FFI available behind the `gpu-nvml` feature gate for operators on GPU-co-located topologies.** Reject `§3.3` and `§3.4` for `v0.4`; document the sidecar option (`§3.3`) as the long-term multi-vendor strategy for `v1.0+`.

Concretely:

- **New trait** `GpuPressureSource` in `crates/riftgate-core/src/gpu.rs`, with `fn current(&self, backend: BackendId) -> Option<GpuPressure>`. `GpuPressure` is a struct of `utilization_pct: f32, memory_used_pct: f32, throttle_state: ThrottleState, ecc_errors_total: u64, observed_at: Instant`. Routers continue to consume the existing `BackendSignals::gpu_pressure: f32` field, which is a derived single-axis summary (`utilization_pct.max(memory_used_pct)`) of the richer struct.
- **`DcgmScrapeSource`** in `crates/riftgate-obs/src/gpu/dcgm.rs` is the default impl. Configuration via TOML:
  ```
  [backends.gpu0.gpu_pressure_source.dcgm]
  endpoint = "http://gpu-node-1.internal:9400/metrics"
  scrape_interval_ms = 5000
  gpu_index = 0  # which GPU on the node maps to this backend
  ```
  Scraping happens in a background task per backend; results are written into an `ArcSwap<GpuPressure>` that the routing hot path reads with no lock. Staleness is documented (`scrape_interval_ms + network jitter`); routers tolerate stale data — that's the whole point of `BackendSignals`.
- **`NvmlSource`** in `crates/riftgate-obs/src/gpu/nvml.rs` is feature-gated `gpu-nvml`. Configuration via TOML when the feature is enabled:
  ```
  [backends.gpu0.gpu_pressure_source.nvml]
  device_uuid = "GPU-abc123..."
  poll_interval_ms = 500
  ```
  Crate layout mirrors `crates/riftgate-io-uring/Cargo.toml`: `[target.'cfg(target_os = "linux")'.dependencies]` for `nvml-wrapper`, `cfg(all(target_os = "linux", feature = "gpu-nvml"))` gates on every module, empty-lib elsewhere. Production deployments without an NVIDIA GPU compile cleanly with no behavioural change.
- **`NoopGpuSource`** is the default for backends without an explicit `gpu_pressure_source` block. Returns `None`; routers see `BackendSignals::gpu_pressure = 0.0` and route accordingly.
- **Observability join.** `DcgmScrapeSource` and `NvmlSource` publish `Histogram` events for `riftgate_backend_gpu_utilization_pct` and `riftgate_backend_gpu_memory_used_pct` per backend on the existing observability bus. The eBPF integration ([Options `014`](014-ebpf-integration.md)) remains optional and orthogonal — BPF programs can attribute syscall costs to specific GPUs, but they do not produce the primary `BackendSignals::gpu_pressure` value.
- **Multi-vendor today.** AMD ROCm SMI exporter [8] uses Prometheus scrape format; an operator can point `DcgmScrapeSource` at an AMD exporter endpoint and the field translation works for utilization and memory. We document this as a known-working path. Habana and Inferentia stay out of `v0.4` scope; revisit if operator demand surfaces.
- **MIG partitioning.** When a backend addresses a MIG slice rather than a whole GPU, the DCGM exporter exposes per-MIG metrics with `migprofile` labels; `DcgmScrapeSource` accepts a `mig_uuid` configuration and reads the matching metric. NVML similarly.

**Conditions to revisit:**

- A production operator surfaces a `v0.4`-shipping deployment where DCGM exporter is unavailable and the eBPF or sidecar path is operationally preferable. Trigger `§3.3` or `§3.4` as an additional impl behind the same trait.
- AMD or Habana operator demand grows past the "we tolerate Prometheus scrape" threshold and requires native ROCm SMI / Habana integration. Add a vendor-specific impl behind the `GpuPressureSource` trait.
- The 5-second default scrape interval proves insufficient for fast-routing decisions. Either tighten the interval (DCGM supports down to 100 ms with higher exporter overhead) or escalate to `§3.3`'s push semantics.
- DCGM exporter's metric naming changes incompatibly between releases (it has historically been stable; this would be unprecedented). Pin the supported version range in `RUNBOOK.md`.

**Non-default candidates kept available:**

- The sidecar option (`§3.3`) is documented in the LLD's "deferred for `v1.0+` multi-vendor strategy" section. No `v0.4` implementation.
- The eBPF option (`§3.4`) is documented in the [eBPF Options doc](014-ebpf-integration.md) as a possible supplementary signal source, not a primary one.

## 7. What we explicitly reject

- **NVML-only (`§3.1`) as the default.** Couples Riftgate to NVIDIA's library and forecloses on Riftgate-on-LB topologies. Revisit only if every Riftgate deployment becomes GPU-co-located, which is contrary to the documented production pattern.
- **Pure sidecar (`§3.3`).** Adds a second Riftgate-maintained binary with its own release cycle and operator deployment burden. Revisit when multi-vendor reach forces it (likely `v1.0`+).
- **Pure eBPF (`§3.4`).** Driver-symbol fragility and inability to surface throttle/ECC state make it the wrong primary source. Useful as a *supplementary* signal layer joined to per-request spans; the eBPF Options doc handles that question.
- **Adding `nvmlDeviceHandle_t` or any vendor-specific type to `BackendSignals`.** The field stays `gpu_pressure: f32`; the rich struct lives behind the `GpuPressureSource` trait. Revisit only with an ADR superseding [ADR `0014`](../06-adrs/0014-weighted-random-router.md).

## 8. References

1. Gregor Hohpe, Bobby Woolf. *Enterprise Integration Patterns.* Addison-Wesley, 2003. ISBN 978-0321200686.
2. Microsoft. *Cloud Design Patterns — Ambassador.* <https://learn.microsoft.com/en-us/azure/architecture/patterns/ambassador>
3. NVIDIA. *Data Center GPU Manager Overview.* <https://developer.nvidia.com/dcgm>
4. NVIDIA. *NVIDIA Management Library (NVML) Reference.* <https://docs.nvidia.com/deploy/nvml-api/>
5. NVIDIA. *DCGM User Guide.* <https://docs.nvidia.com/datacenter/dcgm/latest/user-guide/>
6. NVIDIA `dcgm-exporter`. <https://github.com/NVIDIA/dcgm-exporter>
7. NVIDIA GPU Operator for Kubernetes. <https://github.com/NVIDIA/gpu-operator>
8. AMD ROCm SMI Exporter. <https://github.com/ROCm/rocm_smi_lib>
9. Riftgate. [`docs/04-design/lld-routing.md`](../04-design/lld-routing.md).
10. `nvml-wrapper` Rust crate. <https://github.com/Cldfire/nvml-wrapper>
11. CNCF Sidecar pattern. <https://www.cncf.io/blog/2022/01/27/sidecar-pattern/>
12. NVIDIA driver and BPF interaction discussion. <https://lore.kernel.org/bpf/> (search for nvidia)
