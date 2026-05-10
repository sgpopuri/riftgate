# 002. Async Runtime

> **Status:** `recommended` — Tokio multi-threaded runtime as the only embedded runtime in `v0.1`; thread-per-core runtimes (`monoio`, `glommio`) revisited at the `v0.2` retro behind the `Scheduler` trait. See [ADR 0003](../06-adrs/0003-tokio-multithread-default.md).
> **Foundational topics:** reactor pattern and event loops, `io_uring` (for thread-per-core context), work-stealing schedulers
> **Related options:** [001](001-io-model.md) (IO model), [003](003-concurrency-model.md) (concurrency model), [004](004-request-queue.md) (request queue)
> **Related ADR:** [ADR 0003](../06-adrs/0003-tokio-multithread-default.md)

## 1. The decision in one sentence

> Which Rust async runtime drives Riftgate's data plane in `v0.1`, and how do we keep the choice from leaking through `riftgate-core`'s public trait surface?

## 2. Context — what forces this decision

[Options 001](001-io-model.md) decided the kernel-facing IO mechanism (epoll on Linux, kqueue on macOS, io_uring as a `v0.2` opt-in). The async runtime is the userspace half of that choice: the executor that polls futures, the reactor that registers fds against the IO driver, the scheduler that distributes tasks across worker threads.

Forces driving this decision:

- **The `AsyncIO` trait must be the only abstraction that talks to the kernel** ([`docs/04-design/lld-io-runtime.md`](../04-design/lld-io-runtime.md)). The runtime sits *above* the trait, not beside it. A future swap of the runtime should not require a rewrite of `riftgate-parser`, `riftgate-router`, or any other consumer crate.
- **`v0.1` ships epoll only** ([ADR 0002](../06-adrs/0002-start-on-epoll.md)). A runtime that demands io_uring (`glommio`, `monoio` in their default configurations) is a bad fit for the first milestone even if it might be the right answer at `v0.2`.
- **The Rust async ecosystem is overwhelmingly Tokio-shaped.** `hyper`, `tonic`, `reqwest`, `tower`, `tracing`, `metrics`, `wasmtime`'s async API, OpenTelemetry's Rust SDK — all run on or assume a Tokio reactor. Choosing a non-Tokio runtime imports a substantial integration bill.
- **The `Scheduler` trait** ([`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md)) is where Riftgate makes the per-core / work-stealing choice. The runtime should be agnostic enough that both can sit on top of it (or that we can swap the runtime when the scheduler decision in `v0.2` changes the constraints).
- **Operability targets** ([NFR-O01](../01-requirements/non-functional.md), single static binary; [NFR-C01](../01-requirements/non-functional.md), <50 MB RSS idle) rule out runtimes that link in heavy unrelated dependencies.
- **NFR-P01 / NFR-P02** ask for <2 ms median and <10 ms P99 overhead at 1k QPS in `v0.1`. Every mainstream runtime meets this comfortably; the runtime choice is not the binding constraint here.

The decision is consequential because it cascades: the chosen runtime governs which IO drivers ship in tree, which task-spawning APIs `riftgate-core` exposes, which testing model `tokio::test` vs `monoio::test` we use, and how easy it is for an outside contributor to read the codebase.

## 3. Candidates

We evaluate five candidates spanning the Rust async-runtime spectrum, from the ecosystem default to a hand-rolled reactor.

### 3.1. Tokio (multi-threaded runtime)

**What it is.** The default Rust async runtime. A multi-threaded scheduler with a global injector queue, per-worker local queues, work-stealing across workers, an epoll/kqueue/IOCP reactor, async timer wheel, and a deeply integrated ecosystem (`hyper`, `tonic`, `tower`, `tracing`, `metrics`). Configurable as `current_thread` or `multi_thread`; the multi-thread variant is the default.

**Why it's interesting.**
- The pragmatic default: nearly every Rust async crate either runs on Tokio or assumes its presence. `hyper` and `tonic`, the two crates a gateway is most likely to consume, are Tokio-native.
- Scheduler behavior is well-understood and well-instrumented. `tokio-console` gives task-level introspection out of the box; `tokio-metrics` exports per-worker counters.
- Work-stealing is built in, with a thoughtful design: per-worker LIFO slot for the most-recently-scheduled task to keep producer/consumer pairs hot in cache, FIFO steal across workers to balance.
- Mature timer subsystem (hashed wheel under the hood) covers Riftgate's per-request deadline needs without us building a separate one in `v0.1`.
- Reactor is feature-flag-driven: `mio`-based by default (epoll on Linux, kqueue on macOS), with optional `tokio-uring` for io_uring-backed workloads.
- API stability: 1.x semver, no breaking changes since 1.0 (October 2020).

**Where it falls short.**
- Multi-threaded runtime ships a global injector queue and shared task state. The cache traffic is small but non-zero; thread-per-core proponents argue this is the wrong shape for very-high-fan-out servers.
- Work-stealing introduces a coherence-traffic floor that you cannot fully eliminate with `LocalSet` (Tokio's per-thread escape hatch is real but awkward).
- The reactor and the scheduler are coupled. You cannot swap to a custom IO driver without also bypassing Tokio's task system, which is the opposite of what Riftgate wants long-term.
- Tokio `Send + 'static` requirements on `tokio::spawn` impose lifetime discipline that some Rust patterns (e.g. arena-allocated futures) struggle with. `tokio::task::spawn_local` exists but requires `LocalSet`.
- A Tokio-by-default codebase tends to spread Tokio types (`tokio::sync::Mutex`, `tokio::time::sleep`, `tokio::net::TcpStream`) into module APIs unless reviewed for. The trait-surface discipline must be enforced.

**Real-world systems that use it.** Discord (chat servers, voice gateways), Cloudflare (parts of the edge), Fly.io (proxy layers), Linkerd2-proxy, Vector, Materialize, Polars, Tonic-based gRPC services across the industry. If the Rust project does network IO, the default answer is Tokio.

**Code sketch.**
```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle(stream));
    }
}
```

### 3.2. Tokio (current-thread runtime, sharded across cores)

**What it is.** Tokio's `current_thread` flavor: a single-threaded executor with no work-stealing and no shared state. A "thread-per-core" deployment runs one `current_thread` runtime per pinned worker thread, with the accept loop handing connections to worker shards via a queue. The runtime stays Tokio (so the ecosystem still works), but the multi-threaded scheduler's coherence traffic disappears.

**Why it's interesting.**
- Keeps the entire Tokio ecosystem (`hyper`, `tonic`, `tracing`, ...) while paying the price of giving up work-stealing.
- Predictable per-worker behavior: a slow request on worker 3 stays on worker 3. No tail-latency ripple from cross-worker steals on a hot worker.
- Trivial to combine with the per-core `Scheduler` impl decided in [Options 003](003-concurrency-model.md): `current_thread` Tokio runtimes are themselves the per-core executors.
- Lower per-task overhead. No global injector, no atomic on the LIFO slot, no work-stealing fast path.

**Where it falls short.**
- Loses Tokio's main implementation-quality feature (the work-stealer). For a workload with heterogeneous request costs, sharded `current_thread` can underperform `multi_thread` because slow tasks pile up on a single shard.
- Sharding is an architectural commitment. The accept loop must hash incoming connections to shards (round-robin, or hash-of-source-tuple); rebalancing is the operator's problem.
- API ergonomics regress: `tokio::spawn` from outside the runtime requires `Handle::spawn`; cross-shard task migration is a manual step.
- A "Tokio current-thread per core" architecture is closer to monoio / glommio in shape than to mainstream Tokio. If we're going to commit to thread-per-core, we should evaluate the runtimes that were *designed* for it.

**Real-world systems that use it.** Smaller crates that want Tokio's ecosystem with a shared-nothing shape: some test harnesses, `linkerd2-proxy`'s inbound handler is sharded current-thread-style, parts of `pingora` (Cloudflare's proxy framework) before they moved to their own runtime.

### 3.3. glommio (Datadog)

**What it is.** A thread-per-core async runtime built on io_uring. Each worker thread owns a CPU, an `io_uring` instance, and a local task queue. No work-stealing; cross-thread communication is through explicit channels. Task spawning is `glommio::spawn_local`, never global. Originated at Datadog for their high-throughput observability services.

**Why it's interesting.**
- Explicitly designed for the workload Riftgate is in: many concurrent network connections, low tail-latency budget, willingness to pin cores.
- Native io_uring support. When io_uring is on (`v0.2+` for Riftgate), glommio's design pays off — registered buffers, `IOSQE_FIXED_FILE`, `SQPOLL`-aware scheduling.
- Local-only execution means no `Send` requirement on tasks. Arena-allocated, non-`Send` futures Just Work, which is the right shape for our per-request `BumpArena` ([`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md)).
- The library actively models the cost of cross-CPU work; the API surface forces you to think about it.

**Where it falls short.**
- **io_uring-only.** Will not run on macOS, will not run on Linux <5.10, will not run inside container runtimes that block io_uring via seccomp (which is many of them, see [Options 001 §3.3](001-io-model.md)). For `v0.1`, this is disqualifying.
- **Smaller ecosystem.** `hyper`, `tonic`, `reqwest`, and the rest of the Tokio-native crates do not run on glommio without adapter layers. Riftgate would need to author or vendor those adapters.
- **Active maintenance has been uneven.** Datadog open-sourced glommio in 2020 and used it heavily; community contributions have been slower than Tokio's. As of 2026 the project is alive but on a slower release cadence.
- **Per-core means per-core.** The cost model (one CPU per worker, always pinned) is fine for dedicated boxes, awkward for K8s pods with fractional CPU requests, awkward for users running Riftgate as a sidecar.

**Real-world systems that use it.** Datadog's internal services (telemetry ingestion, agent backends), some smaller storage systems (Glommio is also marketed at Seastar-style storage workloads).

### 3.4. monoio (ByteDance)

**What it is.** A thread-per-core async runtime from ByteDance (the CloudWeGo project), io_uring-first with an `epoll`-fallback shim. Designed for the same shape as glommio but with a more recent codebase, an `IO_URING_SQPOLL` integration, and a small ecosystem of `monoio-*` crates (`monoio-http`, `monoio-tokio` adapter).

**Why it's interesting.**
- Same thread-per-core design as glommio with a more permissive license profile and a more active GitHub release cadence in 2024-2026.
- Provides a Tokio compatibility layer (`monoio-compat`) that lets Tokio-native crates (`hyper`, `tonic`) run on top of monoio's executor with a small overhead. This is meaningfully better than glommio's adapter story.
- io_uring backend is the default; epoll backend exists for portability, so unlike glommio, monoio can at least *run* on Linux <5.10 or in seccomp-restricted containers, even if the perf advantage of io_uring is lost.
- Backed by a hyperscaler (ByteDance) that has run it in production at scale.

**Where it falls short.**
- The compatibility layer is a real cost: every `hyper`-based call pays an extra hop through the adapter, eating much of the per-core advantage.
- Not yet a Rust ecosystem default. Most documentation, examples, and answers on Stack Overflow assume Tokio. New contributor friction.
- The `monoio-http` story is real but immature compared to `hyper`. Riftgate would either author a substantial chunk of HTTP code or accept hyper-via-adapter.
- Same `Send`-not-required model as glommio, which is a wash: it's nicer for arena-allocated futures, but it's a different mental model from the rest of the Rust ecosystem.

**Real-world systems that use it.** ByteDance's production gateways (the `cloudwego/kitex` Go-based gateway is the famous one; monoio is the Rust sibling). Adopted by some smaller Rust shops chasing thread-per-core P99.

### 3.5. Custom reactor on top of the `AsyncIO` trait

**What it is.** Build the runtime ourselves: a hand-rolled executor that polls `Future`s, a hand-rolled reactor that registers wakers against `AsyncIO::poll()`, a hand-rolled scheduler that distributes work across pinned worker threads. Riftgate becomes its own runtime author, with no upstream dependency on Tokio, glommio, or monoio.

**Why it's interesting.**
- Maximum control. Every allocation, every wakeup path, every cross-thread message is something we wrote and can profile.
- Ultimate trait fidelity. The runtime is *just* an implementation detail above `AsyncIO` and `Scheduler`; the abstraction never leaks because there's nothing else to leak.
- A fantastic teaching artifact. A documented in-tree reactor is the kind of thing a "documentation-first" project could be famous for.

**Where it falls short.**
- **Engineering cost.** Building a production-quality async executor is multiple person-years. Not a `v0.1` activity. Not a `v1.0` activity unless the project's center of gravity moves entirely to runtime authorship.
- **Ecosystem cost.** No `hyper`, no `tonic`, no `tracing` async layer, no `wasmtime` async API. We re-author or vendor each one.
- **Maintenance cost forever.** Every new Rust async language feature (e.g. async traits, async closures) becomes our problem to integrate.
- **Distracts from the differentiation pillars.** Riftgate's bet is pluggability + documentation + eBPF. "Custom runtime" is interesting but is not on the bet.
- We do not need this to keep the trait surface clean. Tokio plus discipline (Tokio types stay inside the impl crates, never in `riftgate-core` public APIs) achieves the same goal at 0.1% of the cost.

**Real-world systems that use it.** Pingora (Cloudflare) shipped a custom runtime after hitting Tokio's limits at hyperscaler edge volumes. ScyllaDB uses Seastar (C++); Glommio is the Rust port of that idea. These are projects with hundreds of engineers and a workload that genuinely justifies the build.

## 4. Tradeoff matrix

| Property | Tokio multi-thread | Tokio current-thread (sharded) | glommio | monoio | Custom reactor | Why it matters |
|----------|--------------------|--------------------------------|---------|--------|----------------|----------------|
| `v0.1` epoll support | yes (mio) | yes (mio) | no (io_uring only) | yes (epoll fallback) | yes (we write it) | `v0.1` ships epoll-only per [ADR 0002](../06-adrs/0002-start-on-epoll.md). |
| `v0.2` io_uring support | yes (`tokio-uring`) | yes (`tokio-uring`) | yes (native) | yes (native) | yes (we write it) | We add io_uring as an opt-in in `v0.2`. |
| macOS dev support | yes (kqueue via mio) | yes | no | partial (epoll fallback only) | dev cost | Maintainer on macOS; [NFR-PT03](../01-requirements/non-functional.md). |
| Ecosystem (`hyper`, `tonic`, `tower`, `tracing`, `wasmtime`) | native | native | adapter-only | adapter-only | none | Re-authoring this is a project-sized undertaking. |
| Scheduling model | work-stealing (default good) | shared-nothing (sharded) | shared-nothing | shared-nothing | our choice | The scheduler decision is [Options 003](003-concurrency-model.md). |
| Work over `Send` futures | required (multi-thread) | not required (single-thread) | not required | not required | our choice | Arena-allocated, non-`Send` futures are a Riftgate pattern; we can work around `Send` in Tokio with `LocalSet`. |
| Maturity / battle-testing | very high | very high | medium-high | medium | n/a | `v0.1` should not bet on a runtime that hasn't run heavy production. |
| API stability (1.x semver) | yes (since 2020) | yes | pre-1.0 | pre-1.0 | n/a | `riftgate-core` should not chase pre-1.0 churn. |
| Operational tooling (`tokio-console`, `tracing` integration) | excellent | excellent | basic | basic | none | At 3am on a pager this matters a lot. |
| Maintenance burden on Riftgate | low | low-medium | medium-high (adapter layer) | medium (compat layer) | very high | We have one maintainer in `v0.x`. |
| Compatibility with `AsyncIO` trait | natural fit | natural fit | natural fit | natural fit | trivially natural | The trait is intentionally runtime-agnostic. |
| Compatibility with future thread-per-core `Scheduler` | works (current-thread sharded) | natural fit | natural fit | natural fit | our choice | Door open for [Options 003](003-concurrency-model.md) at `v0.2`. |
| Risk of runtime types leaking into `riftgate-core` public API | medium (must enforce in review) | medium | low (different ecosystem so it's obvious) | low | none | The trait surface is the contract; reviewers must catch leaks. |

## 5. Foundational principles

**Reactor pattern and event loops.** The canonical reactor is described in Schmidt's *Pattern-Oriented Software Architecture, Volume 2* and in libev / libuv's design notes: an event source (selector or poller), a demultiplexer that maps events to handlers, and a dispatcher that runs handlers cooperatively. Tokio is the textbook reactor implementation in Rust — `mio::Poll` is the demultiplexer, the per-worker scheduler is the dispatcher, futures are the handler protocol. The reactor is an implementation detail of the runtime, not of the application: applications should program against the trait surface (`AsyncRead`, `AsyncWrite`, or our own `AsyncIO`) and let the runtime author worry about the loop. This is exactly the discipline we will enforce on `riftgate-core`: no Tokio types in public API surfaces.

**Work-stealing schedulers.** The classic Blumofe–Leiserson Cilk-5 paper makes the case for the default: heterogeneous-cost workloads benefit from cross-worker rebalancing because no single worker becomes a bottleneck. The Tokio scheduler post-mortem ("Making the Tokio scheduler 10× faster") and the Go scheduler design notes both observe the cost: cross-CPU steals trash the L1/L2 cache for the stolen task, and the overhead is significant for tiny tasks (sub-microsecond). For an LLM gateway, requests are *not* tiny — every request involves at minimum the parser, the router, and the upstream call. The cost model favors work-stealing.

**`io_uring` and thread-per-core runtimes.** Thread-per-core runtimes (Seastar, glommio, monoio) are a natural fit for `io_uring`'s per-thread submission-queue model: each thread owns its ring, no cross-thread submission contention. This is the strongest argument for revisiting the runtime choice when `io_uring` lands in `v0.2`. It is also the reason we explicitly decline to lock in a thread-per-core runtime now: the `io_uring` decision drives the runtime choice, not the other way around.

## 6. Recommendation

**`v0.1` ships with the Tokio multi-threaded runtime as the only embedded runtime. Maintain a strict trait-surface discipline so the runtime stays swappable. Revisit at the `v0.2` retro whether to add a thread-per-core runtime (monoio is the front-runner; glommio is the runner-up) once the io_uring backend lands.**

Reasoning, restated:

- Tokio is the pragmatic default: ecosystem-native, well-tooled, well-instrumented, 1.x stable, free of the obvious traps in our threat model.
- The design constraint (`AsyncIO` and `Scheduler` are the abstraction boundary) is enforced by code review and lint, not by the runtime choice. We do not need a non-Tokio runtime to keep our traits clean.
- The `v0.1` workload (epoll-only, single-binary, dev convenience on macOS, ecosystem integration with `hyper`/`tonic`/`tracing`/`wasmtime`/OTel) is a Tokio workload. Forcing a different choice here would manufacture risk for no measurable gain.
- The thread-per-core conversation is the right one to have, but it is the *`v0.2` retro* conversation. By then we have io_uring in tree, we have measured Tokio-multi-thread vs Tokio-current-thread on our own benchmark, and we know whether the multi-thread scheduler's overhead is something we feel.

### Conditions under which we'd revisit

- The `v0.2` retro shows that Tokio multi-thread's tail latency under our heterogeneous workload is materially worse than projected.
- The thread-per-core scheduler decision in [Options 003](003-concurrency-model.md) lands as the production default; at that point, a per-core-shaped runtime (monoio with the compat layer, or sharded current-thread Tokio) is worth measuring.
- Tokio's API stability ever changes meaningfully, or its maintenance posture changes (which would be surprising — the project's track record since 1.0 is excellent).
- If the project ever needs a true sub-millisecond P99 (which we are explicit we will not chase, see [Vision §4](../00-vision.md)).

### What stays available behind feature flags

- `tokio-uring` integration in `v0.2`, gated by the `io-uring` cargo feature decided in [ADR 0002](../06-adrs/0002-start-on-epoll.md). The runtime stays Tokio; only the IO driver changes.
- A future `monoio` or `glommio` `Scheduler` impl can ship behind a `--features per-core-runtime` flag if the `v0.2` retro decides to invest. Until then, the door is open via the trait, not the implementation.

## 7. What we explicitly reject

- **Custom reactor as the `v0.x` runtime.** Multi-person-year cost, no leverage of the existing Rust ecosystem, distracts from Riftgate's actual differentiation pillars. Reconsider only if Riftgate evolves into a runtime-author project, which is not the bet.
- **`async-std`.** Maintenance has stalled (most recent significant release was 1.12 in 2022). Not a serious 2026 candidate.
- **`smol`.** Lightweight, well-engineered, but not the runtime that `hyper`/`tonic`/`tracing`/`wasmtime`'s async API target. We would pay the integration cost for very little benefit.
- **Sharded Tokio `current_thread` as the `v0.1` default.** Premature commitment to thread-per-core before [Options 003](003-concurrency-model.md) has decided. The work-stealing scheduler is fine for the `v0.1` workload.
- **glommio as the `v0.1` runtime.** io_uring-only forecloses macOS dev, forecloses Linux <5.10, forecloses every container runtime that disables io_uring. Out of step with [ADR 0002](../06-adrs/0002-start-on-epoll.md).

## 8. References

1. Tokio project — https://tokio.rs
2. Carl Lerche, "Making the Tokio scheduler 10× faster" — https://tokio.rs/blog/2019-10-scheduler
3. Carl Lerche, "Reducing tail latencies with Tokio" — https://tokio.rs/blog/2020-04-preemption
4. Glommio project — https://github.com/DataDog/glommio
5. monoio project — https://github.com/bytedance/monoio
6. Pingora (Cloudflare) blog: "How we built Pingora, the proxy that connects Cloudflare to the Internet" — https://blog.cloudflare.com/how-we-built-pingora-the-proxy-that-connects-cloudflare-to-the-internet/
7. ScyllaDB / Seastar architecture — https://seastar.io/
8. Robert D. Blumofe and Charles E. Leiserson, *Scheduling Multithreaded Computations by Work Stealing* (FOCS 1994 / J. ACM 1999) — the foundational work-stealing analysis.
9. Douglas C. Schmidt, *Reactor: An Object Behavioral Pattern for Demultiplexing and Dispatching Handles for Synchronous Events* (POSA Volume 2) — the canonical reactor reference.
10. Go runtime scheduler design — https://go.dev/src/runtime/proc.go (top-of-file commentary).
