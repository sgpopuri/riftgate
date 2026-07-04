# 02. MVP-to-v1.0 Roadmap

> Five milestones, no calendar deadlines. Each milestone names the foundational systems principles it draws from, the specific Options docs and ADRs it depends on, and what "done" looks like.
>
> **Pacing philosophy:** milestones complete when they're done, not when a calendar says so. If the options docs, posts, and code flow quickly, milestones complete sooner. If life intervenes, they take longer. Rough estimate: ~12 months total at evenings-and-weekends pace with AI-assisted acceleration, but the milestone sequence is the commitment, not the timeline.

## Distribution (crates.io)

Through **`v1.0`**, Riftgate is distributed **only from the GitHub repository**: clone, `cargo build`, `cargo run`. **The v1.0 retrospective decision is: no crates.io publish at this time.** Distribution remains build-from-source. Revisit when production adoption warrants the registry-maintenance overhead (semver stability guarantees, crate-name reservation, release automation). Do not add `cargo publish` or `cargo install` documentation until this decision is revisited.

## Currently shipping

> _**Project context (read every session):**_
>
> - **Active milestone:** `v1.0` — production-ready and mesh-native — **landed and closed-out**. All v1.0 functional requirements satisfied: K8s operator (`crates/riftgate-operator`) with CRDs + Helm chart (ADR 0030); property-based parser tests + WAL fuzz target (FR-404); `TenantResolver` + `ApiKeyTenantResolver` (ADR 0029); CRD-driven API key Secret reading; smoke test stub. `UPGRADING.md` written. **Distribution decision (v1.0 retro): no crates.io publish at this time; distribution remains build-from-source (`cargo build -p riftgate`). Revisit when production demand justifies the registry-maintenance overhead.** Active focus: post-v1.0 operations and community building.
> - **Recently shipped (`v0.3` close-out, this session):** all `v0.3` programmability code landed and validated: `crates/riftgate-filter` with native `FilterChain` executor and production wasmtime-backed `WasmFilter` runtime (frozen `riftgate:filter/v1` Component Model ABI, wasmtime 31, pooling allocator, load-time AOT precompile validation), `KvAwareRouter` and `HedgedRouter` in `crates/riftgate-router`, stream cancellation primitives in `riftgate-core`, `riftgate-replay` CLI (`dump`, `replay`, `eval` subcommands), per-core-scheduler feature option, and startup filters example boilerplate. Full workspace verification passed: 74 tests green, clippy/fmt/rustdoc gates clean. See [`v0.3` retrospective](02d-v0.3-retrospective.md) for narrative and lessons learned.
> - **In flight (`v0.5` planning):** MCP capability-broker implementation sequencing against [ADR `0015`](06-adrs/0015-mcp-extension-plane-broker.md), with `v0.4` eBPF source-build environment hardening explicitly deferred. For `v0.4`, the accepted operational path is strict EM_BPF artifact staging via [`scripts/build-bpf-objects`](../scripts/build-bpf-objects) `--mode real --from ...` and verifier harness validation.
> - **Recently shipped (`v0.4` close-out, this session):** completed v0.4 milestone close-out documentation and status transition, including retrospective [`02e-v0.4-retrospective.md`](02e-v0.4-retrospective.md). The local `--build-from-source` path remains environment-gated (nightly + build-std=core), but is explicitly classified as non-blocking for `v0.4`; external EM_BPF staging with strict verification is the accepted completion route.
> - **Recently shipped (`v0.4` Phase A — observability substrate, this session):** `TokenLevelAggregator` landed in `crates/riftgate-obs` with per-`(tenant, model, route)` HDR histograms plus Vitter reservoir sampling; `DcgmScrapeSource` landed as the default concrete `GpuPressureSource`; `NvmlSource` landed behind the Linux-only `gpu-nvml` feature using `nvml-wrapper`; and `BpfSink` landed as the feature-gated, runtime-gated scaffold for Aya program loading. Validation passed: `cargo fmt --all --check`, `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `RUSTDOCFLAGS='--deny warnings' cargo doc --workspace --all-features --no-deps` via `scripts/cargow`.
> - **Recently shipped (`v0.4` Phase A — BPF runtime wiring follow-on, this session):** `crates/riftgate` now wires `BpfSink::from_env()` into the binary startup `MultiSink` fanout with explicit runtime-state logs (`CompiledOut`, `DisabledByEnv`, `Loaded { programs }`), and `crates/riftgate` now forwards `bpf` and `gpu-nvml` feature flags to `riftgate-obs` so Linux operators can build BPF-capable binaries deliberately. Operator docs were updated in [`RUNBOOK.md`](../RUNBOOK.md) and [`crates/riftgate/README.md`](../crates/riftgate/README.md). Validation passed in both modes: `./scripts/cargow test -p riftgate --lib`, `./scripts/cargow test -p riftgate --features bpf --lib`, and matching `clippy -- -D warnings` runs.
> - **Recently shipped (`v0.4` Phase A — verifier harness scaffold, this session):** added Linux + `bpf` feature-gated verifier-harness scaffold [`crates/riftgate-obs/tests/bpf_verifier.rs`](../crates/riftgate-obs/tests/bpf_verifier.rs) and documented it in [`crates/riftgate-obs/README.md`](../crates/riftgate-obs/README.md). The test is intentionally `#[ignore]` until Aya program objects land; today it locks wiring invariants (`BACKEND_ENABLED`, stable program slot names). Validation passed: `./scripts/cargow test -p riftgate-obs --features bpf` and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — Aya loader path, this session):** `crates/riftgate-obs` now includes Aya userspace runtime dependency behind the `bpf` feature and promotes [`crates/riftgate-obs/tests/bpf_verifier.rs`](../crates/riftgate-obs/tests/bpf_verifier.rs) from placeholder to a real loader-path assertion (`aya::Ebpf::load` rejects invalid object bytes) plus stable-slot checks. This is intentionally privilege-free (no probe attach, no `CAP_BPF` requirement) so Linux CI can exercise Aya wiring before BPF object artifacts land. Validation passed: `./scripts/cargow test -p riftgate-obs --features bpf` and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — staged object contract, this session):** `crates/riftgate-obs-bpf` now defines canonical staged object contract paths via `STAGED_OBJECT_DIR` and `BpfProgram::staged_object_relpath()` (`crates/riftgate-obs-bpf/obj/<slot>.bpf.o`). [`crates/riftgate-obs/tests/bpf_verifier.rs`](../crates/riftgate-obs/tests/bpf_verifier.rs) now asserts that contract and adds an ignored `aya_loads_staged_object_when_present` path that activates when staged object artifacts exist. Validation passed: `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`, `./scripts/cargow test -p riftgate-obs-bpf --features bpf`, and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — staged artifact workflow, this session):** added executable helper [`scripts/stage-bpf-objects`](../scripts/stage-bpf-objects) to stage Aya object artifacts into canonical `crates/riftgate-obs-bpf/obj/<slot>.bpf.o` paths with optional `--verify-elf` header checks, plus docs in [`RUNBOOK.md`](../RUNBOOK.md) and [`crates/riftgate-obs-bpf/README.md`](../crates/riftgate-obs-bpf/README.md). Local artifact policy is explicit: `obj/.gitkeep` tracked, `*.bpf.o` ignored in [`.gitignore`](../.gitignore). Validation passed: `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier` and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — program-source build contract, this session):** added in-repo Aya program-source workspace crate `crates/riftgate-obs-bpf/programs` and executable helper [`scripts/build-bpf-objects`](../scripts/build-bpf-objects) that deterministically emits canonical staged artifact names and writes `crates/riftgate-obs-bpf/obj/ARTIFACT_FORMAT=host-placeholder` for current placeholder mode. [`crates/riftgate-obs/tests/bpf_verifier.rs`](../crates/riftgate-obs/tests/bpf_verifier.rs) now skips the ignored staged-load assertion when this marker is present, while keeping strict Aya load checks for real staged objects. Validation passed: `./scripts/cargow check -p riftgate-obs-bpf-programs`, `./scripts/build-bpf-objects`, `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`, and `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier -- --ignored aya_loads_staged_object_when_present`.
> - **Recently shipped (`v0.4` Phase A — staged-artifact hardening, this session):** [`scripts/stage-bpf-objects`](../scripts/stage-bpf-objects) now enforces EM_BPF machine validation under `--verify-elf` (not only ELF magic), preventing host placeholder binaries from being misclassified as staged BPF objects. The script now writes `crates/riftgate-obs-bpf/obj/ARTIFACT_FORMAT=staged-elf` on successful staging so the ignored Aya staged-load test runs in strict mode. Validation passed: `./scripts/build-bpf-objects`, `./scripts/stage-bpf-objects --from target/tmp/stage-src --verify-elf` (fails correctly on x86_64 placeholders), `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`, and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — build helper mode switch, this session):** [`scripts/build-bpf-objects`](../scripts/build-bpf-objects) now supports explicit `--mode placeholder|real`. `--mode placeholder` preserves the in-repo source-stub flow and writes `ARTIFACT_FORMAT=host-placeholder`; `--mode real --from <dir>` delegates to hardened staging with EM_BPF validation and writes `ARTIFACT_FORMAT=staged-elf` on success. This creates a single operator entrypoint while keeping strict separation between placeholder and real artifact paths. Validation passed: `./scripts/build-bpf-objects --mode placeholder`, `./scripts/build-bpf-objects --mode real --from target/tmp/real-src` (fails correctly on x86_64 placeholders), and `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`.
> - **Recently shipped (`v0.4` Phase A — real-artifact preflight diagnostics, this session):** added `--check-only` to [`scripts/stage-bpf-objects`](../scripts/stage-bpf-objects) and passthrough support in [`scripts/build-bpf-objects`](../scripts/build-bpf-objects) real mode. Preflight now validates source artifacts and prints per-slot ELF metadata (`Type`, `Machine`) before any copy, making Aya object integration failures easier to debug in-place. Validation passed: `./scripts/build-bpf-objects --mode placeholder`, `./scripts/build-bpf-objects --mode real --from crates/riftgate-obs-bpf/obj --check-only` (fails correctly with explicit `Machine` diagnostics on x86_64 placeholders), `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`, and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`.
> - **Recently shipped (`v0.4` Phase A — Aya program-source implementation pass, this session):** `crates/riftgate-obs-bpf/programs` now includes minimal real Aya eBPF entrypoints for all three slots (`cpu_sample`, `syscall_stall`, `tcp_retransmit`) behind a dedicated `bpf-programs` feature, keeping host workspace checks stable while enabling explicit `bpfel-unknown-none` builds in real mode. [`scripts/build-bpf-objects`](../scripts/build-bpf-objects) now supports `--mode real --build-from-source` and emits actionable diagnostics when local `bpfel` toolchain support is missing. [`crates/riftgate-obs/tests/bpf_verifier.rs`](../crates/riftgate-obs/tests/bpf_verifier.rs) now loads all three staged object slots in strict mode (non-placeholder marker), not only `cpu_sample`. Validation passed: `./scripts/cargow check -p riftgate-obs-bpf-programs`, `./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier`, and `./scripts/cargow clippy -p riftgate-obs --all-targets --features bpf -- -D warnings`. The final remaining blocker is environment-level: local `bpfel-unknown-none` core artifacts were unavailable, so `--build-from-source` fails before object emission.
> - **Recently shipped (`v0.4` Phase B — router signal integration, this session):** `WeightedRandomRouter` now folds `BackendSignal.gpu_pressure` into route-time selection: it prefers closed + cooler backends when available and safely falls back to any closed backend when all are hot. Added focused tests in [`crates/riftgate-router/src/weighted.rs`](../crates/riftgate-router/src/weighted.rs) for hot-backend de-prioritization and all-hot fallback. Validation passed: `./scripts/cargow test -p riftgate-router weighted::tests::`, `./scripts/cargow clippy -p riftgate-router --all-targets -- -D warnings`, and `./scripts/cargow test -p riftgate --features bpf --lib`.
> - **Recently shipped (`v0.4` Phase B — runtime signal refresh path, this session):** `crates/riftgate` now keeps routing signals in a live `ArcSwap<BackendSignals>` snapshot and includes an env-gated background DCGM poll loop (`RIFTGATE_GPU_DCGM_ENDPOINT`) that applies `GpuPressure::scalar_pressure()` updates to the routing snapshot via new helper module [`crates/riftgate/src/signals.rs`](../crates/riftgate/src/signals.rs). Proxy routing now reads the latest snapshot per request, and test/bench harnesses were updated for the new signal-store type. Validation passed: `./scripts/cargow test -p riftgate --lib`, `./scripts/cargow test -p riftgate --tests`, and `./scripts/cargow clippy -p riftgate --all-targets -- -D warnings`.
> - **Recently shipped (`v0.4` Phase C — documentation):** three new Options docs ([`014` eBPF integration](05-options/014-ebpf-integration.md), [`027` token-level metrics](05-options/027-token-level-metrics.md), [`028` GPU pressure correlation](05-options/028-gpu-pressure-correlation.md)); three new accepted ADRs ([`0024` eBPF via Aya](06-adrs/0024-ebpf-via-aya.md), [`0025` token-level metrics](06-adrs/0025-token-level-metrics-probabilistic.md), [`0026` GPU pressure via DCGM exporter](06-adrs/0026-gpu-pressure-via-dcgm-exporter.md)); [`lld-observability.md`](04-design/lld-observability.md) refreshed to promote `BpfSink`, `TokenLevelAggregator`, `DcgmScrapeSource`, `NvmlSource`, `NoopGpuSource` from "v0.4 reserved" to "v0.4 designed"; glossary additions for CMS, HDR histogram, HLL, kprobe, tracepoint, MIG, NVML, reservoir sampling. The §5 invariant ("no code without a corresponding Options doc and ADR") is now satisfied for every `v0.4` surface.
> - **Recently shipped (`v0.3` Phase A — documentation, earlier session):** four new Options docs ([`016`](05-options/016-extension-mechanism.md), [`019`](05-options/019-replay-eval.md), [`024`](05-options/024-stream-cancellation.md), [`025`](05-options/025-v03-routing-strategies.md)); five new accepted ADRs ([`0019`](06-adrs/0019-wasm-extension-mechanism.md), [`0020`](06-adrs/0020-stream-cancellation-cancellation-token.md), [`0021`](06-adrs/0021-external-replay-cli.md), [`0022`](06-adrs/0022-kv-aware-routing-prefix-trie.md), [`0023`](06-adrs/0023-hedged-requests-p99-triggered.md)); new LLD [`lld-filter-chain.md`](04-design/lld-filter-chain.md); refresh of [`lld-routing.md`](04-design/lld-routing.md).
> - **Recently shipped (`v0.2` the systems showpiece, tagged 2026-05-25):** see the [`v0.2` retrospective](02c-v0.2-retrospective.md) for the full close-out narrative.
> - **Environment baseline (implementation execution context):** development now targets a Lima VM (Ubuntu 24.04 LTS) running on macOS, defined in [`lima/riftgate.yaml`](../lima/riftgate.yaml). Lima routes guest network through the macOS host so outbound internet access is available — `rustup` and crates.io work directly with no proxy or internal tarball. System packages come from `apt` (provisioned automatically on first `limactl start`). Docker Engine is available inside the VM via `apt install docker.io` when needed for examples. See [`AGENTS.md`](../AGENTS.md) §11.5 and [`RUNBOOK.md`](../RUNBOOK.md) for the full setup walkthrough.
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

WASM filter chain, plugin-based routing strategies, hedged requests with stream cancellation. **Shipped and closed-out; see [`v0.3` retrospective](02d-v0.3-retrospective.md).**

### Foundational principles
- Extension models and sidecar/ambassador deployment patterns (Microsoft *Cloud Design Patterns*, Hohpe *Enterprise Integration Patterns*); per-stream FSM-based cancellation (table-driven state machines that handle the cancel transition cleanly).

### Functional requirements covered
FR-201 through FR-205, and (gated on `v0.2` retro) FR-206

### Subsystems landed
- `crates/riftgate-filter` — **landed:** native `FilterChain` executor plus production `WasmFilter` runtime over frozen `riftgate:filter/v1` Component Model ABI (wasmtime 31, pooling allocator, load-time AOT precompile validation). **Remaining hardening:** wallclock interruption enforcement and instance reuse micro-optimization.
- `KvAwareRouter<R>` and `HedgedRouter<R>` in `crates/riftgate-router` — **landed** as decorator-shaped impls.
- `crates/riftgate-core::cancel` (`Cancellation` + `CancellationDriver` + typed `CancelCause`) and SSE framer `Cancelled` terminal state — **landed.**
- `riftgate-replay` CLI binary with `dump`, `replay`, `eval` subcommands — **landed.**
- Per-core-scheduler feature option with tokio Handle::spawn dispatch — **landed.**
- Starter filter library boilerplate under `examples/02-starter-filters/` — **landed.**

### Options docs and ADRs added (and accepted)
- `010-routing-strategy.md` extended (KV-aware, hedged) + new ADRs
- `016-extension-mechanism.md` (none vs Lua vs WASM vs native trait) + ADR `0019`
- `019-replay-eval.md` (none vs embedded vs external CLI) + ADR `0021`
- `024-stream-cancellation.md` + ADR `0020`
- `025-v03-routing-strategies.md` + ADRs `0022`, `0023` (KV-aware routing prefix trie; hedged requests with P2 estimator)

### Goal metric
~1500 GitHub stars. First external contributor lands a custom filter or routing strategy. Conference talk submission accepted. ✅ **Gate met:** retrospective complete, all subsystems validated, 74 tests green, all verification gates passed.

### Retrospective
[`02d-v0.3-retrospective.md`](02d-v0.3-retrospective.md) — what shipped, what went well (ADR-first discipline, wasmtime integration, feature-mode testing), what we missed and fixed (scheduler dispatch, WasmFilter API iterations, Debug derive constraints), open questions (wallclock interruption, instance reuse tuning, streaming response filters v2), process notes, and the v0.3-complete decision tag.

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
