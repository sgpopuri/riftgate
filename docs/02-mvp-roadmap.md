# 02. MVP-to-v1.0 Roadmap

> Five milestones, no calendar deadlines. Each milestone names the chapters of the source-systems curriculum it draws from, the specific Options docs and ADRs it depends on, and what "done" looks like.
>
> **Pacing philosophy:** milestones complete when they're done, not when a calendar says so. If the options docs, posts, and code flow quickly, milestones complete sooner. If life intervenes, they take longer. Rough estimate: ~12 months total at evenings-and-weekends pace with AI-assisted acceleration, but the milestone sequence is the commitment, not the timeline.

## Currently shipping

> _**Project context (read every session):**_
>
> - **Active milestone:** `v0.1` — walking skeleton (first Rust binary). Awaiting kickoff; no Rust code yet.
> - **Recently shipped (`v0.0` close-out, 2026-05-03):** Options docs `001`-`005`, `007`, `008` accepted; ADRs `0001`-`0008` accepted. Vision, requirements, HLD, all four plane narratives, ten LLD skeletons, glossary, and the two research-pass docs (Options `021` rate-limiting + LLD; Options `026` MCP orchestration + LLD; ADRs `0009` and `0015` reserved-proposed for `v0.2` and `v0.5` implementations). Repo published at https://github.com/sgpopuri/riftgate; `v0.0` tagged.
> - **In flight:** _(none yet; `v0.1` starts with three remaining prerequisite Options docs — `006-timer-subsystem`, `013-observability-sink`, `015-config-model` — and the first `crates/riftgate-core` trait surface scaffolding derived from the LLDs)_
> - **Open questions:**
>   - Domain availability check for `riftgate.io` / `riftgate.dev` / `riftgate.com` — still not done. Recommended pre-`v0.1`-publication decision item; the project ships fine without a custom domain, but if one is intended it should be reserved before broader announcement.
>   - eBPF library choice (Options `014`: bpftrace vs libbpf vs Aya) — sequencing TBD; explicitly `v0.4` work, no `v0.1` blocker.
>   - At `v0.2` retro: whether to pursue Options `022` (priority / fairness scheduling).
>   - At `v0.4` retro: whether to pursue Options `029` (async telemetry pipeline).
> - **Recent learnings:** `v0.0` closed cleanly with the remaining six Options docs and ADRs landed in one close-out batch. The naming refinement from `PerCoreScheduler` (LLD's outline-stage term) to `PerShardScheduler` ([ADR 0004](06-adrs/0004-per-shard-default-stealing-opt-in.md) final term) was deliberate: the `v0.1` reality is M logical shards on N Tokio threads with no pinning; physical thread-per-core ([ADR 0003](06-adrs/0003-tokio-multithread-default.md) revisit at `v0.2` retro) is a deployment-shape question, not a scheduler-trait question. Competitive landscape research (Apr-May 2026) confirmed TensorZero, Helicone, Envoy AI Gateway, vllm-router/kvfleet/llm-d-kv-cache are serious incumbents. A follow-up 2026-05 research pass added two committed scope items: an in-proc-only rate limiter (behind a trait that can accept a distributed impl later) and first-class MCP capability brokering as a new `v0.5` milestone. Vision `§4` and `§8` (known extension points / deferred hooks) record what we deliberately declined (multi-provider adapters, semantic-cache reference impl, distributed state substrate).
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
- [`docs/03-architecture/hld.md`](03-architecture/hld.md) — high-level design with three planes
- [`docs/03-architecture/{data,control,extension,observability}-plane.md`](03-architecture/) — plane-level narratives
- [`docs/04-design/lld-*.md`](04-design/) — 8 LLD skeletons
- [`docs/05-options/`](05-options/) — first 5-7 Options docs:
  - `001-io-model.md` (epoll vs kqueue vs io_uring vs DPDK vs AF_XDP)
  - `002-async-runtime.md` (tokio vs glommio vs monoio vs custom)
  - `003-concurrency-model.md` (shared vs per-core vs work-stealing)
  - `004-request-queue.md` (mutex vs MPMC vs SPSC vs sharded)
  - `005-allocator.md` (system vs jemalloc vs mimalloc vs arena)
  - `007-protocol-parser.md` (hyper vs combinators vs hand-rolled FSM)
  - `008-stream-framing.md` (SSE vs NDJSON vs gRPC-stream)
- [`docs/06-adrs/`](06-adrs/) — first 5-7 ADRs accepting the above options

### Source chapters drawn from
Ch1 (IO models & multiplexing), Ch2 (event loops & reactor), Ch3 (io_uring), Ch6 (DPDK), Ch7 (work stealing), Ch12 (system design patterns), Ch13 (FSM-based parsing), Ch14 (allocators), Ch15 (timer wheels)

### Goal metric
Prove the documentation-first methodology works publicly. **One GitHub watcher = success at this stage.** A senior systems engineer cites an Options doc on social media = early success.

---

## v0.1 — Walking skeleton

A single Rust binary that proxies real OpenAI-format traffic to one backend with SSE streaming. The minimum useful gateway.

### Source-systems chapters
- Ch1 (epoll basics), Ch2 (reactor), Ch4 (lock-free MPMC for queue), Ch5 (ring buffers for SSE), Ch13 (FSM parser), Ch14 (per-request arena), Ch15 (timer wheel for deadlines)

### Functional requirements covered
FR-001 through FR-008 (see [`01-requirements/functional.md`](01-requirements/functional.md))

### Subsystems landing
- `crates/riftgate-core` — trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `WAL`, `Filter`, `Router`)
- `crates/riftgate-io-epoll` — first `AsyncIO` impl
- `crates/riftgate-parser` — FSM-based HTTP/SSE parser
- `crates/riftgate-router` — round-robin only
- `crates/riftgate-obs` — OTel exporter
- `crates/riftgate` — main binary

### Options docs and ADRs depended on
Options/ADRs 001-008 from `v0.0`, plus:
- `006-timer-subsystem.md` and ADR
- `015-config-model.md` and ADR (initial: static TOML)
- `013-observability-sink.md` and ADR (initial: OTel exporter)

### Goal metric
Someone outside the project can `cargo install riftgate` (or build from source) and proxy real OpenAI-format traffic. ~100 GitHub stars. A second person opens a non-trivial issue.

---

## v0.2 — The systems showpiece

Honest performance on Linux, multi-backend routing, durable request log, circuit breakers, work-stealing scheduler.

### Source-systems chapters
- Ch3 (io_uring), Ch4 (lock-free MPMC), Ch7 (work-stealing), Ch8 (backpressure as policy), Ch9 (LSM concepts for the request log), Ch11 (WAL semantics), Ch12 (circuit breaker)

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

### Source-systems chapters
- Ch12 (extension models, sidecar/ambassador patterns), Ch13 (per-stream FSM for cancellation)

### Functional requirements covered
FR-201 through FR-205, and (gated on `v0.2` retro) FR-206

### Subsystems landing
- `crates/riftgate-filter` — WASM filter chain (wasmtime backend)
- Starter filter library: PII redactor, prompt template substitution, output schema validator, cost guard, token-budget guard
- Routing strategies as plugins: KV-cache-aware (integrating with `vllm-router`'s LMCache or a built-in prefix trie), hedged requests
- Stream cancellation primitives in `riftgate-core`
- (Optional, gated on `v0.2` retro) Priority-aware scheduling in the request queue, citing `trees/ch04_heaps_priority_queues.md`

### Options docs and ADRs added
- `010-routing-strategy.md` extended (KV-aware, hedged) + new ADRs
- `016-extension-mechanism.md` (none vs Lua vs WASM vs native trait) + ADR
- `019-replay-eval.md` (none vs embedded vs external CLI) + ADR

### Goal metric
~1500 GitHub stars. First external contributor lands a custom filter or routing strategy. Conference talk submission accepted.

---

## v0.4 — eBPF and the depths

Gateway-internal continuous profiling and backend GPU pressure observability via Aya-based BPF programs.

### Source-systems chapters
- Ch16 (eBPF), Ch10 (sketches for token-level metrics)

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

### Source-systems chapters
- Ch12 (ambassador pattern, capability-based security; resilience patterns)
- Ch11 (WAL semantics reused for the capability audit log)
- `advanced/ch08_design_data_structures.md` (allowlist data structures: prefix trie, interval tree for time-bounded grants, bit-set allowlists)
- `graphs/ch03_topological_sort_dags.md` (optional; if tool-dependency graphs become relevant, e.g. "tool A must run before tool B")

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

### Source-systems chapters
All 16 are now visible somewhere in the codebase or docs.

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

---

## What happens after v1.0

Decision point. Three paths:

1. **Continue as OSS flagship.** Riftgate becomes a long-running canonical artifact; effort tapers to occasional major releases.
2. **Company exploration.** With 3+ design partners, explore a managed-Riftgate or eBPF-observability-as-a-product wedge. Pre-seed only after clear pull.
3. **Hand off.** If a clear maintainer-successor emerges, transition stewardship.

These paths are not mutually exclusive; the explicit decision happens at the v1.0 retrospective.
