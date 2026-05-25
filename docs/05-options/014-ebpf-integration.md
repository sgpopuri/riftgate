# 014. eBPF integration: how Riftgate observes the kernel and the GPU edges of its own data plane

> **Status:** recommended
> **Foundational topics:** eBPF (verifier, JIT, maps, CO-RE), kprobes / tracepoints / USDT, XDP / TC / LSM attachment points, Aya pure-Rust BPF runtime, libbpf and `libbpf-rs`, `bpftrace`, NVIDIA DCGM (correlation target, governed separately by [Options `028`](028-gpu-pressure-correlation.md))
> **Related options:** [013](013-observability-sink.md), [001](001-io-model.md), [015](015-config-model.md), [027](027-token-level-metrics.md), [028](028-gpu-pressure-correlation.md)
> **Related ADR:** [ADR 0024](../06-adrs/0024-ebpf-via-aya.md)

## 1. The decision in one sentence

> Pick the eBPF runtime and authoring substrate for Riftgate's `v0.4` observability plane — the way we load, attach, and read from BPF programs that profile the gateway and correlate kernel-side signals (CPU on/off-time, syscall stalls, NUMA misses, TCP retransmits) with per-request span data.

## 2. Context — what forces this decision

[`docs/03-architecture/observability-plane.md`](../03-architecture/observability-plane.md) names eBPF as the third differentiation pillar — *integrated* gateway-internal observability rather than a bolted-on sidecar. [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md) reserves `BpfSink` as a `v0.4` impl behind the same `ObservabilitySink` trait that already ships in `v0.1`. The [observability-plane document](../03-architecture/observability-plane.md) names three concrete things the BPF programs must do:

1. Continuous gateway profiling (CPU on/off-time per worker, syscall counts, NUMA misses, page faults).
2. Backend GPU pressure correlation (DCGM / NVML signals merged into `BackendSignals::gpu_pressure`, the routing-input field declared at [`docs/04-design/lld-routing.md`](../04-design/lld-routing.md)). *The signal-source decision lives in [Options `028`](028-gpu-pressure-correlation.md); this Options doc only commits to the eBPF-side surface area.*
3. TCP-level observability (retransmits per upstream, RTT histograms, accept-queue depth).

This is non-negotiable for `v0.4`. What is open is *how* we author and load the programs, and what runtime substrate sits in `crates/riftgate-obs` on Linux. The choice has long-tail consequences: it constrains the kernel version floor (`NFR-PORT01`), the C-toolchain dependency (`NFR-OPS02`, "single-binary distribution"), the verifier-debug experience of every future contributor, and whether the gateway author writes BPF in C, Rust, or a DSL.

Riftgate already ships a Linux-only crate template (`crates/riftgate-io-uring`) that compiles cleanly on macOS as an empty library via `[target.'cfg(target_os = "linux")'.dependencies]`. Whatever runtime we pick must follow that pattern so `cargo build --workspace --all-features` on macOS does not regress (`NFR-PORT02`).

The `v0.1` `ObservabilitySink` trait surface is frozen. The eBPF side must publish `ObservabilityEvent` values into the existing bounded MPSC bus ([ADR `0011`](../06-adrs/0011-otel-default-sink-multisink-fanout.md)); no new trait, no new bus.

## 3. Candidates

### 3.1. None — bolted-on external profiler (declined baseline)

**What it is.** Don't ship BPF integration inside the gateway. Document that operators run `bpftrace`, `bcc`, `parca`, or `pyroscope` independently and correlate by hand against Riftgate's OTel spans via `pid`, `tid`, and timestamp.

**Why it's interesting.** Zero new dependencies. No verifier surface to maintain. The Linux performance-tooling ecosystem is mature and operators already know it. Riftgate stays narrowly-scoped to the gateway and refuses to be a profiler.

**Where it falls short.** This is exactly the integration story Riftgate was built to differentiate against. The third differentiation pillar in [`docs/00-vision.md`](../00-vision.md) and [`AGENTS.md`](../../AGENTS.md) §9 is "integrated eBPF observability, gateway-internal not bolted-on." Choosing this candidate is choosing not to ship the `v0.4` milestone. It also pushes the correlation cost onto the operator — joining `bpftrace` output to OTel spans via `pid` + timestamp is brittle, manual, and impossible to do at the trace-attribute level (where Riftgate wants the signal to land).

**Real-world systems that use it.** Most Envoy and HAProxy deployments. The convention in `kube-prometheus-stack`: Pyroscope or Parca runs as a DaemonSet and joins to traces via PID lookup. It works; it does not differentiate.

### 3.2. `bpftrace` — DSL-authored programs invoked out-of-process

**What it is.** Author BPF programs in `bpftrace`'s awk-ish DSL. Riftgate ships scripts under `crates/riftgate-obs/bpf/`; an out-of-process `bpftrace` invocation loads them, attaches probes, and streams JSON output back to Riftgate via a Unix domain socket or stdout pipe. Iovisor / Brendan Gregg lineage.

**Why it's interesting.** The most ergonomic authoring experience of any candidate. One-line probes like `tracepoint:sched:sched_switch { @[pid] = count(); }` are trivially readable. The DSL hides the verifier almost entirely. `bpftrace` runtime errors are clear. Iteration is fast.

**Where it falls short.** Requires the `bpftrace` binary installed on the target host (`NFR-OPS02` violation — Riftgate is not a single binary anymore). Cross-process IPC for every BPF event adds serialization cost on the publish path. `bpftrace` is a DSL, not a library: there is no Rust-typed event API; everything is stringly-typed JSON from a pipe. Probe lifecycle and attachment correctness depend on a separate process that can crash or get OOM-killed independently of the gateway. Production deployments — especially `distroless`-style containers — typically do not include `bpftrace`.

**Real-world systems that use it.** Most ad-hoc Linux performance investigation; almost no production daemon embeds it as an integral observability path.

### 3.3. `libbpf-rs` — C-authored BPF, Rust loader

**What it is.** Author BPF programs in C against `libbpf` (the kernel project's blessed userspace BPF library), compile with `clang -target bpf`, and load them from Rust via `libbpf-rs`. CO-RE-compatible — BPF object files relocate against the running kernel's BTF.

**Why it's interesting.** The most "blessed" path: `libbpf` is maintained alongside the kernel. CO-RE is a first-class feature, so the same BPF object loads on Linux 5.15 and 6.6 without recompilation. Production Cilium, Pixie, and Grafana Beyla use this approach. The C authoring is verbose but the verifier diagnostics are well-understood by anyone who has shipped BPF before.

**Where it falls short.** Adds a C toolchain to the build: `clang`, `bpftool`, `libbpf` headers. Riftgate's bedrock decision is "Rust, not C, in the data plane" ([ADR `0001`](../06-adrs/0001-rust-not-go-or-zig.md)); reintroducing C — even in an out-of-band BPF program directory — multiplies the build matrix and complicates the contributor story. CI must run `clang -target bpf`, ship vmlinux.h or rely on host BTF, and validate the produced object against multiple kernels. The Rust side is a thin loader, but the BPF authoring side is C with its full complement of footguns.

**Real-world systems that use it.** Cilium (the canonical example). Grafana Beyla. Parca-agent. Most production-grade BPF tooling in 2026 still has C-authored BPF.

### 3.4. Aya — pure-Rust BPF, end to end

**What it is.** Author BPF programs in Rust against the [`aya-ebpf`](https://github.com/aya-rs/aya) crate; load and attach from Rust via the `aya` crate. The BPF side compiles to BPF bytecode via `cargo`. CO-RE is supported via `aya-bpf-bindings` and BTF relocations against the running kernel.

**Why it's interesting.** Rust on both sides — no C toolchain, no `clang -target bpf` step beyond what `cargo` already does for the `bpfel-unknown-none` target. The Rust type system catches a class of BPF authoring errors before the verifier sees them. Active development with strong community traction in 2024–2026 (Cloudflare, Red Hat, Isovalent contributors). The crate ecosystem maps naturally to Riftgate's: empty-lib-elsewhere via `[target.'cfg(target_os = "linux")'.dependencies]`, exactly the pattern `crates/riftgate-io-uring` already uses.

**Where it falls short.** Younger than `libbpf-rs` — fewer years of production hardening. Some BPF features land in Aya after they land in `libbpf` (e.g. recent `kfunc` support was libbpf-first). Verifier errors come through Aya's error types, which are improving but still less battle-worn than libbpf's. A small subset of BPF program types (notably some XDP edge cases) historically had rough corners. The pure-Rust toolchain occasionally surfaces issues that libbpf hides (e.g. linker quirks on older Rust nightlies).

**Real-world systems that use it.** Aya is the substrate for several 2024–2026 production BPF projects (the Bottlerocket eBPF tooling, parts of the Kubewarden policy-engine BPF surface, internal observability stacks at multiple Cloudflare and Isovalent properties). Less battle-worn than libbpf but well past the "research project" threshold.

### 3.5. Hybrid — Aya for new code, libbpf-rs fallback when verifier or feature gaps bite

**What it is.** Default to Aya for `v0.4` programs. Document the conditions under which a specific program drops to `libbpf-rs` (verifier rejection on Aya path that succeeds on libbpf-compiled object; BPF feature only exposed via libbpf bindings). Build both into `crates/riftgate-obs` behind feature gates `bpf-aya` (default) and `bpf-libbpf` (escape hatch).

**Why it's interesting.** Pragmatic: we get Aya's authoring ergonomics for the 95% case and a documented escape hatch for the cases where Aya hits a wall.

**Where it falls short.** Two toolchains to support. CI matrix doubles for the BPF path. Contributors face the choice of "which substrate?" on every new probe, which is exactly the kind of friction the harness exists to remove. The escape hatch becomes load-bearing as soon as the first program lands on `libbpf-rs`, and the discipline to keep it small is fragile.

## 4. Tradeoff matrix

| Property | None | bpftrace | libbpf-rs | Aya | Hybrid | Why it matters |
|----------|------|----------|-----------|-----|--------|----------------|
| Single-binary distribution (`NFR-OPS02`) | yes | no (needs bpftrace bin) | yes (statically links libbpf) | yes (pure Rust) | yes | Riftgate ships one binary; runtime dependencies poison the operator experience. |
| Differentiation against LiteLLM / TensorZero | no | partial | yes | yes | yes | "Integrated eBPF, gateway-internal" is one of three pillars per AGENTS.md §9. |
| Rust-authoring on the BPF side | n/a | no (DSL) | no (C) | yes | partial | Riftgate is a Rust project; multi-language data-plane code adds reviewer cost. |
| CO-RE portability across kernels | n/a | n/a (bpftrace handles) | yes | yes | yes | Operators run mixed-kernel fleets (5.15 LTS through 6.6+). |
| Toolchain footprint | none | bpftrace binary | + clang, bpftool, libbpf headers | + nightly rust component for `bpfel-unknown-none` | both | CI cost and contributor-bootstrap friction. |
| Verifier-debug experience | n/a | DSL hides verifier | mature, well-known | improving, less mature than libbpf | best of both at higher cost | A bad day debugging the verifier is a long day; this matters more than it should. |
| Production-hardening track record | n/a | low (ad-hoc tool) | very high (Cilium et al.) | medium-high (growing) | high (with cost) | Riftgate is conservative about runtime substrates. |
| Compile path on macOS contributors | n/a | n/a | empty-lib via cfg | empty-lib via cfg | empty-lib via cfg | Cross-platform contributor on-ramp matters. |
| Integration with `ObservabilitySink` trait | bolted-on join | external pipe parser | direct Rust API | direct Rust API | direct Rust API | The trait is frozen; eBPF must publish through it. |
| Kernel-version floor | n/a | bpftrace-pinned | libbpf-pinned (5.4+) | aya-pinned (5.8+ for CO-RE) | max(libbpf, aya) | `NFR-PORT01` requires a documented floor. |
| Future feature parity with libbpf | n/a | n/a | reference | follows by ≤6 months | n/a | New kernel features (kfuncs, new program types) land in libbpf first. |

## 5. Foundational principles

The kernel's BPF subsystem [1] gives userspace a verified, JIT-compiled in-kernel virtual machine, accessed via the `bpf(2)` syscall family. The verifier walks every reachable instruction and rejects programs whose control flow or memory accesses cannot be proven safe; the JIT then emits native code for the target architecture. Maps are the in-kernel-to-userspace data transport. CO-RE [2] uses BTF (BPF Type Format) metadata to relocate field accesses at load time so a single BPF object works across kernel versions — the canonical reference is Andrii Nakryiko's CO-RE writeup [3]. Attachment points (kprobes / tracepoints / USDT / fentry / XDP / TC / LSM) are well-documented in the kernel tree [4].

The authoring-substrate question is therefore: which userspace toolchain produces the BPF object and loads it via `bpf(2)`? `libbpf` [5] is the kernel project's reference; `libbpf-rs` [6] is a Rust binding around it. Aya [7] is the alternative Rust-native path: it produces BPF bytecode from Rust source compiled to the `bpfel-unknown-none` target, loads via the same `bpf(2)` syscalls, and bypasses libbpf entirely (with the cost that bugs which would be caught by libbpf surface as Aya-specific issues instead).

The systems lineage for Riftgate's chosen scope of BPF use is Brendan Gregg's *BPF Performance Tools* [8] — CPU on/off-time, syscall stalls, TCP retransmits, NUMA effects. These are the questions an SRE asks at 3 a.m.; eBPF answers them with bounded overhead and no kernel patching. The continuous-profiling pattern (sampled stack traces aggregated into flame graphs) traces to Gregg's flame-graph work [9] and Facebook's BPF-based fleet profiler [10]; Parca-agent [11] and Pyroscope [12] are the modern open-source instantiations.

The "integrated, not bolted-on" framing is the differentiator: production gateways today either ship no first-class profiling (Envoy, HAProxy, traefik) or rely on a sidecar / DaemonSet (Cilium's hubble-relay for some signal classes). Riftgate's bet — articulated in [`docs/00-vision.md`](../00-vision.md) — is that operators want the BPF signal joined to the per-request OTel span *inside the gateway*, with no separate join step. That bet only pays off if the BPF runtime is in-process; bolted-on (`§3.1`) is structurally unable to deliver it.

The framework-not-product invariant ([`AGENTS.md`](../../AGENTS.md) §9) constrains the choice in a specific way: every subsystem must be a trait with multiple impls, and the eBPF runtime here is no exception. Whichever candidate wins, `BpfSink` must implement the `ObservabilitySink` trait that already governs `OtelSink`, `JsonStdoutSink`, and `PrometheusSink`. The eBPF side does not get its own trait surface or its own bus; it publishes to the same bounded MPSC channel ([ADR `0011`](../06-adrs/0011-otel-default-sink-multisink-fanout.md)) as every other sink.

## 6. Recommendation

**Adopt Aya (`§3.4`) as the `v0.4` eBPF runtime; reject the hybrid path until a concrete program forces it.**

- New crate scaffolding under `crates/riftgate-obs/src/bpf/` gated by `cfg(all(target_os = "linux", feature = "bpf"))`. The `bpf` feature is opt-in; default builds on Linux and all builds on macOS produce no BPF code. Pattern mirrors `crates/riftgate-io-uring/Cargo.toml`.
- BPF programs live in a sibling module `crates/riftgate-obs-bpf` (Aya convention separating userspace from BPF-target compilation) and are loaded via Aya's `BpfLoader` from the userspace side.
- Kernel-version floor: Linux 5.15 (Ubuntu 22.04 LTS / RHEL 9 baseline). Anything older is unsupported; we document this in [`RUNBOOK.md`](../../RUNBOOK.md).
- BPF is enabled only when `RIFTGATE_ENABLE_BPF=1` is set explicitly in the environment, and the process must have `CAP_BPF` (or `CAP_SYS_ADMIN` on pre-5.8 kernels we don't formally support but tolerate). Without the env var, no BPF programs load and the BPF sink is a no-op — matching the [observability-plane contract](../03-architecture/observability-plane.md).
- `BpfSink` implements `ObservabilitySink`; events flow through the existing bounded MPSC bus. No new trait. No new bus.
- The three programs from the [observability-plane document](../03-architecture/observability-plane.md) (CPU on/off-time, syscall stalls, TCP retransmits) land first. GPU-pressure correlation [Options `028`](028-gpu-pressure-correlation.md) is governed by its own decision and may or may not land via BPF — the signal source for DCGM/NVML is a separate question.
- Continuous-profiling output uses sampled stack-trace aggregation into a per-shard ring buffer, drained by a userspace worker into OTel as `pprof`-compatible profile bodies (or via OTel's profiling signal once it ships out of experimental). The sampling rate is operator-configurable (default 19 Hz, the Linux `perf` default).

**Conditions to revisit:**

- A specific BPF program we need (a new program type, a new kfunc, a verifier-acceptance scenario Aya cannot navigate) blocks on Aya. Trigger the hybrid path (`§3.5`) for that program only, with a paired ADR.
- Aya's release cadence falls more than 12 months behind libbpf for a feature we need. Revisit substrate choice.
- We add a non-Linux BPF-equivalent target (eBPF on Windows, DTrace on Solaris derivatives, BSD's eBPF port) — at which point the substrate choice becomes a per-OS question and the trait surface absorbs the difference. Not on the `v1.0` roadmap.

**Non-default candidates kept available:**

- `libbpf-rs` documented as the escape hatch in the LLD's open-questions section. The `bpf-libbpf` feature gate is reserved but not implemented in `v0.4`.
- `bpftrace` scripts under `RUNBOOK.md` for ad-hoc operator investigation; these do not load through `BpfSink` and do not produce OTel events. They are operator tooling, not gateway code.

## 7. What we explicitly reject

- **None / bolted-on (`§3.1`).** Would forfeit the third differentiation pillar. Revisit only if Riftgate strategically narrows scope and decides to compete on something other than integrated observability.
- **`bpftrace` (`§3.2`).** Adds a runtime dependency that breaks single-binary distribution and forces an IPC join path for every BPF event. Revisit only if Aya and libbpf-rs both become unmaintained.
- **`libbpf-rs` (`§3.3`).** Reintroduces a C toolchain into the build for BPF authoring. Revisit only if Aya cannot reach feature parity for a program we need and the hybrid path's two-toolchain cost is preferable to switching wholesale.
- **Hybrid (`§3.5`) as the default.** Premature complexity. Revisit only when triggered by a concrete blocker, not preemptively.

## 8. References

1. Linux kernel BPF documentation. <https://www.kernel.org/doc/html/latest/bpf/>
2. Andrii Nakryiko. *BPF CO-RE (Compile Once – Run Everywhere)*. 2020. <https://nakryiko.com/posts/bpf-portability-and-co-re/>
3. Andrii Nakryiko. *BTF deduplication and CO-RE*. LWN. <https://lwn.net/Articles/803258/>
4. Linux kernel source, `Documentation/bpf/` and `tools/bpf/`. <https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/Documentation/bpf>
5. `libbpf` — userspace BPF library. <https://github.com/libbpf/libbpf>
6. `libbpf-rs` — Rust bindings to libbpf. <https://github.com/libbpf/libbpf-rs>
7. Aya — pure-Rust BPF framework. <https://github.com/aya-rs/aya>; the Aya book at <https://aya-rs.dev/book/>.
8. Brendan Gregg. *BPF Performance Tools.* Addison-Wesley, 2019. ISBN 978-0136554820.
9. Brendan Gregg. *Flame Graphs.* <https://www.brendangregg.com/flamegraphs.html>
10. Facebook engineering. *BPF: A New Type of Software.* <https://engineering.fb.com/2018/12/03/data-infrastructure/bpf-a-new-type-of-software/>
11. Parca and Parca-agent. <https://github.com/parca-dev/parca>, <https://github.com/parca-dev/parca-agent>
12. Pyroscope. <https://github.com/grafana/pyroscope>
13. OpenTelemetry profiling signal proposal. <https://github.com/open-telemetry/oteps/blob/main/text/profiles/0212-profiling-vision.md>
14. Cilium project. *eBPF-based Networking, Security, and Observability.* <https://cilium.io/>
15. `kube-prometheus-stack` Helm chart. <https://github.com/prometheus-community/helm-charts/tree/main/charts/kube-prometheus-stack>
