# 02. MVP-to-v1.0 Roadmap

> Five milestones, no calendar deadlines. Each milestone names the foundational systems principles it draws from, the specific Options docs and ADRs it depends on, and what "done" looks like.
>
> **Pacing philosophy:** milestones complete when they're done, not when a calendar says so. If the options docs, posts, and code flow quickly, milestones complete sooner. If life intervenes, they take longer. Rough estimate: ~12 months total at evenings-and-weekends pace with AI-assisted acceleration, but the milestone sequence is the commitment, not the timeline.

## Distribution (crates.io)

Through **`v0.4`**, Riftgate is distributed **only from the GitHub repository**: clone, `cargo build`, `cargo run`. We do **not** publish workspace crates to [crates.io](https://crates.io) and we do **not** treat `cargo install riftgate` as a milestone deliverable.

At the **`v1.0` retrospective**, maintainers decide whether to publish to crates.io (crate-name reservation, semver API stability, release automation). Until that decision, documentation and CI must not imply that registry install is supported or required.

Tag pushes may run release **build + test** workflows; they do not publish to crates.io.

## Currently shipping

> _**Project context (read every session):**_
>
> - **Active milestone:** `v0.3` Programmability and `v0.4` eBPF and the depths, run **in parallel**. **All design documentation is complete for both** (Phase A = `v0.3` docs, done; Phase C = `v0.4` docs, done). The previously separate implementation Phases B (`v0.3` code), D (`v0.4` code), and E (close-out) are now **folded into a single combined implementation-and-close-out phase**: there is no remaining doc/decision gate between them, so the work proceeds as one stream — land the `v0.3` and `v0.4` code, then close out both milestones together with retrospectives, decision tags, and benchmark/example/README sync.
> - **Recently shipped (combined implementation phase — first code landings):** `crates/riftgate-filter` (native `FilterChain` + `WasmFilter` scaffold), `KvAwareRouter` and `HedgedRouter` in `crates/riftgate-router`, `crates/riftgate-core::cancel` + SSE `Cancelled` state, the `GpuPressureSource` trait + `NoopGpuSource` in `crates/riftgate-core`, and the `crates/riftgate-obs-bpf` crate shell. `cargo test --workspace`, `cargo clippy`, and `cargo fmt --check` are green on the current tree.
> - **Recently shipped (`v0.4` Phase C — documentation):** three new Options docs ([`014` eBPF integration](05-options/014-ebpf-integration.md), [`027` token-level metrics](05-options/027-token-level-metrics.md), [`028` GPU pressure correlation](05-options/028-gpu-pressure-correlation.md)); three new accepted ADRs ([`0024` eBPF via Aya](06-adrs/0024-ebpf-via-aya.md), [`0025` token-level metrics](06-adrs/0025-token-level-metrics-probabilistic.md), [`0026` GPU pressure via DCGM exporter](06-adrs/0026-gpu-pressure-via-dcgm-exporter.md)); [`lld-observability.md`](04-design/lld-observability.md) refreshed to promote `BpfSink`, `TokenLevelAggregator`, `DcgmScrapeSource`, `NvmlSource`, `NoopGpuSource` from "v0.4 reserved" to "v0.4 designed"; glossary additions for CMS, HDR histogram, HLL, kprobe, tracepoint, MIG, NVML, reservoir sampling. The §5 invariant ("no code without a corresponding Options doc and ADR") is now satisfied for every `v0.4` surface.
> - **Recently shipped (`v0.3` Phase A — documentation, this same commit cycle):** four new Options docs ([`016`](05-options/016-extension-mechanism.md), [`019`](05-options/019-replay-eval.md), [`024`](05-options/024-stream-cancellation.md), [`025`](05-options/025-v03-routing-strategies.md)); five new accepted ADRs ([`0019`](06-adrs/0019-wasm-extension-mechanism.md), [`0020`](06-adrs/0020-stream-cancellation-cancellation-token.md), [`0021`](06-adrs/0021-external-replay-cli.md), [`0022`](06-adrs/0022-kv-aware-routing-prefix-trie.md), [`0023`](06-adrs/0023-hedged-requests-p99-triggered.md)); new LLD [`lld-filter-chain.md`](04-design/lld-filter-chain.md); refresh of [`lld-routing.md`](04-design/lld-routing.md).
> - **Recently shipped (`v0.2` the systems showpiece, tagged 2026-05-25):** see the [`v0.2` retrospective](02c-v0.2-retrospective.md) for the full close-out narrative.
> - **In flight — combined implementation + close-out (folded Phases B + D + E):** both milestones' design docs are frozen; this single phase lands all remaining `v0.3` and `v0.4` code, then closes out both milestones together. Progress to date is marked per item.
>   - **`v0.3` programmability code:**
>     - `crates/riftgate-filter` — **landed (partial):** native `FilterChain` executor and the `WasmFilter` / `WasmFilterConfig` public type surface compile today; the scaffold returns `FilterAction::Continue` for every call. **Remaining:** the production `WasmFilter` over the frozen `riftgate:filter/v1` Component Model ABI (wasmtime engine, WIT bindings, AOT precompile, instance pooling, host-function table).
>     - `KvAwareRouter<R>` and `HedgedRouter<R>` in `crates/riftgate-router` — **landed** as decorator-shaped impls. **Remaining:** wire them (and cancellation) through the `riftgate` binary end-to-end.
>     - `crates/riftgate-core::cancel` (`Cancellation` + `CancellationDriver` newtype around `tokio_util::sync::CancellationToken`; typed `CancelCause`) and the SSE framer `Cancelled` terminal state — **landed.**
>     - `riftgate-replay` `[[bin]]` target with `dump`, `replay`, `eval` subcommands — **remaining** (the `FileWal` exists; the CLI does not yet).
>     - Starter filter library under `examples/02-starter-filters/` — **remaining** (only `examples/01-basic-openai-proxy/` exists today).
>   - **`v0.4` observability code (scaffold only so far):**
>     - `crates/riftgate-obs-bpf` — **landed (shell):** crate manifest, `BACKEND_ENABLED` descriptor, and `BpfProgram` slot enum compile on every target. **Remaining:** the new module `crates/riftgate-obs/src/bpf/` plus the three Aya programs (CPU on/off-time profiling, syscall stalls, TCP retransmits per upstream) gated `cfg(all(target_os = "linux", feature = "bpf"))`, and the `BpfSink` impl of the existing `ObservabilitySink` trait.
>     - `crates/riftgate-obs/src/token_level/` housing `TokenLevelAggregator`: per-`(tenant, model, route)` HDR histograms, Vitter Algorithm R reservoir (default `K=100`, 60 s window), per-token WAL `TokenEvent` records, bounded dimension cap (default 10 000) with `(other, other, other)` fallback — **remaining.**
>     - `GpuPressureSource` trait — **landed** in `crates/riftgate-core/src/gpu.rs` together with the `NoopGpuSource` null impl. **Remaining:** `DcgmScrapeSource` (default, in `crates/riftgate-obs/src/gpu/dcgm.rs`) and `NvmlSource` (feature-gated `gpu-nvml`, in `crates/riftgate-obs/src/gpu/nvml.rs`).
>     - `RIFTGATE_ENABLE_BPF=1` startup gate documented in [`RUNBOOK.md`](../RUNBOOK.md); `CAP_BPF` required — **remaining.**
>   - **Close-out (after the code lands):** retrospectives for `v0.3` and `v0.4`, decision tags, benchmark gates, and example/README sync.
> - **Open questions (carried forward + v0.4-specific):**
>   - BPF-sourced byte-egress timestamps via `bpf-token-timestamps` feature for sub-millisecond inter-token-latency precision — `v0.4` ships userspace `Instant::now()` as the default; BPF path is an additive impl behind the same aggregator trait. (The eBPF *plan* is complete — [Options `014`](05-options/014-ebpf-integration.md), [ADR `0024`](06-adrs/0024-ebpf-via-aya.md), the [`lld-observability.md`](04-design/lld-observability.md) refresh, and the `crates/riftgate-obs-bpf` scaffold. Only the Aya implementation remains, and it is major work — Linux host, `bpfel-unknown-none` target, clang/LLVM, kernel verifier — that stays in the combined implementation phase.)
>   - Multi-vendor GPU telemetry beyond Prometheus scrape (AMD ROCm native, Habana, Inferentia) — `v0.4` works via DCGM-exporter-compatible scrape against AMD ROCm SMI exporter; native vendor-specific `GpuPressureSource` impls become clean `v1.0+` additions behind the same trait.
>   - Sidecar option ([Options `028` §3.3](05-options/028-gpu-pressure-correlation.md)) deferred to `v1.0+` as the long-term multi-vendor strategy.
>   - CMS heavy-hitters extension ([Options `027` §3.5](05-options/027-token-level-metrics.md)) deferred to `v1.0` unless operator demand surfaces at `v0.4` close-out.
> - **Recently decided (`v0.3 + v0.4` session, this commit):**
>   - **Domain reservation — dropped.** `riftgate.io` / `.dev` / `.com` reservation is no longer tracked as an open question; revisit only if and when a public launch needs it.
>   - **`BumpArena` pooling shape — per-shard (decided).** [ADR `0027`](06-adrs/0027-per-shard-bump-arena-pool.md): a per-shard free-list with no cross-core synchronization on the recycle path, bounded by `arena_pool_max` (default 32) and `arena_pool_retain_cap_bytes` (default 64 KiB). A shared/global pool is rejected; the accepted cost is slightly higher idle RSS.
>   - **`HierarchicalWheel` cutover — benchmark-gated (decided).** [ADR `0028`](06-adrs/0028-timer-cutover-benchmark-gated.md) supersedes ADR 0010's "v0.2 land / v0.3 default" schedule: `BinaryHeapTimers` stays the default indefinitely; `HierarchicalWheel` is built only when [`benches/timers.rs`](../crates/riftgate-core/benches/timers.rs) shows the heap's per-tick p99 exceeding the 100 µs budget at ≥100k live timers, and its promotion to default is then pre-authorized (no further ADR needed to flip the default once the gate is met).
>   - **MCP capability broker (`v0.5`) — stays queued, not pulled forward.** MCP is an external, still-evolving protocol with inherent external requirements (spec fidelity, per-tenant allowlist schema, HMAC attestation); [ADR `0015`](06-adrs/0015-mcp-extension-plane-broker.md) remains `proposed`, targeted at the open of `v0.5` after `v0.4` lands. Implementing it now would accept a proposed ADR ahead of its milestone and violate the milestone sequence.
>   - **`v1.0` — stays queued, not session-scoped.** The K8s operator, CRDs, replay-framework maturity, and production hardening are multi-milestone work, not a single session's; the crates.io distribution decision remains reserved for the `v1.0` retrospective per [Distribution](#distribution-cratesio).
> - **Recent learnings (`v0.4` Phase C close-out):**
>   - Match-substrate-to-question is the load-bearing discipline for `v0.4` observability: HDR for aggregate latency, reservoir for forensic sampling, WAL for per-request replay. Three substrates each pulling their weight beats one substrate over-stretched.
>   - Reservoir sampling preserves rare slow tails; rate sampling discards them. The latter is fine for the request-root span (where the population is dense and uniform), wrong for token sub-spans (where slow streams *are* the population of interest).
>   - DCGM-scrape-versus-NVML-FFI is fundamentally a topology question, not a performance question. Riftgate-on-LB topology wants scrape; Riftgate-co-located-with-GPU topology wants FFI. The trait surface absorbs the difference without `BackendSignals` schema change.
>   - "Integrated, not bolted-on" is a structural claim that depends on the BPF runtime being in-process. Aya delivers; bolted-on alternatives (`bpftrace`, sidecars) would forfeit the differentiation pillar even though they're operationally simpler.
>
> _Update this section in every session that ships meaningful progress. This is the live project-context surface for the context harness defined in [`AGENTS.md`](../AGENTS.md)._

---

## v0.0 — Public design phase

**Pure markdown. Zero Rust.** This milestone is deliberately documentation-only so the build-in-public discipline is established before any code lands.

### Deliverables

- [`README.md`](../README.md), [`AGENTS.md`](../AGENTS.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), `LICENSE`
- [`docs/00-vision.md`](00-vision.md) — north star, non-goals, differentiation
- [`docs/01-requirements/`](01-requirements/) — functional, non-functional, personas
- [`docs/02-mvp-roadmap.md`](02-mvp-roadmap.md) (this file)
- [`docs/02a-v0.0-retrospective.md`](02a-v0.0-retrospective.md) — one-time milestone retrospective anchored to the `v0.0` tag
- [`docs/03-architecture/hld.md`](03-architecture/hld.md) — high-level design across the four planes
- [`docs/03-architecture/{data,control,extension,observability}-plane.md`](03-architecture/) — plane-level narratives (data, control, extension, observability)
- [`docs/04-design/lld-*.md`](04-design/) — 10 LLD skeletons (eight core: io-runtime, scheduling, parsing, storage, allocator, timers, routing, observability; plus the two from the 2026-05 research pass: `lld-rate-limiter`, `lld-mcp-capability`)
- [`docs/05-options/`](05-options/) — nine Options docs:
  - `001-io-model.md` (epoll vs kqueue vs io_uring vs DPDK vs AF_XDP)
  - `002-async-runtime.md` (tokio vs glommio vs monoio vs custom)
  - `003-concurrency-model.md` (shared vs per-shard vs work-stealing)
  - `004-request-queue.md` (mutex vs MPMC vs SPSC vs sharded)
  - `005-allocator.md` (system vs jemalloc vs mimalloc vs arena)
  - `007-protocol-parser.md` (hyper vs combinators vs hand-rolled FSM)
  - `008-stream-framing.md` (SSE vs NDJSON vs gRPC-stream)
  - `021-rate-limiting.md` (research-pass: fixed-window vs sliding-window vs token bucket vs leaky bucket vs GCRA vs distributed variants)
  - `026-mcp-orchestration.md` (research-pass: passthrough vs inspector vs broker vs mediator)
- [`docs/06-adrs/`](06-adrs/) — eight accepted ADRs (`0001`–`0008`) plus two ADRs in `proposed` status (`0009` rate-limiter trait + in-proc-only impl, targeting the open of `v0.2`; `0015` MCP-as-extension-plane-broker, targeting the open of `v0.5`)
- [`docs/08-glossary.md`](08-glossary.md) — public glossary of Riftgate terms

### Foundational principles drawn from
Unix I/O multiplexing (`epoll`/`kqueue`/IOCP), reactor pattern and event loops, `io_uring` (shared SQ/CQ rings, batched submission), DPDK / kernel-bypass networking, work-stealing schedulers, system-design patterns (bulkhead, sidecar/ambassador, circuit breaker), FSM-based protocol parsing, memory allocators (`jemalloc`/`mimalloc`/arenas), hierarchical / hashed timing wheels.

### Goal metric
Prove the documentation-first methodology works publicly. **One GitHub watcher = success at this stage.** A senior systems engineer cites an Options doc on social media = early success.

---

## v0.1 — Walking skeleton

A single Rust binary that proxies real OpenAI-format traffic to one backend with SSE streaming. The minimum useful gateway. **Shipped 2026-05-10; close-out narrative in [`02b-v0.1-retrospective.md`](02b-v0.1-retrospective.md).**

### Foundational principles
- Unix I/O multiplexing (`epoll` basics), reactor pattern, lock-free MPMC queues for the request queue, ring buffers for SSE response framing, FSM-based protocol parsing, per-request bump-arena allocation, hashed timer wheels for per-request deadlines.

### Functional requirements covered
FR-001 through FR-008 (see [`01-requirements/functional.md`](01-requirements/functional.md))

### Subsystems landing
- `crates/riftgate-core` — trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `Filter`, `Router`, `ObservabilitySink`) plus the impl-only-deferred traits (`RateLimiter`, `WAL`, `CapabilityBroker`); in-core impls for `SystemAllocator`, `BumpArena`, `BinaryHeapTimers`, `DeterministicTimers`, `IdentityFilter`, `LoggingFilter`, `InMemorySink`
- `crates/riftgate-io-epoll` — first `AsyncIO` impl (mio under the hood; `epoll(7)` on Linux, `kqueue(2)` on macOS/BSD)
- `crates/riftgate-parser` — FSM-based HTTP/1.1 + SSE parser (`Http1Parser` + `SseFramer`)
- `crates/riftgate-config` — TOML schema + env-override loader + fail-loudly validation + `Secret<T>` redaction
- `crates/riftgate-router` — `RoundRobinRouter` (atomic-cursor) and `ConstantRouter` (test impl)
- `crates/riftgate-obs` — bounded MPSC bus with drop-on-full + `OtelSink` (OTLP/gRPC) + `JsonStdoutSink` + `MultiSink` fan-out + canonical span-name registry
- `crates/riftgate` — main binary: tokio multi-thread runtime, accept loop, hyper-rustls upstream client, SSE forwarding, `/health` + `/ready`, SIGTERM drain
- [`examples/01-basic-openai-proxy`](../examples/01-basic-openai-proxy/) — self-contained dev loop with OTel collector docker-compose

### Options docs and ADRs depended on
Options/ADRs 001-008 from `v0.0`, plus:
- `006-timer-subsystem.md` and ADR
- `015-config-model.md` and ADR (initial: static TOML)
- `013-observability-sink.md` and ADR (initial: OTel exporter)

### Goal metric
Someone outside the project can clone the repo, `cargo build --release -p riftgate`, and proxy real OpenAI-format traffic (see [`examples/01-basic-openai-proxy`](../examples/01-basic-openai-proxy/)). ~100 GitHub stars. A second person opens a non-trivial issue. Registry distribution is explicitly out of scope until the v1.0 crates.io decision ([Distribution](#distribution-cratesio)).

### Retrospective
- [`02b-v0.1-retrospective.md`](02b-v0.1-retrospective.md) — what shipped, what went well, what we missed and fixed in close-out, what's open at `v0.1` close, process notes for `v0.2` and beyond.

---

## v0.2 — The systems showpiece

Honest performance on Linux, multi-backend routing, durable request log, circuit breakers, work-stealing scheduler.

### Foundational principles
- `io_uring` (shared-memory ring submission), lock-free MPMC queues, work-stealing schedulers (Cilk-5 / Chase-Lev), backpressure as policy, LSM-tree concepts for the request log, write-ahead-logging semantics (ARIES), circuit-breaker resilience pattern (Nygard, *Release It*).

### Functional requirements covered
FR-101 through FR-108

### Subsystems landing
- `crates/riftgate-io-uring` — second `AsyncIO` impl behind a feature flag
- Work-stealing scheduler in `riftgate-core`
- Multi-backend routing (round-robin, weighted-random)
- Circuit breaker per backend
- Adaptive backpressure (queue-depth high-water mark)
- `crates/riftgate-replay` — append-only WAL, basic replay CLI
- In-proc token-bucket `RateLimiter` impl in `riftgate-core` — trait-shaped to accept a future distributed impl without breakage

### Options docs and ADRs added
- `009-request-log.md` (append file vs embedded-rocks vs custom WAL) + ADR
- `010-routing-strategy.md` (RR vs weighted vs KV-aware vs hedged) + ADR (the first three; KV-aware deferred to `v0.3`)
- `011-circuit-breaker.md` (3-state vs sliding-window vs adaptive) + ADR
- `012-backpressure.md` (drop-newest vs drop-oldest vs block vs 503) + ADR
- [`021-rate-limiting.md`](05-options/021-rate-limiting.md) (fixed-window vs sliding-window vs token bucket vs leaky bucket vs GCRA vs distributed variants) + ADR `0009`

### `v0.2` retro gate
At close, decide whether Options `022` (priority / fairness scheduling) is worth pursuing in `v0.3`. The `FR-206` requirement is gated on this decision.

### Honest benchmark
Riftgate `v0.2` vs LiteLLM (we expect a clear win), vs a published TensorZero claim (we expect to lose; that's fine and we'll say so), reproducible from `benchmarks/`.

### Goal metric
~500 GitHub stars. Cited by 1-2 known systems voices. First external contributor lands a non-trivial PR.

---

## v0.3 — Programmability

WASM filter chain, plugin-based routing strategies, hedged requests with stream cancellation.

### Foundational principles
- Extension models and sidecar/ambassador deployment patterns (Microsoft *Cloud Design Patterns*, Hohpe *Enterprise Integration Patterns*); per-stream FSM-based cancellation (table-driven state machines that handle the cancel transition cleanly).

### Functional requirements covered
FR-201 through FR-205, and (gated on `v0.2` retro) FR-206

### Subsystems landing
- `crates/riftgate-filter` — WASM filter chain (wasmtime backend)
- Starter filter library: PII redactor, prompt template substitution, output schema validator, cost guard, token-budget guard
- Routing strategies as plugins: KV-cache-aware (integrating with `vllm-router`'s LMCache or a built-in prefix trie), hedged requests
- Stream cancellation primitives in `riftgate-core`
- (Optional, gated on `v0.2` retro) Priority-aware scheduling in the request queue, built on a binary or d-ary heap (CLRS ch. 6).

### Options docs and ADRs added
- `010-routing-strategy.md` extended (KV-aware, hedged) + new ADRs
- `016-extension-mechanism.md` (none vs Lua vs WASM vs native trait) + ADR
- `019-replay-eval.md` (none vs embedded vs external CLI) + ADR

### Goal metric
~1500 GitHub stars. First external contributor lands a custom filter or routing strategy. Conference talk submission accepted.

---

## v0.4 — eBPF and the depths

Gateway-internal continuous profiling and backend GPU pressure observability via Aya-based BPF programs.

### Foundational principles
- eBPF (verifier, JIT, maps, kprobes / tracepoints / XDP / TC / LSM); streaming sketches for token-level metrics (Count–Min Sketch, HyperLogLog, reservoir sampling).

### Functional requirements covered
FR-301, FR-302, FR-303

### Subsystems landing
- `crates/riftgate-obs` extended with Aya BPF programs
- Continuous gateway profiling (CPU on/off, syscall stalls, NUMA misses)
- Backend GPU pressure correlation (DCGM/NVML)
- Token-level SLO metrics emitted (TTFT, inter-token latency, jitter)

### Options docs and ADRs added
- `014-ebpf-integration.md` (bpftrace vs libbpf vs Aya vs none) + ADR
- `013-observability-sink.md` extended for token-level metrics

### Goal metric
~3000 GitHub stars. The eBPF integration becomes the moment external observers say "this project is serious."

---

## v0.5 — Agentic capability plane

First-class [Model Context Protocol](https://modelcontextprotocol.io/) support. The gateway learns to understand MCP — which tools and resources the model is trying to reach — and becomes the capability broker that authorizes (or denies) each invocation on behalf of the tenant. Not a new plane; a first-class feature inside the extension plane, alongside filters and routing strategies.

The agentic-era posture: the gateway is no longer just a byte proxy. It is the capability ledger — who-called-what-on-whose-behalf, durably recorded, queryable after the fact.

### Foundational principles
- Ambassador pattern and capability-based security (KeyKOS / EROS / seL4 lineage; Mark Miller, *Robust Composition*); resilience patterns (Nygard, *Release It*).
- Write-ahead-logging semantics (ARIES) reused for the capability audit log.
- Allowlist data structures: prefix trie / radix tree (Knuth TAOCP §6.3), interval tree for time-bounded grants (CLRS ch. 14), bit-set allowlists.
- Topological sort over DAGs (Kahn 1962; CLRS ch. 22) — optional, if tool-dependency graphs become relevant (for example "tool A must run before tool B").

### Functional requirements covered
FR-501 through FR-504

### Subsystems landing
- `crates/riftgate-mcp` — MCP request parser and capability broker implementing the new `CapabilityBroker` trait
- Per-tenant tool/resource allowlist, loaded from config (and, at `v1.0`, from CRDs)
- Audit pipeline: every MCP `tools/call` decision is written to the WAL and surfaced to OTel as a structured log
- Attestation headers (`riftgate-mcp-caller`, `riftgate-mcp-tool`, `riftgate-mcp-decision`) for downstream policy engines

### Options docs and ADRs added
- [`026-mcp-orchestration.md`](05-options/026-mcp-orchestration.md) (gateway-as-passthrough vs inspector vs broker vs mediator) + ADR `0015`

### Why this milestone exists

The 2026 research pass confirmed that Anthropic's MCP and the broader tool-use surface are the agentic-era edge: the gateway is no longer just routing chat completions, it is routing *capabilities*. Riftgate's extension plane is the right home for this — a dedicated trait (`CapabilityBroker`) with multiple impls, auditing through the existing WAL, and zero additional planes. Declining to address MCP before `v1.0` would leave an obvious hole in the agentic-era positioning; inventing a fourth pillar would violate the three-pillar discipline. This milestone is the compromise: first-class feature, not a new plane. Scope analysis and rejected alternatives live in [Options `026`](05-options/026-mcp-orchestration.md).

### Goal metric
~2500–3500 GitHub stars (after `v0.4`'s eBPF moment). First external contributor lands an MCP-related PR — a new allowlist policy, a new attestation header scheme, or an MCP-specific filter. One external agentic-workflow team pilots Riftgate as their capability ledger.

---

## v1.0 — Production-ready and mesh-native

K8s operator, CRDs, sidecar deployment, comprehensive test suite, replay framework.

### Foundational principles
At `v1.0` every foundational principle introduced in earlier milestones — Unix I/O multiplexing, `io_uring`, reactor and work-stealing schedulers, lock-free MPMC queues, ring-buffer / zero-copy I/O paths, FSM-based parsing, per-request arena allocation, hierarchical timer wheels, write-ahead logging, LSM-tree storage concepts, system-design and resilience patterns, eBPF observability, capability-based security — is visible somewhere in the codebase or docs.

### Functional requirements covered
FR-401 through FR-405; all cross-cutting requirements (FR-X01 through FR-X05) are at green.

### Subsystems landing
- Kubernetes operator with CRDs (`Riftgate`, `RiftgateBackend`, `RiftgateRoute`)
- Sidecar deployment manifest verified against Istio and Linkerd
- `riftgate-replay` CLI mature: replay any logged request against a different config
- Property-based tests on the parser, fuzz tests on the wire format
- `UPGRADING.md` for every prior minor release

### Options docs and ADRs added
- `017-multitenancy.md` + ADR
- `018-deployment.md` + ADR
- `015-config-model.md` extended for CRD-driven config

### Goal metric
- 3+ design partners willing to pilot Riftgate in production.

### Distribution decision (v1.0)
At the v1.0 retrospective, decide whether to publish workspace crates to crates.io (`cargo install riftgate` and dependency-order `cargo publish`). If yes, add an Options doc + ADR, reserve crate names, and wire release automation. If no, document build-from-source / container images as the supported distribution path going forward.

---

## What happens after v1.0

Decision point. Three paths:

1. **Continue as OSS flagship.** Riftgate becomes a long-running canonical artifact; effort tapers to occasional major releases.
2. **Company exploration.** With 3+ design partners, explore a managed-Riftgate or eBPF-observability-as-a-product wedge. Pre-seed only after clear pull.
3. **Hand off.** If a clear maintainer-successor emerges, transition stewardship.

These paths are not mutually exclusive; the explicit decision happens at the v1.0 retrospective.
