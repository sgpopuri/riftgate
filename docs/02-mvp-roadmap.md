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
> - **Active milestone:** `v0.3` prep — perf-stabilization sweep, headline benchmarks, `PerShardScheduler` binary cutover, `io_uring` conformance harness lift.
> - **Recently shipped (`v0.2` the systems showpiece, tagged at this commit):** five new Options docs ([`009`](05-options/009-request-log.md), [`010`](05-options/010-routing-strategy.md), [`011`](05-options/011-circuit-breaker.md), [`012`](05-options/012-backpressure.md), [`023`](05-options/023-token-bucket-parameters.md)); five new accepted ADRs ([`0013`](06-adrs/0013-append-only-file-wal.md), [`0014`](06-adrs/0014-weighted-random-router.md), [`0016`](06-adrs/0016-three-state-circuit-breaker.md), [`0017`](06-adrs/0017-drop-newest-503-backpressure.md), [`0018`](06-adrs/0018-token-bucket-parameters.md)); [ADR `0009`](06-adrs/0009-rate-limiter-trait-in-proc-only.md) promoted to `accepted`; four LLDs refreshed. New code: `MpmcQueue` + `ShardedMpmcQueue` + `PerShardScheduler` in `crates/riftgate`; `TokenBucketLimiter` + `NoopLimiter` in `crates/riftgate-core`; `WeightedRandomRouter` + `CircuitBreakerArbiter` in `crates/riftgate-router`; `BackpressurePolicy` trait + `HighWaterPolicy` + `DenialReason` in `crates/riftgate-core`; new crate `crates/riftgate-replay` with `FileWal` implementing `riftgate_core::wal::WAL` per ADR `0013`; new crate `crates/riftgate-io-uring` (Linux + `--features io-uring`, empty-lib elsewhere). The full close-out narrative lives in the [`v0.2` retrospective](02c-v0.2-retrospective.md).
> - **In flight (`v0.3` Phase 1 — perf-sweep scoping):**
>   - Draft Options docs for the four bench targets (`scheduler`, `rate_limit`, `routing`, `wal`) under `benchmarks/v0.2-headline/`.
>   - Lift `crates/riftgate-io-epoll/tests/conformance.rs` into a shared `AsyncIO` conformance harness imported by both `riftgate-io-epoll` and `riftgate-io-uring`.
>   - `PerShardScheduler` binary cutover behind `--features per-core-scheduler`; compare against the tokio multi-thread baseline.
> - **Upcoming (`v0.3` Phase 2+):**
>   - Headline benchmarks vs LiteLLM and one published Rust gateway (per AGENTS.md §5 reproducibility contract).
>   - `BumpArena` pooling decision (per-shard vs shared) once measured under sustained load.
>   - `HierarchicalWheel` cutover decision per [ADR `0010`](06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md), gated on the timer microbenchmark crossing a documented threshold.
> - **Open questions (carried forward from `v0.2` close-out):**
>   - Domain availability for `riftgate.io` / `riftgate.dev` / `riftgate.com` — still not reserved. Now two milestones overdue.
>   - `BumpArena` pooling shape — per-shard or shared. v0.3 decides under measurement.
>   - `HierarchicalWheel` cutover threshold — schedule/cancel/tick ops/sec above which the binary-heap impl is no longer acceptable.
>   - Options `022` (priority / fairness scheduling) — v0.3 retro decides whether to commission the Options doc.
>   - eBPF library choice (Options `014`: bpftrace vs libbpf vs Aya) — explicitly v0.4 work; no v0.3 blocker.
> - **Recent learnings (the `v0.2` retrospective is the canonical surface):**
>   - Packed-`AtomicU64`-with-CAS-loop is now a documented v0.2 idiom — used by both `TokenBucketLimiter` (tokens mantissa + epoch ms) and `CircuitBreakerArbiter` (state tag + failure count). Document it in the next refresh of `lld-rate-limiter` and `lld-routing`.
>   - Empty-on-non-Linux crate compile works cleanly by combining `target.'cfg(target_os = "linux")'.dependencies` with `cfg(all(target_os = "linux", feature = "..."))` gates on every module. Reusable pattern for future Linux-only backends.
>   - Vose's alias method (1991) was the right choice for `WeightedRandomRouter` — O(1) per pick, O(N) one-time setup, fits the v0.2 32-backend ceiling comfortably.
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
