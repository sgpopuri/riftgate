# 08. Glossary

> Terms used across Riftgate docs. Keep this list short, precise, and one definition per term. If a term needs a paragraph, link out to the relevant Options doc or LLD instead.

---

**ADR — Architecture Decision Record.** A short, decisive document capturing context, decision, and consequences for a single architectural choice. Michael Nygard's format. See [`docs/06-adrs/`](06-adrs/).

**AF_XDP.** Linux socket type that pairs with an XDP eBPF program to bypass most of the kernel network stack while staying inside the kernel security model. See [Options 001 §3.5](05-options/001-io-model.md#35-af_xdp-kernel-assisted-bypass).

**Aya.** A pure-Rust eBPF library; the planned eBPF runtime for Riftgate's [observability plane](03-architecture/observability-plane.md).

**Backpressure.** A mechanism by which a downstream component signals an upstream component to slow down or stop. In Riftgate, backpressure is a *policy* (drop newest, drop oldest, block, return 503), not a *mechanism* — the mechanism is the bounded MPSC channel.

**BPF.** Berkeley Packet Filter. The original meaning is the packet-filtering language; modern usage typically means **eBPF** (extended BPF), which extends the original to a general in-kernel virtual machine.

**Circuit breaker.** A pattern that takes a failing dependency out of rotation after a failure threshold and probes it periodically to see if it has recovered. Three states: closed (normal), open (failing, traffic excluded), half-open (probing).

**Component context.** Durable, theory-of-the-system knowledge for one subsystem, co-located with the code. The component-context surfaces in Riftgate are the [LLDs](04-design/) and the [`AGENTS.md`](../AGENTS.md) entry points.

**CO-RE — Compile Once Run Everywhere.** A technique for writing eBPF programs that work across kernel versions by reading kernel structures via `BPF_CORE_READ` and BTF (BPF Type Format) metadata.

**CRD — Custom Resource Definition.** Kubernetes-specific concept; an extension to the Kubernetes API. Riftgate `v1.0` will define `Riftgate`, `RiftgateBackend`, and `RiftgateRoute` CRDs.

**Data plane.** The per-request hot path. In Riftgate, the data plane is the Rust kernel: IO, parser, queue, scheduler, allocator, timers, request log. Distinct from the [control plane](03-architecture/control-plane.md), [extension plane](03-architecture/extension-plane.md), and [observability plane](03-architecture/observability-plane.md).

**DCGM.** NVIDIA Data Center GPU Manager. Provides telemetry from NVIDIA GPUs (utilization, memory pressure, throttle reasons). Riftgate `v0.4` correlates DCGM signals with backend choice.

**DPDK — Data Plane Development Kit.** Userland NIC framework that bypasses the kernel entirely. See [Options 001 §3.4](05-options/001-io-model.md#34-dpdk-kernel-bypass-userland-nic).

**eBPF — extended BPF.** A safe, verified, JIT-compiled in-kernel programming environment. Used in Riftgate for the observability plane.

**Edge-triggered (ET).** An epoll mode where the kernel notifies the application only when an fd's readiness *changes*. Requires the application to drain the fd to `EAGAIN` to avoid hangs. Faster than level-triggered when correctly implemented.

**epoll.** The Linux readiness-based fd multiplexer. The default IO model in Riftgate `v0.1`. See [Options 001 §3.1](05-options/001-io-model.md#31-epoll-linux).

**Extension plane.** The pluggable behavior surface in Riftgate: filter chain, WASM runtime, routing strategies.

**Filter (Riftgate).** A request- or response-side hook that can read, modify, or terminate a request. Implemented as a `Filter` trait impl, either native Rust or compiled to WASM.

**Hedged request.** A request sent to two backends in parallel; the first response wins, the slower is cancelled mid-stream. Standard Google SRE technique. Lands in Riftgate `v0.3`.

**Hierarchical timing wheel.** A multi-level timing-wheel data structure providing O(1) amortized insert and cancel for large numbers of concurrent timers. See [Options 006](05-options/006-timer-subsystem.md).

**HLD — High-Level Design.** [Architecture overview](03-architecture/hld.md) at the level of subsystems and planes, not implementations.

**HSM — Hierarchical State Machine.** An FSM with parent/child state relationships and inherited transitions. Used in protocol parsing for shared transitions (e.g. "any state → reset on connection close").

**io_uring.** Linux's completion-based async IO interface introduced in 2019. Two shared-memory rings between userspace and kernel. See [Options 001 §3.3](05-options/001-io-model.md#33-io_uring-linux-51).

**KV cache.** In LLM inference, the key-value tensors cached for previously-seen prompt tokens. Routing requests to the backend with a warm prefix-matching KV cache reduces latency dramatically. See [Options 010](05-options/010-routing-strategy.md).

**kqueue.** BSD/macOS unified event-notification interface. Riftgate's macOS backend.

**Level-triggered (LT).** An epoll mode where the kernel notifies the application as long as an fd is ready. Easier to write correctly than ET; slightly higher overhead.

**LLD — Low-Level Design.** Per-subsystem detailed design. See [`docs/04-design/`](04-design/).

**LMCache.** A KV-cache management library used by some vLLM deployments. The `vllm-router` project provides prefix-aware routing using LMCache's lookup endpoint. Riftgate may integrate as one of several routing strategies.

**MPMC.** Multi-Producer Multi-Consumer queue. A lock-free queue where multiple threads can both enqueue and dequeue. Vyukov's bounded MPMC is the canonical reference.

**MPSC.** Multi-Producer Single-Consumer channel. The pattern Riftgate uses between the data plane (many producers) and the observability sinks (one consumer per sink).

**NUMA — Non-Uniform Memory Access.** A multi-socket CPU architecture where memory access cost depends on which socket owns the memory. Cross-NUMA access is ~2× slower than local. Affects scheduler and IO design.

**Observability plane.** The OTel + Prometheus + eBPF surface in Riftgate. See [`docs/03-architecture/observability-plane.md`](03-architecture/observability-plane.md).

**Options doc.** A Riftgate-specific design-decision artifact. Exhaustive exploration of candidates for one decision; ends with a recommendation that becomes an ADR. See [`docs/05-options/_template.md`](05-options/_template.md).

**OTel — OpenTelemetry.** The vendor-neutral standard for traces, metrics, and logs. Riftgate emits OTel as the default observability path.

**Persona (Riftgate).** A specific named user we design for. See [`docs/01-requirements/personas.md`](01-requirements/personas.md). Pia (platform engineer), Rohan (inference SRE), Maya (systems learner), Devansh (contributor).

**Plausible-wrong.** Output that looks correct, reads fluently, passes fast review, and is incorrect in a way only a careful read reveals. The category Riftgate's docs and reviews exist to defend against.

**Plane (Riftgate architecture).** One of four logical layers: data plane, extension plane, observability plane, control plane. Plane boundaries are the natural seams for swapping implementations.

**Project context.** Temporary, project-scoped knowledge — current spec, open questions, session logs, handoffs. The project-context surface in Riftgate lives at the top of [`docs/02-mvp-roadmap.md`](02-mvp-roadmap.md) under "Currently shipping."

**Reactor pattern.** Event-driven concurrency model: an event loop demultiplexes IO events to handlers. Riftgate's default pattern. Contrast with the proactor pattern (completion-based).

**Replay log.** The Riftgate request log; see WAL.

**Router (Riftgate).** A `Router` trait impl that decides which backend should serve a request. Pluggable.

**SQE / CQE.** io_uring's Submission Queue Entry (64 bytes) and Completion Queue Entry (16 bytes). The basic units of work in io_uring.

**SQPOLL.** io_uring mode where a kernel thread polls the SQ continuously, allowing userspace to submit work with zero syscalls. Costs a CPU core; valuable on dedicated hardware.

**SSE — Server-Sent Events.** The HTTP streaming format used by OpenAI's `chat/completions` and many LLM APIs. `data:` lines separated by blank lines. Parsing requires an FSM that handles partial lines. See [`docs/04-design/lld-parsing.md`](04-design/lld-parsing.md).

**Thread-per-core.** A concurrency model where each CPU core has one dedicated worker thread, with no shared mutable state in the hot path. Riftgate's default. See [Options 003](05-options/003-concurrency-model.md).

**TTFT — Time To First Token.** The wall-clock time from when a streaming request is received to when the first token is emitted to the client. The user-perceived "is it working?" latency. Riftgate emits TTFT histograms in `v0.4`.

**Vyukov MPMC.** Dmitry Vyukov's bounded multi-producer multi-consumer queue using sequence numbers per cell. Riftgate's `MpmcQueue` implementation pattern.

**WAL — Write-Ahead Log.** An append-only log that records intended state changes before they are applied. Riftgate's request log is WAL-shaped: records (request, response) pairs for replay. See [`docs/04-design/lld-storage.md`](04-design/lld-storage.md).

**WASM — WebAssembly.** A bytecode format with a sandboxed execution model. Riftgate's filter chain runs WASM modules via wasmtime in `v0.3`.

**Work stealing.** A scheduler pattern where idle workers steal tasks from busy workers' queues. Chase-Lev deque is the canonical implementation. Opt-in in Riftgate `v0.2`.

**XDP — eXpress Data Path.** A Linux feature that allows eBPF programs to run at the NIC driver level, before the kernel network stack. See [Options 001 §3.5](05-options/001-io-model.md#35-af_xdp-kernel-assisted-bypass).
