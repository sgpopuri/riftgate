# 003. Concurrency Model

> **Status:** `recommended` — shared-nothing per-shard request scheduling in `v0.1`; work-stealing scheduler added as an opt-in in `v0.2`. See [ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md).
> **Source-systems chapters:** `Ch7 (work stealing)`, `Ch12 (system design patterns)`, `Ch4 (lock-free MPMC)` (for the queue half of the decision)
> **Related options:** [001](001-io-model.md) (IO model), [002](002-async-runtime.md) (async runtime), [004](004-request-queue.md) (request queue), [022](README.md) (priority/fairness, optional, gated on `v0.2` retro)
> **Related ADR:** [ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md)

## 1. The decision in one sentence

> How does Riftgate distribute work across worker threads at the `Scheduler` trait level — shared-state, shared-nothing per shard, work-stealing across shards, or actor-model?

## 2. Context — what forces this decision

[Options 002](002-async-runtime.md) decided the runtime layer (Tokio multi-thread). This decision is *one layer up*: how the `Scheduler` trait in `riftgate-core` distributes requests across worker shards, regardless of how the underlying runtime schedules futures across OS threads.

The two layers do not collapse into one another. Tokio's internal scheduler is work-stealing across OS threads; Riftgate's `Scheduler` trait can be shared-nothing at the request-distribution level even if the underlying executor steals tasks. They are concerns at different scales:

- **Runtime scheduling** ([Options 002](002-async-runtime.md)): which OS thread polls a given future. Tokio handles this.
- **Request scheduling** (this Options doc): which Riftgate worker shard owns a given request from accept to response. This is a Riftgate decision about pluggability, predictability, and cache locality.

Forces driving this decision:

- **Tail latency over peak throughput.** [NFR-P02](../01-requirements/non-functional.md) targets <10 ms P99 in `v0.1`. A shared global queue introduces lock contention or cross-CPU coherence traffic that bites tail latency under load.
- **Predictability for `v0.1`, throughput optimization for `v0.2`.** The first milestone wants the scheduler to be *easy to reason about* so that we can ship a walking skeleton and benchmark honestly. Work-stealing is the right answer for heterogeneous mixes but is the wrong starting point because it hides the load shape from operators.
- **Cache locality on the hot path.** A request's per-request arena ([`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md)), parser scratch buffers, and filter state should not bounce between CPU caches. Shared-nothing keeps everything on one shard's L1/L2.
- **The `Scheduler` trait is pluggable.** [`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md) names `PerCoreScheduler` (`v0.1`) and `WorkStealingScheduler` (`v0.2` opt-in). The decision is which shape ships first as the default and what the trait must accommodate to keep the second one swap-in friendly.
- **Linear scalability.** [NFR-S03](../01-requirements/non-functional.md) targets ≥80% of single-core throughput per added core, up to 16 cores. A shared global queue plateaus well before 16 cores under contention. Sharded models scale further.
- **Operability.** [NFR-O02](../01-requirements/non-functional.md) and [NFR-OBS02](../01-requirements/non-functional.md) want per-shard queue depths in the metrics endpoint. Sharded models surface this naturally; shared models surface only an aggregate.

## 3. Candidates

We evaluate four candidates spanning the scheduler design space, from the textbook-naive default to the model favored by the most-engineered network systems.

### 3.1. Shared-state worker pool (single global queue + N workers)

**What it is.** A single FIFO queue (mutex-protected `VecDeque<Task>`, or a single lock-free MPMC) plus N worker threads that each `pop` from it. The accept loop pushes; workers compete for tasks. The simplest scheduler shape; the textbook answer for "I just need to do work in parallel."

**Why it's interesting.**
- Easiest to write. Easiest to reason about for "throughput as a property" (work that arrives gets done if any worker is free).
- Naturally load-balances homogeneous work: any free worker grabs the next task, no shaping needed.
- Smallest amount of code in the hot path.

**Where it falls short.**
- **Single contention point.** Mutex-based variants serialize on the lock; lock-free MPMC variants serialize on the head atomic. Either way, scaling past ~4 workers brings sharply diminishing returns on a network workload.
- **Cache-line bouncing.** Every worker reads the queue head, so the cache line is constantly invalidated across CPUs. The cost is small per operation but scales with worker count.
- **Tail latency unpredictable.** A burst of requests + one slow worker = the queue head sticks behind the slow worker's slot in the FIFO. Hard to debug without per-worker visibility, which a single-queue model does not provide.
- **No per-shard isolation.** If a runaway request consumes a worker for seconds, the global queue still appears "fast" because other workers drain it; but per-shard SLOs (a `v0.x+` need) cannot be enforced.

**Real-world systems that use it.** Many small Rust services, default behavior of stock thread pools (`rayon::ThreadPool`, `threadpool` crate), pre-vertical-scaling Java / Python services. Generally a transitional default that gets replaced when the workload outgrows it.

**Code sketch.**
```rust
let queue = Arc::new(MpmcQueue::new(2048));
for _ in 0..num_workers {
    let q = queue.clone();
    std::thread::spawn(move || loop {
        if let Some(task) = q.pop() { task.run(); }
    });
}
```

### 3.2. Shared-nothing per-shard scheduling

**What it is.** N worker shards, each owning a private MPMC queue (sharded for accept-side fan-in). The accept loop hashes incoming connections to a shard (round-robin, source-tuple hash, or least-loaded). A connection's lifetime is bound to its shard: parser, filter chain, router, response framing all run on the same shard's worker(s). No cross-shard task migration, no stealing.

**Why it's interesting.**
- **Predictable tail latency.** A request enters a shard and stays. The cost model is `time_in_my_shard's_queue + my_processing_time`. Easy to instrument, easy to explain to operators.
- **Cache locality.** The per-request arena, parser state, filter state — all stay on one shard's CPU caches. Per-request memory bandwidth drops materially.
- **No coherence traffic.** Atomics on the queue's head and tail are the only shared writes; everything else is shard-local.
- **Per-shard SLOs.** Each shard exposes its own queue depth, latency histogram, error rate. Operators can correlate "shard 3 is hot" with the source of the load and decide whether to rebalance the hash, add a shard, or scale out.
- **Friendly to the `Scheduler` trait.** Adding a `WorkStealingScheduler` later is a non-breaking change because the trait is already shaped around per-shard queues.
- **Friendly to thread-per-core in `v0.2`.** When [Options 002](002-async-runtime.md) revisits Tokio-multi-thread vs sharded current-thread, the per-shard scheduler is already the right shape.

**Where it falls short.**
- **Heterogeneous workloads suffer.** A few slow requests on one shard create a hotspot while other shards idle. Without stealing, this is just the price.
- **Hash-quality matters.** A bad hash (e.g. all clients behind one NAT hashed to one shard) creates a permanent hotspot. Mitigations: rendezvous hashing, source-port mixing, manual rebalance hooks.
- **More code than option 3.1.** Sharded queues + accept-side fan-in is real engineering, even if each piece is small.

**Real-world systems that use it.** Most production-shaped network servers in shared-nothing mode: Seastar / ScyllaDB, sharded nginx (one worker per CPU + `SO_REUSEPORT`), HAProxy with `nbproc`, vector data planes that pin per-core, Pingora's per-thread design.

### 3.3. Work-stealing scheduler (Chase-Lev deque per worker, FIFO steal)

**What it is.** Each worker owns a local Chase-Lev deque (LIFO at the owner's end, FIFO at the thief's end). Workers push and pop from their own end (cheap, no atomics on the fast path). When a worker's deque is empty, it picks another worker at random and steals from the FIFO end. The classic Cilk / TBB / Rayon design.

**Why it's interesting.**
- **Heterogeneous workloads love it.** Slow tasks pile up on the originating worker; fast workers drain their own queues, then come help. Tail latency improves on uneven mixes.
- **Cache-friendly fast path.** Owner-side LIFO push/pop hits the most-recently-touched task, which is most likely to be cache-warm.
- **Mature, well-studied.** Chase-Lev is a 2005 paper with a 2013 verified-correctness follow-up; the algorithm is settled.
- **Already inside Tokio.** Tokio's multi-thread runtime work-steals at the OS-thread level (per [Options 002](002-async-runtime.md)). Adding work-stealing at the request level mirrors the same shape one layer up.

**Where it falls short.**
- **Tail latency on small-task workloads is noisier.** Stealing pulls a task across CPUs; the stolen task's cache footprint moves with it. For sub-microsecond tasks, the steal cost dominates the work; for sub-millisecond tasks, it's a wash. For our HTTP-request granularity (typically tens to hundreds of microseconds for the gateway path), it's a win.
- **Hides load shape.** Because tasks migrate, it becomes harder to tell from queue-depth metrics alone where the actual load is. Operators have to look at per-worker steal rates and CPU time, not just queue depths.
- **Memory ordering is subtle.** Chase-Lev requires careful `Acquire`/`Release`/`SeqCst` placement; bugs are real and silent. `loom` testing on the implementation is mandatory ([`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md) Pitfalls).
- **Per-shard SLO enforcement is harder.** A tiered/priority model on top of a stealing scheduler needs explicit guards (e.g. "do not steal from the premium shard") which adds complexity.

**Real-world systems that use it.** Cilk (the canonical), TBB (Intel), Rayon (Rust data parallelism), Tokio (internal task scheduler), Go runtime (goroutine scheduler), Java ForkJoinPool.

### 3.4. Actor model (one actor per connection, mailbox per actor)

**What it is.** Every connection becomes an actor with an isolated state and a private mailbox. Workers pick the next actor with a non-empty mailbox and process one (or N) messages. State changes are local; cross-actor communication is by message passing. The Erlang/OTP, Akka, Pony shape.

**Why it's interesting.**
- **Strongest isolation.** A misbehaving actor cannot corrupt another actor's state.
- **Natural fit for very-many-very-small concurrent units** (millions of independent connections doing simple things).
- **Fault tolerance is built-in if you take the supervision-tree pattern seriously.**

**Where it falls short.**
- **Wrong shape for HTTP request lifecycle.** A request arrives, runs through parser → filter chain → router → upstream call → response framing. There's no meaningful intra-request message-passing; the actor model adds overhead for no gain.
- **Memory overhead per actor.** Each actor needs its own mailbox, supervisor link, and state. For millions of idle connections, this dwarfs the per-connection memory budget ([NFR-P04](../01-requirements/non-functional.md) targets <16 KB per idle connection).
- **Two scheduling layers.** The runtime schedules the actor mailbox-poller; the actor schedules its own internal state machine. Most of this scheduling overhead doesn't pay for itself in our workload.
- **Ecosystem mismatch.** Rust has actor crates (`actix`, `bastion`, `xtra`) but they are not the dominant pattern. Adopting them imports library risk for no clear payback.

**Real-world systems that use it.** Erlang/OTP (RabbitMQ, WhatsApp), Akka (Java/Scala services), Pony, some Rust services that wrap `actix` for legacy compatibility. Almost no high-throughput Rust gateways pick this shape.

## 4. Tradeoff matrix

| Property | Shared global queue | Per-shard (no stealing) | Work-stealing (per shard + steal) | Actor model | Why it matters |
|----------|---------------------|-------------------------|-----------------------------------|-------------|----------------|
| Linear scaling to 16 cores ([NFR-S03](../01-requirements/non-functional.md)) | poor (≥4 cores plateau) | good | very good | medium | The benchmark target won't be met by the shared model. |
| Tail latency on homogeneous workload | medium | good | good | medium | LLM gateway requests are roughly homogeneous in cost. |
| Tail latency on heterogeneous workload | poor | medium | very good | medium | Slow requests + fast requests is the realistic mix. |
| Cache locality (per-request arena, parser state) | poor | very good | good | poor | Memory-bandwidth tax matters at our QPS targets. |
| Predictability / explainability for operators | medium (one queue) | very good (per-shard metrics) | medium (load migrates) | poor (millions of actors) | At 3am on a pager this matters more than peak QPS. |
| Per-shard SLO enforcement (priority tiers in `v0.3`) | impossible | natural | possible with guards | natural | [FR-206](../01-requirements/functional.md) is gated on this. |
| Implementation complexity | low | medium | medium-high (Chase-Lev + `loom` tests) | high | One maintainer in `v0.x`. |
| Maturity in the Rust ecosystem | very high (every thread pool) | very high (sharded patterns common) | very high (Rayon, Tokio internal) | medium | We bet our default. |
| Compatibility with the `Scheduler` trait | natural | natural fit | natural fit | trait would need redesign | Pluggability is a Riftgate principle. |
| Compatibility with future thread-per-core runtime ([Options 002](002-async-runtime.md) `v0.2` retro) | poor | natural fit | possible | possible | Door must stay open. |
| Compatibility with future priority scheduling ([Options 022](README.md), if pursued) | poor | natural | possible with steal-guards | natural | Gated on `v0.2` retro. |

## 5. What the source-systems chapters say

`Ch7 (work stealing)` is the single most important reference here. The chapter walks through Cilk's scheduler, the Chase-Lev deque, and the empirical case for stealing on heterogeneous workloads. Two passages stand out:

1. **The fast-path / slow-path analysis.** Owner-side `push` and `pop` use only acquire-release atomics on the owner's deque (the fast path). Steal operations require an interlocked `cas` on the bottom pointer (the slow path). For workloads where stealing is rare, the cost is essentially free; for workloads where it is common, the cost is real and visible.
2. **The "first-touch" cache argument.** A task that runs on its originating CPU enjoys cache-warm data. A stolen task pulls its working set across the cache hierarchy. The chapter is explicit: **steal coarse-grained tasks; never steal sub-microsecond tasks.** Riftgate's request granularity (tens to hundreds of microseconds) is firmly in the "steal-friendly" range.

`Ch12 (system design patterns)` covers the bulkhead and shard patterns at the architecture level. The bulkhead pattern argues for *separate queues per failure domain* so that one slow tenant cannot starve others. Shared-nothing per-shard is the bulkhead pattern at the worker level. The chapter is also clear: **start with shared-nothing; add stealing or sharing only when measurements demand it.** This is the source of our staged approach (per-shard in `v0.1`, work-stealing as opt-in in `v0.2`).

`Ch4 (lock-free MPMC)` covers the queue substrate. The Vyukov MPMC bounded queue is the canonical Rust implementation in `crossbeam`; sharded MPMC is just N of them. The chapter makes the cache-line padding case explicitly: producer and consumer atomics must live on different cache lines or false sharing degrades the queue back toward mutex-level performance. Riftgate's `MpmcQueue<T>` impl will follow this guidance.

## 6. Recommendation

**`v0.1` ships a shared-nothing per-shard `Scheduler` impl (`PerShardScheduler`, sometimes still called `PerCoreScheduler` in design docs). `v0.2` adds a work-stealing impl (`WorkStealingScheduler`) behind a `--features work-stealing` cargo feature and a `scheduler = "work-stealing"` config setting. Reject shared-state worker pools and the actor model.**

The reasoning, restated:

- The `v0.1` workload is homogeneous-enough that work-stealing's main benefit (heterogeneous-mix tail latency) is small. Per-shard is simpler, more predictable, and easier to instrument.
- The `Scheduler` trait stays shaped around per-shard queues from day one, so adding stealing in `v0.2` is a new impl, not a redesign.
- Per-shard scheduling unlocks the future per-tenant SLO and priority-tier story ([FR-206](../01-requirements/functional.md), [Options 022](README.md)) without committing to it now.
- The relationship to [Options 002](002-async-runtime.md) is layered: Tokio multi-thread runs the actual futures across OS threads, while Riftgate's `Scheduler` distributes *requests* across logical shards. In `v0.1` we run M shards on N Tokio threads with no pinning; in `v0.2` we may revisit pinning + sharded current-thread Tokio per shard, which is the textbook thread-per-core deployment.

### Conditions under which we'd revisit

- The `v0.2` retro shows per-shard tail latency is materially worse than work-stealing on the realistic mix. (Likely outcome: we promote work-stealing to default for some deployment shapes.)
- A priority-tier requirement (FR-206) lands in `v0.3`; we may need to combine sharded queues with stealing-with-guards.
- Hash quality issues in production (one shard always hot) push us to a least-loaded routing or to stealing at the shard level.

### What stays available behind feature flags

- `WorkStealingScheduler` ships in `v0.2` as `--features work-stealing`. The default remains per-shard until at least the `v0.2` retro has data.
- `PriorityPerShardScheduler` (gated on [Options 022](README.md), gated on the `v0.2` retro) would extend the per-shard impl with tier-aware queues and steal-guards.

## 7. What we explicitly reject

- **Shared global queue as the default.** Lock contention or head-atomic coherence traffic plateaus scaling well before 16 cores. Reconsider only if a future profile shows our shard-routing decision is itself the bottleneck (which would be a different kind of problem).
- **Actor model.** Wrong cost model for HTTP-request-shaped work; per-actor memory overhead violates [NFR-P04](../01-requirements/non-functional.md). Reconsider only if Riftgate's workload mix shifts toward very-many-very-small persistent units (which is not on the roadmap).
- **Work-stealing as the `v0.1` default.** Premature optimization for a workload whose heterogeneity we have not yet measured. Reconsider after the `v0.2` retro produces real data.
- **Auto-detect best scheduler at runtime.** Same posture as [Options 001 §7](001-io-model.md): users should know what scheduling shape they are running.

## 8. References

1. Robert D. Blumofe and Charles E. Leiserson, *Scheduling Multithreaded Computations by Work Stealing* (FOCS 1994). https://dl.acm.org/doi/10.5555/795662.796339
2. David Chase and Yossi Lev, *Dynamic Circular Work-Stealing Deque* (SPAA 2005). https://dl.acm.org/doi/10.1145/1073970.1073974
3. Nhat Minh Lê, Antoniu Pop, Albert Cohen, Francesco Zappa Nardelli, *Correct and Efficient Work-Stealing for Weak Memory Models* (PPoPP 2013). https://dl.acm.org/doi/10.1145/2442516.2442524
4. crossbeam-deque crate (Rust impl of Chase-Lev) — https://docs.rs/crossbeam-deque
5. Carl Lerche, *Making the Tokio scheduler 10x faster* — https://tokio.rs/blog/2019-10-scheduler
6. Avi Kivity, *Seastar: a high-performance shared-nothing framework* (ScyllaDB) — https://seastar.io/
7. Andrew Hunt, *The pragmatic case for shared-nothing in production servers* (collected blog discussion).
8. Riftgate source-systems chapter `Ch7 (work stealing)`
9. Riftgate source-systems chapter `Ch12 (system design patterns)`
10. Riftgate source-systems chapter `Ch4 (lock-free MPMC)`
