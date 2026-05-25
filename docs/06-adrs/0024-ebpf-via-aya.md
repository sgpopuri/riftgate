# ADR 0024. eBPF integration via Aya (pure-Rust BPF), Linux 5.15+, feature-gated and opt-in

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [014-ebpf-integration](../05-options/014-ebpf-integration.md)
> **Deciders:** Sriram Popuri

## Context

`v0.4`'s differentiation pillar is integrated, gateway-internal eBPF observability — not bolted-on. The [observability-plane document](../03-architecture/observability-plane.md) names three concrete BPF programs (CPU on/off-time profiling, syscall stalls, TCP retransmits) and the [observability LLD](../04-design/lld-observability.md) reserves `BpfSink` as a `v0.4` impl behind the same `ObservabilitySink` trait already shipped in `v0.1`. Five candidates (none, `bpftrace`, `libbpf-rs`, Aya, hybrid) were evaluated in [Options `014`](../05-options/014-ebpf-integration.md); the choice is between Aya's pure-Rust ergonomics, libbpf-rs's C-authoring maturity, and bolted-on alternatives that would forfeit the differentiation pillar.

## Decision

**`v0.4` adopts Aya as the BPF runtime and authoring substrate; programs are pure Rust on both sides; integration is feature-gated (`bpf`), Linux-only (`cfg(target_os = "linux")`), and opt-in at runtime via `RIFTGATE_ENABLE_BPF=1`. libbpf-rs and `bpftrace` are rejected; the hybrid path is not the default and is not implemented in `v0.4`.**

- New module `crates/riftgate-obs/src/bpf/` gated by `cfg(all(target_os = "linux", feature = "bpf"))`. Cargo dependencies follow the `crates/riftgate-io-uring/Cargo.toml` `[target.'cfg(target_os = "linux")'.dependencies]` pattern. macOS builds compile cleanly with no BPF code.
- BPF programs live in a sibling crate `crates/riftgate-obs-bpf` compiled to the `bpfel-unknown-none` target (Aya convention separating userspace from BPF-target compilation). Userspace loads via Aya's `BpfLoader`.
- Kernel-version floor: Linux 5.15 (Ubuntu 22.04 LTS / RHEL 9 baseline). Documented in [`RUNBOOK.md`](../../RUNBOOK.md). Older kernels: unsupported.
- Runtime activation: `RIFTGATE_ENABLE_BPF=1` is required; absence is a no-op. Process must have `CAP_BPF` (or `CAP_SYS_ADMIN` on pre-5.8 kernels we tolerate but do not officially support).
- `BpfSink` implements the existing `ObservabilitySink` trait; events flow through the bounded MPSC bus from [ADR `0011`](0011-otel-default-sink-multisink-fanout.md). No new trait. No new bus.
- The three `v0.4` programs from the [observability-plane document](../03-architecture/observability-plane.md) land first: CPU on/off-time profiling (sampled stack traces aggregated per shard, default sample rate 19 Hz matching Linux `perf`), syscall stalls (tracepoint-based), TCP retransmits per upstream (kprobe-based on TCP layer).
- GPU-pressure correlation is governed by [Options `028`](../05-options/028-gpu-pressure-correlation.md) / [ADR `0026`](0026-gpu-pressure-via-dcgm-exporter.md), *not* by this ADR. Whether the GPU pressure signal flows through Aya BPF or DCGM scrape is a separate decision.

## Consequences

- **Positive:**
  - Pure-Rust toolchain on both sides; no C compiler in the BPF authoring path; no `clang -target bpf` step; no `libbpf` header dependency. Matches Riftgate's "Rust, not C, in the data plane" bedrock decision ([ADR `0001`](0001-rust-not-go-or-zig.md)).
  - Single-binary distribution preserved (`NFR-OPS02`): no `bpftrace` or `bcc` runtime dependency.
  - CO-RE (Compile Once Run Everywhere) supported; same BPF object loads on Linux 5.15 through 6.6+ via BTF relocation.
  - Crate layout matches the existing `crates/riftgate-io-uring` pattern; macOS contributors see no regression.
  - `BpfSink` is a sink among sinks; the trait surface in `riftgate-core` is unchanged.
  - The opt-in `RIFTGATE_ENABLE_BPF=1` gate gives operators a one-flag escape from any BPF-specific issue (verifier rejection on a new kernel, transient driver bug).
- **Negative / accepted tradeoffs:**
  - Aya is less battle-worn than libbpf; verifier errors and edge-case BPF program types surface as Aya-specific issues rather than well-known libbpf ones. Mitigated by pinning Aya versions and reading the Aya release notes carefully.
  - New BPF features (kfuncs, new program types) typically land in libbpf before Aya; we may wait up to ~6 months for parity on a feature we want.
  - The verifier-debug experience is real work; we accept the cost as part of choosing pure Rust.
  - `cargo build --workspace --all-features` on Linux now includes the BPF compile step; CI cost increases marginally. Acceptable.
- **Future work this enables:**
  - GPU-pressure correlation can elect a BPF-supplementary signal layer if [ADR `0026`](0026-gpu-pressure-via-dcgm-exporter.md)'s DCGM-scrape primary proves insufficient.
  - Token-level metrics ([ADR `0025`](0025-token-level-metrics-probabilistic.md)) can elect BPF-source byte-egress timestamps for sub-millisecond inter-token-latency precision.
  - Continuous profiling output (`pprof`-compatible profile bodies) can land as an OTel profiling-signal export once that signal exits experimental.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship `bpftrace` scripts as part of the gateway's loaded BPF surface (operators may still run them ad-hoc; the [`RUNBOOK.md`](../../RUNBOOK.md) documents this).
  - Riftgate will not ship C-authored BPF programs in `v0.4`.
  - Riftgate will not load BPF programs without `RIFTGATE_ENABLE_BPF=1`. The default deployment runs with no elevated kernel-level capability.

## Compliance

- `crates/riftgate-obs/src/bpf/` exists only behind `cfg(all(target_os = "linux", feature = "bpf"))`. CI runs both with and without the feature on Linux; CI on macOS runs without (and never compiles BPF code).
- `crates/riftgate-obs-bpf/` builds for the `bpfel-unknown-none` target as a separate crate target; `cargo xtask build-bpf` is the convenience command.
- `crates/riftgate-obs/tests/bpf_verifier.rs` exercises every shipped BPF program against the verifier on the Linux 5.15 LTS and 6.1 LTS kernels via a containerized harness; failures are CI gates.
- Adding a new BPF program does **not** require a new ADR. Switching the runtime substrate (Aya → libbpf-rs, or adding the hybrid path) **does**.
- `RIFTGATE_ENABLE_BPF` is documented in [`docs/05-options/015-config-model.md`](../05-options/015-config-model.md)'s environment-override section and in [`RUNBOOK.md`](../../RUNBOOK.md).

## Notes

- The kernel-version floor (Linux 5.15) is conservative: 5.8+ technically supports the BPF features we need with `CAP_BPF`, but 5.15 is the LTS line that production fleets (Ubuntu 22.04, RHEL 9) actually run. We document both numbers and support 5.15+.
- The 19 Hz default sample rate for continuous profiling matches Linux `perf`'s default; operators familiar with `perf record` will see identical sampling overhead. Higher rates are available via config (`profile.sample_hz`) with documented CPU-cost increase.
- The choice to ship BPF programs in a sibling crate (`crates/riftgate-obs-bpf`) rather than inside `crates/riftgate-obs/src/bpf/` is structural: BPF-target compilation has different `rustc` flags from userspace compilation, and a separate crate is the cleanest way to express that. The userspace `bpf` module loads the compiled BPF objects via `include_bytes!`.
- We deliberately reject reading from existing `bcc`-based fleet profilers (Parca-agent, Pyroscope) as a substitute. They are valuable supplementary tools; they are not the *integrated* path Riftgate's third differentiation pillar requires.
