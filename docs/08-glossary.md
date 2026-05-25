# 08. Glossary

> Terms used across Riftgate docs. Keep this list short, precise, and one definition per term. If a term needs a paragraph, link out to the relevant Options doc or LLD instead.

---

**ADR — Architecture Decision Record.** A short, decisive document capturing context, decision, and consequences for a single architectural choice. Michael Nygard's format. See [`docs/06-adrs/`](06-adrs/).

**AF_XDP.** Linux socket type that pairs with an XDP eBPF program to bypass most of the kernel network stack while staying inside the kernel security model. See [Options 001 §3.5](05-options/001-io-model.md#35-af_xdp-kernel-assisted-bypass).

**Aya.** A pure-Rust eBPF library; the planned eBPF runtime for Riftgate's [observability plane](03-architecture/observability-plane.md).

**Backpressure.** A mechanism by which a downstream component signals an upstream component to slow down or stop. In Riftgate, backpressure is a *policy* (drop newest, drop oldest, block, return 503), not a *mechanism* — the mechanism is the bounded MPSC channel.

**BPF.** Berkeley Packet Filter. The original meaning is the packet-filtering language; modern usage typically means **eBPF** (extended BPF), which extends the original to a general in-kernel virtual machine.

**Cancellation token.** A clonable handle exposing a `cancelled()` future plus a sticky boolean state. Cancelling the token wakes every waiter and flips the state. Riftgate wraps `tokio_util::sync::CancellationToken` in a `Cancellation` newtype that pairs the token with a typed `CancelCause`. See [Options `024`](05-options/024-stream-cancellation.md) and [ADR `0020`](06-adrs/0020-stream-cancellation-cancellation-token.md).

**CancelCause.** A Riftgate-typed enum describing *why* a cancellation fired: `HedgedLoser`, `FilterTerminate`, `ClientDisconnect`, `UpstreamTimeout`, `Shutdown`. Carried alongside the cancellation primitive so post-incident attribution is mechanical, not heuristic.

**Circuit breaker.** A pattern that takes a failing dependency out of rotation after a failure threshold and probes it periodically to see if it has recovered. Three states: closed (normal), open (failing, traffic excluded), half-open (probing).

**Component Model (WebAssembly).** The component-level abstraction that sits above core WebAssembly modules: typed interfaces (WIT), worlds (capability bundles), and the discipline of cross-language composition without a shared ABI in the C sense. The WebAssembly Component Model is Riftgate's `v0.3` filter ABI substrate — the contract is frozen at `riftgate:filter/v1`. See [Options `016`](05-options/016-extension-mechanism.md).

**Count–Min Sketch (CMS).** A probabilistic counting structure (Cormode–Muthukrishnan, 2005). A `d × w` matrix of counters with `d` independent hash functions; updates are O(1) per observation; queries return an overcount-only estimate with bounded error. Riftgate considers CMS for `v1.0+` "top-K tenants by token burn" queries; `v0.4` defers per [Options `027`](05-options/027-token-level-metrics.md).

**Component context.** Durable, theory-of-the-system knowledge for one subsystem, co-located with the code. The component-context surfaces in Riftgate are the [LLDs](04-design/) and the [`AGENTS.md`](../AGENTS.md) entry points.

**CO-RE — Compile Once Run Everywhere.** A technique for writing eBPF programs that work across kernel versions by reading kernel structures via `BPF_CORE_READ` and BTF (BPF Type Format) metadata.

**CRD — Custom Resource Definition.** Kubernetes-specific concept; an extension to the Kubernetes API. Riftgate `v1.0` will define `Riftgate`, `RiftgateBackend`, and `RiftgateRoute` CRDs.

**Data plane.** The per-request hot path. In Riftgate, the data plane is the Rust kernel: IO, parser, queue, scheduler, allocator, timers, request log. Distinct from the [control plane](03-architecture/control-plane.md), [extension plane](03-architecture/extension-plane.md), and [observability plane](03-architecture/observability-plane.md).

**DCGM.** NVIDIA Data Center GPU Manager. Provides telemetry from NVIDIA GPUs (utilization, memory pressure, throttle reasons, ECC error counts). Riftgate `v0.4`'s default `GpuPressureSource` impl scrapes the `dcgm-exporter` Prometheus endpoint at operator-configured cadence. See [ADR `0026`](06-adrs/0026-gpu-pressure-via-dcgm-exporter.md).

**DPDK — Data Plane Development Kit.** Userland NIC framework that bypasses the kernel entirely. See [Options 001 §3.4](05-options/001-io-model.md#34-dpdk-kernel-bypass-userland-nic).

**eBPF — extended BPF.** A safe, verified, JIT-compiled in-kernel programming environment. Used in Riftgate `v0.4` for the observability plane (CPU on/off-time profiling, syscall stalls, TCP retransmits per upstream) via Aya, behind `cfg(all(target_os = "linux", feature = "bpf"))` and runtime-gated by `RIFTGATE_ENABLE_BPF=1`. See [Options `014`](05-options/014-ebpf-integration.md) and [ADR `0024`](06-adrs/0024-ebpf-via-aya.md).

**Edge-triggered (ET).** An epoll mode where the kernel notifies the application only when an fd's readiness *changes*. Requires the application to drain the fd to `EAGAIN` to avoid hangs. Faster than level-triggered when correctly implemented.

**epoll.** The Linux readiness-based fd multiplexer. The default IO model in Riftgate `v0.1`. See [Options 001 §3.1](05-options/001-io-model.md#31-epoll-linux).

**Extension plane.** The pluggable behavior surface in Riftgate: filter chain, WASM runtime, routing strategies.

**Filter (Riftgate).** A request- or response-side hook that can read, modify, or terminate a request. Implemented as a `Filter` trait impl, either native Rust or compiled to WASM.

**Hedged request.** A request sent to two backends in parallel; the first response wins, the slower is cancelled mid-stream. Standard Google SRE technique. Lands in Riftgate `v0.3`.

**HDR histogram.** Gil Tene's High Dynamic Range histogram — a fixed-precision histogram across a bounded but wide range (e.g. 1 µs to 1 hour) with O(1) update cost and constant memory. Riftgate `v0.4`'s `TokenLevelAggregator` uses HDR histograms for TTFT, inter-token, and jitter latency aggregates. See [ADR `0025`](06-adrs/0025-token-level-metrics-probabilistic.md).

**Hierarchical timing wheel.** A multi-level timing-wheel data structure providing O(1) amortized insert and cancel for large numbers of concurrent timers. See [Options 006](05-options/006-timer-subsystem.md).

**HyperLogLog (HLL).** A cardinality-estimation sketch (Flajolet et al., 2007). Fixed memory (~12 KB) gives ±2% standard error across billions of distinct items. Mergeable across shards. Catalogued for Riftgate but **not** used in `v0.4` token-level metrics — the dimension-cap pattern is simpler. See [Options `027`](05-options/027-token-level-metrics.md).

**HLD — High-Level Design.** [Architecture overview](03-architecture/hld.md) at the level of subsystems and planes, not implementations.

**HSM — Hierarchical State Machine.** An FSM with parent/child state relationships and inherited transitions. Used in protocol parsing for shared transitions (e.g. "any state → reset on connection close").

**io_uring.** Linux's completion-based async IO interface introduced in 2019. Two shared-memory rings between userspace and kernel. See [Options 001 §3.3](05-options/001-io-model.md#33-io_uring-linux-51).

**KV cache.** In LLM inference, the key-value tensors cached for previously-seen prompt tokens. Routing requests to the backend with a warm prefix-matching KV cache reduces latency dramatically. See [Options 010](05-options/010-routing-strategy.md).

**kprobe.** A Linux kernel-tracing mechanism allowing eBPF programs to attach to function entry points anywhere in the kernel. Used in Riftgate `v0.4` for TCP-layer observability (retransmit accounting). Per kernel-version-specific symbol stability; CO-RE mitigates portability.

**kqueue.** BSD/macOS unified event-notification interface. Riftgate's macOS backend.

**Level-triggered (LT).** An epoll mode where the kernel notifies the application as long as an fd is ready. Easier to write correctly than ET; slightly higher overhead.

**LLD — Low-Level Design.** Per-subsystem detailed design. See [`docs/04-design/`](04-design/).

**LMCache.** A KV-cache management library used by some vLLM deployments. The `vllm-router` project provides prefix-aware routing using LMCache's lookup endpoint. Riftgate may integrate as one of several routing strategies.

**MPMC.** Multi-Producer Multi-Consumer queue. A lock-free queue where multiple threads can both enqueue and dequeue. Vyukov's bounded MPMC is the canonical reference.

**MPSC.** Multi-Producer Single-Consumer channel. The pattern Riftgate uses between the data plane (many producers) and the observability sinks (one consumer per sink).

**MIG — Multi-Instance GPU.** NVIDIA's hardware partitioning of a single GPU into independent slices, each with its own SM, memory, and L2 cache partition. Riftgate `v0.4`'s `DcgmScrapeSource` and `NvmlSource` both accept a `mig_uuid` to address a specific slice per backend.

**NUMA — Non-Uniform Memory Access.** A multi-socket CPU architecture where memory access cost depends on which socket owns the memory. Cross-NUMA access is ~2× slower than local. Affects scheduler and IO design.

**NVML — NVIDIA Management Library.** NVIDIA's in-process C library for per-host GPU telemetry (utilization, memory, throttle reasons, per-process GPU memory). Riftgate `v0.4` offers `NvmlSource` as a feature-gated (`gpu-nvml`) alternative `GpuPressureSource` impl for GPU-co-located deployments. See [ADR `0026`](06-adrs/0026-gpu-pressure-via-dcgm-exporter.md).

**Observability plane.** The OTel + Prometheus + eBPF surface in Riftgate. See [`docs/03-architecture/observability-plane.md`](03-architecture/observability-plane.md).

**Options doc.** A Riftgate-specific design-decision artifact. Exhaustive exploration of candidates for one decision; ends with a recommendation that becomes an ADR. See [`docs/05-options/_template.md`](05-options/_template.md).

**OTel — OpenTelemetry.** The vendor-neutral standard for traces, metrics, and logs. Riftgate emits OTel as the default observability path.

**P² algorithm.** Jain & Chlamtac's 1985 quantile-estimation algorithm. Five moving markers updated in O(1) per observation; estimates a target quantile (e.g. p95) without storing the sample. Riftgate uses one P² estimator per backend in `HedgedRouter` to compute first-byte p95 latency for the hedge trigger. See [ADR `0023`](06-adrs/0023-hedged-requests-p99-triggered.md).

**Persona (Riftgate).** A specific named user we design for. See [`docs/01-requirements/personas.md`](01-requirements/personas.md). Pia (platform engineer), Rohan (inference SRE), Maya (systems learner), Devansh (contributor).

**Plausible-wrong.** Output that looks correct, reads fluently, passes fast review, and is incorrect in a way only a careful read reveals. The category Riftgate's docs and reviews exist to defend against.

**Plane (Riftgate architecture).** One of four logical layers: data plane, extension plane, observability plane, control plane. Plane boundaries are the natural seams for swapping implementations.

**Prefix trie.** A tree whose nodes index strings (or hashed byte chunks) by their prefix; descending from root to leaf builds the full key. Knuth, TAOCP vol. 3 §6.3. Riftgate's `KvAwareRouter` uses a prefix trie keyed by chunked xxHash3-64 hashes to route requests with shared prefixes to the same backend. See [ADR `0022`](06-adrs/0022-kv-aware-routing-prefix-trie.md).

**Project context.** Temporary, project-scoped knowledge — current spec, open questions, session logs, handoffs. The project-context surface in Riftgate lives at the top of [`docs/02-mvp-roadmap.md`](02-mvp-roadmap.md) under "Currently shipping."

**Reactor pattern.** Event-driven concurrency model: an event loop demultiplexes IO events to handlers. Riftgate's default pattern. Contrast with the proactor pattern (completion-based).

**Replay log.** The Riftgate request log; see WAL.

**Reservoir sampling.** Vitter's Algorithm R (1985): a one-pass sampling algorithm that maintains a uniformly-random `K`-element sample from a stream of unknown length using `O(K)` memory. Riftgate `v0.4`'s `TokenLevelAggregator` uses reservoir sampling for bounded random per-token spans (default `K = 100` per `(tenant, model, route)` per 60 s window) — the count-bounded shape preserves rare slow tails that a rate-based sampler would discard. See [ADR `0025`](06-adrs/0025-token-level-metrics-probabilistic.md).

**Router (Riftgate).** A `Router` trait impl that decides which backend should serve a request. Pluggable.

**SQE / CQE.** io_uring's Submission Queue Entry (64 bytes) and Completion Queue Entry (16 bytes). The basic units of work in io_uring.

**SQPOLL.** io_uring mode where a kernel thread polls the SQ continuously, allowing userspace to submit work with zero syscalls. Costs a CPU core; valuable on dedicated hardware.

**SSE — Server-Sent Events.** The HTTP streaming format used by OpenAI's `chat/completions` and many LLM APIs. `data:` lines separated by blank lines. Parsing requires an FSM that handles partial lines. See [`docs/04-design/lld-parsing.md`](04-design/lld-parsing.md).

**Thread-per-core.** A concurrency model where each CPU core has one dedicated worker thread, with no shared mutable state in the hot path. Riftgate's default. See [Options 003](05-options/003-concurrency-model.md).

**TTFT — Time To First Token.** The wall-clock time from when a streaming request is received to when the first token is emitted to the client. The user-perceived "is it working?" latency. Riftgate emits TTFT histograms in `v0.4` via the `TokenLevelAggregator` (HDR histogram per `(tenant, model, route)`).

**Tracepoint.** A static, kernel-developer-curated tracing hook compiled into the Linux kernel. More stable across kernel versions than kprobes; used by Riftgate `v0.4` for syscall-stall observability. Documented under `Documentation/trace/tracepoints.rst` in the kernel tree.

**Vyukov MPMC.** Dmitry Vyukov's bounded multi-producer multi-consumer queue using sequence numbers per cell. Riftgate's `MpmcQueue` implementation pattern.

**WAL — Write-Ahead Log.** An append-only log that records intended state changes before they are applied. Riftgate's request log is WAL-shaped: records (request, response) pairs for replay. See [`docs/04-design/lld-storage.md`](04-design/lld-storage.md).

**WASI Preview 2.** The 2024 iteration of the WebAssembly System Interface, defined in terms of the Component Model. Replaces the ad-hoc Preview 1 imports with typed interface contracts. The substrate on which `riftgate:filter/v1` is defined.

**WASM — WebAssembly.** A bytecode format with a sandboxed execution model. Riftgate's filter chain runs WebAssembly components via wasmtime in `v0.3`.

**WIT — WebAssembly Interface Type.** The IDL of the Component Model. Riftgate's filter contract is declared in a `.wit` file under `crates/riftgate-filter/wit/`.

**Work stealing.** A scheduler pattern where idle workers steal tasks from busy workers' queues. Chase-Lev deque is the canonical implementation. Opt-in in Riftgate `v0.2`.

**XDP — eXpress Data Path.** A Linux feature that allows eBPF programs to run at the NIC driver level, before the kernel network stack. See [Options 001 §3.5](05-options/001-io-model.md#35-af_xdp-kernel-assisted-bypass).

**xxHash3.** A non-cryptographic hash function family (Yann Collet, 2019). The 64-bit variant is fast (sub-ns per byte on modern x86_64), well-distributed, and `no_std`-friendly. Riftgate uses xxHash3-64 in `KvAwareRouter` to chunk-hash the request prefix into trie keys. See [ADR `0022`](06-adrs/0022-kv-aware-routing-prefix-trie.md).
