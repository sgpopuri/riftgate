# 004. Request Queue

> **Status:** `recommended` — sharded MPMC queue strategy fixed by trait in `v0.1` (backed by `crossbeam-channel`); hand-rolled Vyukov MPMC and `loom`-verified sharded variant land in `v0.2` per [FR-106](../01-requirements/functional.md). See [ADR 0005](../06-adrs/0005-sharded-mpmc-queue.md).
> **Foundational topics:** lock-free MPMC queues (Vyukov bounded), bulkhead pattern / per-shard isolation, queue-as-circuit-breaker
> **Related options:** [001](001-io-model.md) (IO model), [002](002-async-runtime.md) (async runtime), [003](003-concurrency-model.md) (concurrency model)
> **Related ADR:** [ADR 0005](../06-adrs/0005-sharded-mpmc-queue.md)

## 1. The decision in one sentence

> What queue strategy hands off requests from the accept loop to worker shards, what concrete `Queue<T>` impl ships in `v0.1`, and what hand-rolled impl supersedes it in `v0.2`?

## 2. Context — what forces this decision

[Options 003](003-concurrency-model.md) decided that `v0.1` ships a `PerShardScheduler` with N worker shards, each owning a private queue. This Options doc decides *what kind* of queue that is and *how it is implemented*.

Forces driving this decision:

- **Linear scaling to 16 cores** ([NFR-S03](../01-requirements/non-functional.md)). A queue that serializes producers, consumers, or both will not meet this.
- **Bounded memory per queue.** Unbounded queues defer backpressure to the OS killer; bounded queues let us implement [NFR-R03](../01-requirements/non-functional.md) (graceful 503s under overload, see also [FR-104](../01-requirements/functional.md)).
- **Lock-free producer and consumer.** [`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md) names a Vyukov MPMC as the `v0.2` impl. The `v0.1` shape needs to be Vyukov-compatible at the trait level so the `v0.2` swap is non-breaking.
- **`loom`-testable correctness.** [`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md) Pitfalls calls out memory-ordering bugs as the most common cause of silent corruption in this code. The hand-rolled impl must be `loom`-tested ([Standards and review gates](../04-design/lld-scheduling.md)).
- **Engineering capacity.** A hand-rolled, `loom`-verified MPMC is a multi-week project. `v0.1` is markdown + walking-skeleton work; we should not block on writing one.
- **Trait pluggability.** The `Queue<T>` trait shape ([`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md)) must accommodate at minimum: bounded MPMC, sharded MPMC, and (later) priority-aware variants from [Options 022](README.md) if pursued.

The decision is consequential because the queue is on every request's hot path. Even small per-operation overheads multiply by the QPS targets in [NFR-P03](../01-requirements/non-functional.md).

## 3. Candidates

### 3.1. `Mutex<VecDeque<T>>` (locked queue)

**What it is.** A `std::collections::VecDeque<T>` wrapped in a mutex (`std::sync::Mutex` or `parking_lot::Mutex`), with `Condvar`-based wakeups for blocking semantics. The simplest queue you can write that is FIFO, bounded, and thread-safe.

**Why it's interesting.**
- Trivially correct. No memory-ordering bugs because the mutex provides total ordering.
- Easy to reason about. Easy to instrument.
- Fast on single-producer, single-consumer paths and on low contention.

**Where it falls short.**
- **Lock contention scales poorly.** Every push, every pop, and every length check serializes on the same mutex. Past 4 producers/consumers on a hot queue, throughput plateaus and tail latency grows.
- **Priority inversion is real.** A worker holding the mutex while it is descheduled blocks every other worker. `parking_lot`'s eventual fairness mode helps but does not eliminate the problem.
- **No way to express "lock-free fast path."** Even uncontended access pays the mutex cost (typically tens of nanoseconds per op, dominated by atomic exchange).
- **Awkward fit with the per-shard model.** Sharded mutexes work — but at that point the lock is just paying for the fact that we did not write a lock-free queue.

**Real-world systems that use it.** Many small Rust services, prototype code, queue-as-an-afterthought designs. Almost no high-throughput servers use this in production hot paths.

### 3.2. Lock-free MPMC (Vyukov bounded queue)

**What it is.** A bounded multi-producer multi-consumer queue using per-slot sequence numbers. Each slot carries a `seq: AtomicUsize`; producers `cas` to claim a slot when `seq == enqueue_pos`, consumers `cas` to claim when `seq == dequeue_pos + 1`. The data structure has no internal locks; backpressure is signaled by `push` returning `Err(value)` when full.

**Why it's interesting.**
- The canonical lock-free MPMC. Designed by Dmitry Vyukov, used in `crossbeam-queue`, `tokio::sync::mpsc` (with adaptations), Boost.Lockfree, and many other production systems.
- Per-operation cost is one `cas` plus one read on the fast path. Sub-100 ns per op on modern x86.
- Bounded: enables clean backpressure semantics ([FR-104](../01-requirements/functional.md), [NFR-R03](../01-requirements/non-functional.md)).
- Cache-friendly when the slots are padded: producers and consumers each touch their own pos-counter, plus their slot's seq.
- Already implemented in mature crates (`crossbeam-queue::ArrayQueue` is essentially Vyukov; `crossbeam-channel::bounded` builds blocking semantics on top).

**Where it falls short.**
- **Hand-rolling is a real engineering project.** Memory-ordering correctness requires `loom` testing; the `seq` arithmetic is subtle. We do not need to hand-roll for `v0.1`; we will for `v0.2` per [`docs/04-design/lld-scheduling.md`](../04-design/lld-scheduling.md) so we own the instrumentation hooks.
- **Bounded means the operator must size it.** Too small → 503s under burst; too large → memory bloat and head-of-line blocking. Default sizing must be conservative + tunable.
- **Single shared queue is still a hot point.** Even lock-free, the producer atomic on the head is contended at very high QPS. Sharding fixes this (3.4 below).

**Real-world systems that use it.** crossbeam, Disruptor (LMAX, Java), Folly (Facebook), most C++ HFT systems, every modern lock-free queue tutorial since 2013.

**Code sketch (Vyukov bounded MPMC, simplified).**
```rust
struct Slot<T> { seq: AtomicUsize, val: UnsafeCell<MaybeUninit<T>> }
struct MpmcQueue<T> { slots: Box<[Slot<T>]>, head: AtomicUsize, tail: AtomicUsize, cap: usize }

impl<T> MpmcQueue<T> {
    fn push(&self, v: T) -> Result<(), T> {
        let mut pos = self.tail.load(Acquire);
        loop {
            let slot = &self.slots[pos % self.cap];
            let seq = slot.seq.load(Acquire);
            match seq.cmp(&pos) {
                Equal => match self.tail.compare_exchange_weak(pos, pos + 1, Relaxed, Relaxed) {
                    Ok(_)  => { unsafe { (*slot.val.get()).write(v); }
                                slot.seq.store(pos + 1, Release);
                                return Ok(()); }
                    Err(p) => pos = p,
                },
                Less    => return Err(v),
                Greater => pos = self.tail.load(Acquire),
            }
        }
    }
}
```

### 3.3. SPSC ring (single producer, single consumer per pair)

**What it is.** A ring buffer where exactly one thread pushes and exactly one thread pops, separated by `Acquire`/`Release` atomics on the head and tail. The simplest possible lock-free queue. To use SPSC across N producers + M consumers, you need N×M (or N + M) of them plus a routing layer.

**Why it's interesting.**
- The fastest queue you can build. No `cas`, just plain atomic load/store on the fast path. Pure relaxed-on-the-data path with acquire/release fences only on head/tail.
- Latency floor in the low tens of nanoseconds per op.
- Cache-line padded properly, the producer and consumer never share a dirty cache line.
- LMAX Disruptor's heritage.

**Where it falls short.**
- **Topology-fixed.** SPSC is for one producer talking to one consumer. To go many-to-many you need to compose them (one SPSC per producer/consumer pair) plus a routing layer that selects which queue to push to. The composition complexity quickly exceeds Vyukov MPMC's complexity.
- **Brittle to producer/consumer topology changes.** Adding a worker means rebuilding the topology. Riftgate's worker count is configurable per deployment; SPSC topology means we recompute the routing fabric on every config change.
- **Hard to express "any worker can take this" semantics** without doing exactly what MPMC does — at which point we have rebuilt MPMC out of SPSC primitives.

**Real-world systems that use it.** LMAX Disruptor (with a sequencer barrier on top to coordinate multiple producers/consumers), DPDK rings, low-level driver code. Almost never used directly as the application-level work queue.

### 3.4. Sharded MPMC (one MPMC per worker shard + accept-side fan-in)

**What it is.** N MPMC queues, one per worker shard. The accept loop selects a shard for each new connection and pushes to that shard's queue. Workers in a shard pop from their own queue. The `PerShardScheduler` from [Options 003](003-concurrency-model.md) is shaped exactly for this.

**Why it's interesting.**
- **No queue is hot for very long.** Producers fan out across N shards; each shard's queue sees ~1/N of the global QPS, which keeps contention low on each individual queue.
- **Backpressure is per-shard.** A hot shard's queue fills up and that shard returns 503s; other shards keep flowing. This is the bulkhead pattern at the queue layer.
- **Naturally composes with future work-stealing.** Each shard's queue is the "owner deque" for the work-stealing scheduler in `v0.2`; the steal protocol pops from the FIFO end of the *other* shard's queue.
- **Per-shard metrics fall out.** `riftgate_shard_queue_depth{shard="N"}` is just `queue.len()` per shard.
- **Clean composition with priority-tier scheduling later.** The "premium tier shard" simply has its own queue; routing decides which shard a given priority lands in.

**Where it falls short.**
- **Hash quality is an operational concern.** A bad hash creates a permanent hotspot. Mitigations: round-robin (default), source-tuple hash with mixing, or least-loaded.
- **More memory than a single queue.** N queues of capacity K each vs one queue of capacity N×K. In practice the difference is small because queue slots are tiny (16-32 bytes for a request handle).
- **Marginally more complex to write.** The fan-in routing function and the per-shard size-tuning are extra surface.

**Real-world systems that use it.** Seastar/ScyllaDB (per-CPU queues), nginx with `SO_REUSEPORT` (kernel does the sharding), Pingora (Cloudflare), every production-shaped server that wants per-CPU isolation.

## 4. Tradeoff matrix

| Property | `Mutex<VecDeque>` | Single Vyukov MPMC | SPSC + routing | Sharded MPMC | Why it matters |
|----------|-------------------|--------------------|----------------|--------------|----------------|
| Producer scaling | poor (lock) | good (one `cas`) | very good | very good | Accept loop is single-producer to a shard; scaled across shards. |
| Consumer scaling | poor (lock) | good | very good (1:1) | very good | Workers within a shard contend only locally. |
| Per-op latency (uncontended) | ~30-80 ns | ~50-100 ns | ~10-30 ns | ~50-100 ns | At 1k QPS this is in the noise; at 100k QPS per shard it matters. |
| Per-op latency (contended) | grows fast | flat (small N) | flat | flat (per-shard) | Tail latency under burst. |
| Bounded backpressure | yes (with explicit cap) | yes (capacity at construct) | yes | yes | [FR-104](../01-requirements/functional.md). |
| Per-shard isolation | no | no | yes (manual) | yes (natural) | [NFR-OBS02](../01-requirements/non-functional.md) per-shard metrics. |
| `loom`-testability | trivial | hard (we test the impl, not the mutex) | medium | hard (test the cell impl + sharding wiring) | Memory-ordering bugs are silent. |
| Available off-the-shelf | yes (`std::sync::Mutex`) | yes (`crossbeam_channel::bounded`) | yes (`rtrb`, `spsc-buffer`) | yes (compose `crossbeam_channel`) | `v0.1` should not write its own. |
| Compatibility with `Queue<T>` trait | natural | natural | trait may need a routing helper | natural | Pluggability. |
| Compatibility with future work-stealing | poor | medium | n/a | natural | [FR-107](../01-requirements/functional.md) `v0.2`. |
| Compatibility with future priority tiers ([Options 022](README.md), if pursued) | poor | hard | hard | natural (per-tier shards) | Gated, but door must stay open. |
| Engineering cost in `v0.1` | trivial | low (use `crossbeam-channel`) | medium | low (use `crossbeam-channel` per shard) | We have one maintainer. |

## 5. Foundational principles

**Lock-free MPMC queues (Vyukov bounded).** Three takeaways from the lock-free-queue literature shape the choice:

1. **Vyukov bounded MPMC is the canonical design.** The per-slot sequence-number protocol and the FIFO + lock-freedom proof are well-documented in Vyukov's 1024cores writeup. The implementation in `crossbeam-queue` is faithful to this; Riftgate will follow the same design when we hand-roll in `v0.2`.
2. **Cache-line padding is mandatory.** Producer-side and consumer-side counters must live on different cache lines, or false sharing degrades the queue back toward mutex-level performance. `#[repr(align(64))]` (or `crossbeam_utils::CachePadded`) on the relevant atomics is non-negotiable. McKenney's *Is Parallel Programming Hard?* is the standard reference for the underlying microarchitecture.
3. **Bounded vs unbounded is a backpressure decision, not a memory decision.** Unbounded queues mean your kernel decides when to OOM; bounded queues mean your application decides when to shed load. Riftgate is in the second camp on principle.

**Bulkhead pattern and per-shard isolation.** Nygard's *Release It* and the broader resilience-patterns literature offer one prescriptive insight: the unit of failure isolation should be the same as the unit of resource allocation. For Riftgate, that unit is the worker shard. Each shard owning its own queue means queue-fullness fails closed for that shard only; other shards keep serving. A single shared queue makes the entire data plane the failure unit.

**Queue-as-circuit-breaker.** A full queue is itself a backpressure signal. The application can choose to return 503, to delay, or to spill to a degraded path; the queue does not need to be smart, but it needs to be observable. Sharded queues give us per-shard observability for free.

## 6. Recommendation

**`v0.1` ships a `Queue<T>` trait with a `CrossbeamQueue<T>` default impl wrapping `crossbeam_channel::bounded`. The `PerShardScheduler` from [Options 003](003-concurrency-model.md) instantiates one `CrossbeamQueue<T>` per shard. The accept loop fans in via round-robin (configurable to source-tuple hash). `v0.2` adds in-tree `MpmcQueue<T>` (Vyukov, hand-rolled, `loom`-tested) and `ShardedMpmcQueue<T>` impls per [FR-106](../01-requirements/functional.md), which become the new default.**

The reasoning, restated:

- The architectural answer is sharded MPMC. The implementation answer for `v0.1` is "use a battle-tested crate"; we will hand-roll for `v0.2` to own instrumentation hooks (per-slot seq inspection, custom metrics) and to control memory-ordering choices.
- `crossbeam-channel::bounded` is essentially Vyukov MPMC with sender/receiver wrappers and `Condvar`-style blocking semantics. It is mature, fast, well-tested, and meets the trait shape.
- The hand-roll in `v0.2` is justified by the project's "we own the substrate" posture: hand-rolled MPMC is a textbook teaching artifact, fits the documentation-first pillar, and lets us add features (e.g. per-slot metrics) without forking `crossbeam-channel`.
- The trait shape is the only abstraction that crosses crate boundaries. Whether `v0.1` uses `crossbeam-channel` or `v0.2` uses our own MPMC, consumers of `riftgate-core::Queue<T>` see no difference.

### Conditions under which we'd revisit

- The hand-rolled `v0.2` impl shows worse benchmarks than `crossbeam-channel`. We would keep `crossbeam-channel` as the default and the hand-roll as a documented teaching artifact.
- Per-shard hash quality issues in production motivate a different routing strategy (e.g. rendezvous hashing, least-loaded). Routing is decided at the `PerShardScheduler` level, not at the queue level, so this is a follow-on Options doc rather than a revisit of this one.
- Priority tiers ([FR-206](../01-requirements/functional.md), [Options 022](README.md)) land. The trait may need a `priority: Tier` parameter on `push` for tier-aware routing. We will revise the trait then if needed.

### What stays available behind feature flags

- `v0.2` `MpmcQueue<T>` and `ShardedMpmcQueue<T>` ship behind `--features riftgate-mpmc`. Default in `v0.2+` flips to the in-tree impl after the conformance + benchmark suite passes.
- Future priority-tier impls (gated on [Options 022](README.md)) ship behind their own feature flag.

## 7. What we explicitly reject

- **`Mutex<VecDeque>` as the production default.** Lock contention scales poorly past a few cores; per-shard mutexes are a lateral move that pays for the lock without buying anything Vyukov MPMC doesn't already give us.
- **Naked SPSC + manual routing.** Topology fragility, no real win over MPMC for our request shape, and the routing layer ends up reimplementing what MPMC does internally.
- **Unbounded queues.** They convert backpressure into OOM; explicit prohibition by [NFR-R03](../01-requirements/non-functional.md).
- **Async-aware queues from `tokio::sync::mpsc` as the default cross-thread queue.** `tokio::sync::mpsc::channel` is excellent for *task-to-task* communication within Tokio, but the per-shard accept-to-worker handoff is a thread-to-thread pattern (the accept loop is a Tokio task, but the workers may be on different runtime threads under future thread-per-core). `crossbeam-channel` is the right tool for that scope.
- **Custom MPMC for `v0.1`.** Engineering cost without payoff; we ship the trait and use a mature crate, then hand-roll in `v0.2`.

## 8. References

1. Dmitry Vyukov, *Bounded MPMC queue* — https://www.1024cores.net/home/lock-free-algorithms/queues/bounded-mpmc-queue
2. crossbeam-channel — https://docs.rs/crossbeam-channel
3. crossbeam-queue (`ArrayQueue`) — https://docs.rs/crossbeam-queue
4. LMAX Disruptor — https://lmax-exchange.github.io/disruptor/
5. Folly MPMCQueue (Facebook) — https://github.com/facebook/folly/blob/main/folly/MPMCQueue.h
6. Maged M. Michael and Michael L. Scott, *Simple, Fast, and Practical Non-Blocking and Blocking Concurrent Queue Algorithms* (PODC 1996).
7. Maged M. Michael, *Hazard Pointers: Safe Memory Reclamation for Lock-Free Objects* (IEEE TPDS 2004).
8. The `loom` crate (concurrency permutation tester) — https://docs.rs/loom
9. Paul E. McKenney, *Is Parallel Programming Hard, And, If So, What Can You Do About It?* — https://mirrors.edge.kernel.org/pub/linux/kernel/people/paulmck/perfbook/perfbook.html
10. Michael Nygard, *Release It! Design and Deploy Production-Ready Software* (2nd ed., 2018) — bulkhead pattern.
