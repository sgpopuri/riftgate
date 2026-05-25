# 006. Timer subsystem

> **Status:** `recommended` — `BinaryHeapTimers` (a `std::collections::BinaryHeap`-backed min-heap with lazy cancellation) in `v0.1`; `HierarchicalWheel` (Varghese & Lauck, SOSP 1987) in `v0.2` behind the same `TimerSubsystem` trait. See [ADR `0010`](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md).
> **Foundational topics:** hashed and hierarchical timing wheels, binary and `d`-ary heaps, OS timer interfaces (`timerfd_create(2)`, `kevent` `EVFILT_TIMER`), monotonic clocks (`clock_gettime(CLOCK_MONOTONIC)`), low-level synchronization primitives (`futex`, mutex, condvar)
> **Related options:** [`001`](001-io-model.md) (IO model — `tick` is driven from the same per-shard event loop as `poll`), [`003`](003-concurrency-model.md) (concurrency model — timers are per-shard), [`004`](004-request-queue.md) (request queue — cross-shard timer dispatch borrows the same MPMC)
> **Related ADR:** [ADR `0010`](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md)

## 1. The decision in one sentence

> Which data structure does Riftgate use to track per-request deadlines (request-overall, upstream-call, idle-stream) at the scale of 100k+ concurrent timers, without paying super-linear cost on insert, cancel, or per-tick processing?

## 2. Context — what forces this decision

Riftgate has three distinct deadline classes that must be enforced on every request:

- **Request-overall** — the top-level "this request must complete or fail in N seconds" deadline. One timer per request.
- **Upstream-call** — the deadline on each individual hop to a backend (the connect timeout, the headers-received timeout, the body-received timeout). Up to a handful per request.
- **Idle-stream** — for streaming responses, the inter-token idle timeout. Re-armed on every token; cancelled on stream close.

The combined timer rate at our `v0.2` scaling targets ([NFR-S01](../01-requirements/non-functional.md): ≥50k concurrent connections; [NFR-S02](../01-requirements/non-functional.md): ≥10k concurrent in-flight streaming requests) sits in the high tens of thousands of *live* timers, with a *churn* rate (insert + cancel + fire) of several hundred thousand per second when the idle-stream re-arm path is included. This rules out structures whose insert / cancel cost is super-linear, or whose tick cost scans every live timer.

Three forces drive the design:

- **Cancellation is the dominant operation.** Streaming requests re-arm their idle timer on every token; the *previous* timer is cancelled and a new one is inserted. A timer subsystem that makes cancellation cheap (O(1) given a handle) is dramatically better than one that does not. This is the signature shape of the workload that motivated Varghese & Lauck's wheel paper in the first place.
- **Tick processing must not scan all live timers.** A naive "every 10 ms, walk every timer and check `now >= deadline`" structure is O(n) per tick, which is the failure mode [`FR-008`](../01-requirements/functional.md) is written to prevent. Acceptable shapes are those whose tick cost is proportional to the number of *expired* timers, not the number of *live* timers.
- **The trait is the abstraction boundary.** [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md) defines `TimerSubsystem` with `schedule`, `cancel`, and `tick`. The trait must accommodate both a heap (where insert/cancel are O(log n)) and a wheel (where insert/cancel are O(1)) without callers caring which one is wired up. This is the same pluggability discipline as [Options `001`](001-io-model.md) and [Options `005`](005-allocator.md): one trait, multiple impls, one default.

A secondary force: per-tick precision is bounded. We do not need sub-millisecond accuracy for request-deadline use; a 10 ms tick resolution is plenty (the resulting "5-second timeout fires between 5.000s and 5.010s" jitter is well below any SLO an LLM gateway exposes). This relaxes the design considerably — coarse-tick structures (heap walks at 10 ms cadence; wheels with 10 ms slots) all qualify, where a sub-millisecond requirement would push us toward `timerfd` per timer or a hardware-clock-driven structure.

The decision is consequential because the timer subsystem is touched on every request and on every streamed token. A wrong choice in `v0.1` would propagate cost into every later subsystem that uses deadlines (rate limiter, circuit breaker, hedged request cancellation, capability broker audit timeouts).

## 3. Candidates

We evaluate five candidates spanning the spectrum from "literally a `std::collections::BinaryHeap`" to "delegate to the runtime."

### 3.1. Binary heap (`std::collections::BinaryHeap` with lazy cancellation)

**What it is.** A min-heap keyed by deadline. Each scheduled timer is pushed onto the heap with `(deadline, timer_id)`. `tick(now)` peeks the top; while the top's deadline is `<= now`, pop it and (if not lazy-cancelled) fire it. Cancellation is implemented by inserting `timer_id` into a per-shard `HashSet<TimerId>` (the *cancelled* set) and rechecking on pop — the heap entry is left in place to avoid the O(n) scan that direct removal would cost.

State per timer: one heap entry (24 bytes for `(Instant, u64)` on a 64-bit target plus heap overhead) plus, in the worst case, one `HashSet` entry on cancellation.

**Why it's interesting.**
- Trivial. `std::collections::BinaryHeap` is in the standard library, well-tested, well-documented.
- O(log n) insert; O(log n) pop; O(1) push-into-cancelled-set (which is the *real* cancel cost on the hot path).
- Tick processes only expired entries — O(k log n) where `k` is the number of expirations this tick, NOT O(n) over live timers. This satisfies the FR-008 acceptance ("100k concurrent timers cost less than O(n) per tick"). The constant is small and entirely cache-friendly because the heap is array-backed.
- Lazy cancellation is a well-understood pattern (Linux's `epoll_wait` does an analog with the ready list; many event-loop implementations use it for timer cancellation specifically).
- Memory cost is bounded — the cancelled set never exceeds the number of cancelled-but-not-yet-popped entries, and stale entries are dropped at pop time.
- Zero unsafe code. Zero hand-rolled data structures. Zero opportunity for the kind of bug that hides in a wheel cascade.

**Where it falls short.**
- **Insert and pop are O(log n).** At 100k live timers, `log n` is ~17 — a few hundred nanoseconds at most. At 10M live timers (well outside any v0.x target), `log n` is ~24 — still tractable, but we are paying a real per-operation cost the wheel does not pay.
- **Cancellation grows the cancelled set.** A pathological pattern where many timers are cancelled and few are popped lets the set grow. Periodic compaction (drain the cancelled set when its size exceeds a fraction of the heap) keeps this bounded; an explicit operator-visible metric (`riftgate_timers_cancelled_pending`) makes it observable.
- **No "schedule far in the future without paying for it" trick.** The heap is one-tier; an entry scheduled 10 minutes out occupies its slot the entire time, where a hierarchical wheel would cascade it down only as the moment approached. Practically irrelevant for our workload (request deadlines are bounded by the request timeout in seconds, not minutes), but worth naming.

**Real-world systems that use it.** Tokio's classic timer driver used a hierarchical wheel; many simpler async runtimes (`async-std`, the Erlang OTP timer wheel inside `erlang:send_after`) use heap variants. The Linux kernel's `hrtimers` uses an rb-tree (logarithmic insert/cancel for the same reason) before being augmented with a wheel-style fast path for low-precision timers; this is the same shape of decision.

**Sketch.**
```rust
pub struct BinaryHeapTimers {
    heap: BinaryHeap<Reverse<(Instant, TimerId)>>,
    callbacks: HashMap<TimerId, Box<dyn FnOnce() + Send>>,
    cancelled: HashSet<TimerId>,
    next_id: u64,
}

impl TimerSubsystem for BinaryHeapTimers {
    fn schedule(&mut self, deadline: Instant, on_fire: Box<dyn FnOnce() + Send>) -> TimerHandle {
        let id = self.next_id; self.next_id += 1;
        self.heap.push(Reverse((deadline, id)));
        self.callbacks.insert(id, on_fire);
        TimerHandle(id)
    }
    fn cancel(&mut self, handle: TimerHandle) -> bool {
        self.callbacks.remove(&handle.0).is_some() && self.cancelled.insert(handle.0)
    }
    fn tick(&mut self, now: Instant) {
        while let Some(Reverse((deadline, _))) = self.heap.peek() {
            if *deadline > now { break; }
            let Reverse((_, id)) = self.heap.pop().unwrap();
            if self.cancelled.remove(&id) { continue; }
            if let Some(cb) = self.callbacks.remove(&id) { cb(); }
        }
    }
}
```

### 3.2. Hashed timing wheel (single-level)

**What it is.** An array of `W` buckets indexed by `slot = (deadline_ticks) mod W`, each bucket a doubly-linked list of timers. `schedule(deadline, ...)` computes the slot and pushes to its list (O(1)); `cancel(handle)` unlinks the timer from its bucket (O(1) given the handle). `tick(now)` advances the current-slot pointer and fires every timer in that one bucket. For deadlines beyond `W` ticks, the bucket also stores a "rounds remaining" counter that is decremented on each rotation; the timer fires only when the counter reaches zero.

Originally introduced by Varghese & Lauck (SOSP 1987) as the "simple timing wheel." `W` is typically chosen as a power of two for cheap modular arithmetic.

**Why it's interesting.**
- O(1) insert. O(1) cancel given the handle. O(slot-list-length) per tick.
- The structure is small and cache-friendly: a flat array of bucket heads, each a doubly-linked list of nodes. No tree, no heap, no rebalancing.
- The "rounds remaining" trick lets a single-level wheel handle deadlines arbitrarily far in the future; the cost is that a far-future timer is *visited* once per rotation (decrementing its counter), which is wasted work compared to the hierarchical variant.

**Where it falls short.**
- **The "rounds remaining" decrement is per-rotation work.** For `W = 512` slots at 10 ms tick (5.12-second wheel rotation), a 1-hour deadline visits the timer ~700 times before firing. At our timer count this is a real CPU cost that the hierarchical variant avoids by cascading.
- **Slot-list length is unbounded in the worst case.** A pathological pattern where many timers expire on the same tick produces an arbitrarily long list to walk; the LLD's "tick processing time at peak should be <100 µs" budget can blow under skewed loads.
- **`W` sizing is a tradeoff with no obvious right answer.** Small `W` means more rotations and more rounds-remaining work; large `W` means more memory for the slot array and worse cache behavior on a sparse wheel.

**Real-world systems that use it.** The Linux kernel's classic `timer_wheel` (pre-`hrtimers`); many embedded systems' timer subsystems; some game-loop event timers.

### 3.3. Hierarchical timing wheel (multi-level)

**What it is.** Varghese & Lauck's full proposal: a stack of wheels, each coarser than the one below, with a *cascade* operation that demotes timers from a coarse wheel to a finer one as their deadlines approach. The lowest wheel has tick-resolution slots (e.g. 10 ms); the next has slot-width = lowest-wheel-rotation (e.g. 5.12 s); the next has slot-width = second-wheel-rotation; and so on. A 1-hour deadline lives in (say) wheel 3 until its 5-minute mark, when it cascades to wheel 2; until its 5-second mark, when it cascades to wheel 1; until its 10-ms tick, when it fires.

State per timer: a single doubly-linked-list node, but the node's bucket changes on each cascade.

**Why it's interesting.**
- O(1) amortized insert, cancel, and tick (when the slot list length is bounded).
- Far-future timers do *not* pay per-rotation work: they sit in a coarse-wheel bucket and are visited only when their cascade boundary is crossed.
- The structure scales naturally to very large timer counts and very long deadline ranges. Linux's modern `timer_wheel` (post-rework circa 2016) is essentially this shape.
- This is the right answer for a steady-state production system at our scale targets.

**Where it falls short.**
- **Cascade cost is real and bursty.** When the second hand of a coarse wheel ticks, every timer in that bucket is moved down one wheel — a burst of work proportional to the bucket's population. This is amortized O(1) but can produce noisy tick latency at wheel-boundary crossings.
- **Implementation complexity is high.** Multi-level cascade, bucket-list management, careful ordering of "what fires before what cascades," edge cases at boot (clock jumps, suspended processes resuming) — a wheel implementation that is wrong in any of these places is silently wrong (a timer fires late or never).
- **Hand-rolled or vendor-in.** No suitable Rust crate ships a hierarchical wheel with a stable public API the way `std::collections::BinaryHeap` ships a heap. Tokio's timer driver implements one but is an internal type. We would either vendor in code from `tokio-util` or write our own.
- **Debugging is harder.** A "timer fired wrong" bug in a wheel is a "what was the cascade ordering at this moment?" question; the same bug in a heap is "what was at the top of the heap?" — much easier to print and stare at.

**Real-world systems that use it.** Linux kernel `timer_wheel` (modern), Tokio's internal timer driver, Cassandra's hinted-handoff timers, Netty's `HashedWheelTimer`, Kafka's `TimingWheel`.

### 3.4. OS timer interfaces (`timerfd` + epoll, `kevent` `EVFILT_TIMER`)

**What it is.** Delegate every individual timer to the kernel: one `timerfd_create(2)` (Linux) or one `kevent(EVFILT_TIMER)` (BSD/macOS) per timer, registered with the same epoll/kqueue instance the IO subsystem uses. Firing happens via the regular event loop — when the kernel says "this timerfd is readable," the event loop reads the expiration count and runs the callback.

A coalescing variant: maintain one `timerfd` per shard, set to the next imminent deadline; when a closer deadline is scheduled, re-arm the `timerfd`; when it fires, walk a per-shard sorted structure (heap or wheel) to find expirations.

**Why it's interesting.**
- Sub-millisecond precision when configured for it (`hrtimer`-backed on Linux). For request-deadline use this is overkill; for some hypothetical "trade-execution-grade timer" use it would matter.
- The kernel manages the firing — userland just reacts. No tick loop in our code.
- The coalescing variant is genuinely small: one fd per shard, one callback hook in the event loop, plus whatever per-shard sorted structure (heap/wheel) we already have.

**Where it falls short.**
- **One `timerfd` per timer is wrong at our scale.** At 100k live timers, that is 100k file descriptors per process, plus the per-fd kernel state. The fd-table cost alone (`/proc/sys/fs/file-max`, `ulimit`) makes this impractical.
- **The coalescing variant *is* a heap or wheel.** Once we are maintaining a sorted structure per shard and using `timerfd` only as the "next deadline" wakeup, we have re-derived candidate 3.1 or 3.3 with an extra fd in the mix. The fd is an awkward fifth wheel — it gives us an event-loop wakeup we already get from the runtime's own timer driver.
- **Portability cost.** `timerfd` is Linux-only; `kevent` `EVFILT_TIMER` is the macOS/BSD analog with subtly different semantics. The kqueue variant cannot represent very-far-future deadlines without re-arming. Bridging the two cleanly inside the `TimerSubsystem` trait costs us code that buys no measurable accuracy improvement for our workload.

**Real-world systems that use it.** Single-fd-per-timer is rare at scale; coalesced `timerfd` + heap is what Tokio's I/O driver historically did to wake the runtime when the next-deadline came due (orthogonal to our trait choice — we care about the structure that *holds* the timers, not the wakeup mechanism).

### 3.5. Delegate to Tokio's built-in timer driver

**What it is.** Don't write a `TimerSubsystem` impl at all; use `tokio::time::Sleep` (a future) and `tokio::time::Instant` directly inside the request handlers. Tokio's runtime has a built-in timer driver (a hierarchical wheel internally) shared across all worker threads.

**Why it's interesting.**
- Zero code. The async runtime already manages timers. Every `tokio::time::sleep(...)` becomes a scheduled timer in Tokio's internal wheel.
- The hierarchical-wheel benefits of candidate 3.3 are inherited "for free" — Tokio's implementation is mature and battle-tested.
- The most idiomatic shape for async Rust code.

**Where it falls short.**
- **Tokio's timer driver is an internal type.** It does not implement a public trait we can substitute. Choosing this candidate means *not* having a `TimerSubsystem` trait at all — which violates [Vision §3](../00-vision.md)'s pluggability principle and [`AGENTS.md` §5](../../AGENTS.md) ("no silent change to public traits"; the trait surface is the kernel's contract).
- **No conformance suite.** Future contributors who want to swap the timer subsystem (for a hardware-clock-driven version, for an instrumentation hook, for a deterministic-test-mode driver) have nowhere to plug in.
- **Coupling.** Every place that needs a timer is now coupled to `tokio::time::*`. If the project ever moves off Tokio (which [ADR `0003`](../06-adrs/0003-tokio-multithread-default.md) leaves as a future possibility at the `v0.2` retro), every timer call site is an edit.
- **Cancellation semantics differ.** Dropping a `tokio::time::Sleep` future cancels it implicitly; our trait expresses cancellation as an explicit `cancel(handle)` call so that non-future-shaped call sites (the per-shard worker loop) can manage timers without spawning tasks.

**Real-world systems that use it.** Almost every Tokio-based application that doesn't have specific timer-subsystem requirements. The right default for code that doesn't care about the shape; the wrong default for a pluggable kernel that does.

## 4. Tradeoff matrix

| Property | Binary heap (3.1) | Hashed wheel (3.2) | Hierarchical wheel (3.3) | `timerfd` per timer (3.4) | Tokio direct (3.5) | Why it matters |
|---|---|---|---|---|---|---|
| Insert cost | O(log n) | O(1) | O(1) | one syscall | O(log n) (Tokio internal) | Hot path on streaming-token re-arm. |
| Cancel cost (with handle) | O(1) (via cancelled-set) | O(1) | O(1) | one syscall | implicit on drop | The dominant operation on streaming workloads. |
| Tick cost | O(k log n), `k` = expirations | O(slot-list length) | O(1) amortized + cascade bursts | event-driven (no tick) | runtime-managed | [`FR-008`](../01-requirements/functional.md): less than O(n) per tick. |
| Memory per timer | ~80 bytes (heap node + callback + handle entry) | ~64 bytes (linked-list node) | ~64 bytes | ~1 KB (kernel fd table) | runtime-internal | LLD target: <64 bytes. Heap is ~80 if we count the cancelled-set worst case. |
| Far-future timer cost | sits in heap, no per-rotation work | per-rotation rounds-remaining decrement | sits in coarse wheel, cascades only at boundaries | one fd, kernel-managed | runtime-managed | Mostly irrelevant for request deadlines (seconds, not hours). |
| Implementation complexity | trivial (uses `std`) | moderate (hand-rolled wheel) | high (multi-level, cascade) | high (portability + scale issues) | zero | Engineering capacity in `v0.1` is finite. |
| Conformance-test surface | small | medium | large (cascade ordering, boot, clock jump) | medium | n/a (no trait) | Tests are the contract. |
| Compatibility with `TimerSubsystem` trait | natural | natural | natural | awkward (fd wrapping) | violates the trait | Pluggability is a Riftgate principle. |
| Compatibility with deterministic test mode | yes (advance `tick(now)` manually) | yes | yes | no (kernel-driven) | no (runtime-driven) | We want to test deadline behavior without sleeping in tests. |
| Tick precision | bounded by tick rate (10 ms) | bounded by tick rate | bounded by tick rate | sub-ms (`hrtimer`) | bounded by Tokio's tick rate | Request deadlines do not need sub-ms. |
| `unsafe` code in our crate | none | small (linked list) | larger (cascade) | none (delegates to libc) | none | Lower `unsafe` is lower bug surface. |
| Engineering cost in `v0.1` | hours | days | week+ | days | zero | We want to ship a walking skeleton, not a research paper. |

## 5. Foundational principles

**Hierarchical timing wheels (Varghese & Lauck, SOSP 1987).** The wheel paper's central observation is that for a workload dominated by *cancellation* — telecom call setup/teardown, in their case; streaming-token idle re-arm, in ours — a structure with O(1) cancel beats a structure with O(log n) cancel by orders of magnitude in steady state. The single-level wheel (§3.2) is the simplest realization; the hierarchical variant (§3.3) is the production-grade refinement that handles arbitrary deadline ranges without per-rotation rounds-remaining work. Both are O(1) per operation, where the heap is O(log n). This is the canonical reference for any timer-subsystem decision and is cited directly here.

**`d`-ary heaps and the practical irrelevance of O(log n) at our scale (CLRS ch. 6).** The asymptotic argument for a wheel over a heap is correct but the constants matter. At 100k live timers, `log_2(n)` is ~17; at 1M, ~20. With cache-friendly array layout (which `std::collections::BinaryHeap` provides), the per-operation cost is in the low hundreds of nanoseconds — not zero, but well below our [`NFR-P05`](../01-requirements/non-functional.md) <5 ms TTFT budget by four orders of magnitude. The heap's "log n" is a practical O(1) at any scale we will actually see in `v0.1`, which is why we ship it as the default and treat the wheel as a `v0.2` upgrade once we have benchmark data justifying the implementation cost.

**Lazy deletion as the cancel pattern.** The lazy-cancellation pattern (insert into a "cancelled" set; drop on pop) is a textbook trick for making heap-backed priority queues support O(1) effective cancel. The same shape appears in the Linux kernel's `epoll` ready-list (an entry can be on the list after the fd has been closed; the `epoll_wait` consumer rechecks), in the Java `DelayQueue`, and in the Go runtime's old timer implementation pre-2018. The key invariant is bounded growth — the cancelled set must shrink as cancelled entries are popped — which is why we add a periodic compaction trigger at a configurable threshold.

**Monotonic clock as the timer reference.** Every credible timer subsystem reads `clock_gettime(CLOCK_MONOTONIC)` (Linux) or its equivalent (`mach_absolute_time` on macOS, exposed through `std::time::Instant`). Wall clocks are wrong for this purpose — NTP step adjustments, daylight-savings transitions, container clock-skew on cold-migration — would all produce real bugs. The `Instant` type in Rust's standard library is the right primitive; the trait signature uses it directly.

**The `tick` is driven from the same per-shard event loop as the IO `poll`.** This is the practical glue between the timer subsystem and the rest of the data plane: each shard's event loop alternates `poll(timeout = next_deadline - now)` and `tick(now)`, so the runtime wakes precisely when the next deadline is due (no busy-loop, no oversampling). This is the standard "compute next-deadline; pass it as the poll timeout" pattern used by every event-driven server (`libev`, `libuv`, `nginx`, `redis`).

## 6. Recommendation

**`v0.1` ships `BinaryHeapTimers` — a `std::collections::BinaryHeap` of `(deadline, timer_id)` with lazy cancellation via a per-shard `HashSet<TimerId>`. `v0.2` adds `HierarchicalWheel` behind the same `TimerSubsystem` trait, becoming the default once benchmarks demonstrate the constant-factor win.**

The reasoning, restated:

- The heap satisfies [`FR-008`](../01-requirements/functional.md)'s acceptance criterion ("100k concurrent timers cost less than O(n) per tick") because tick processes only expired entries, not all live entries. The asymptotic class is O(k log n) per tick, not O(n). The "hierarchical timer wheel" wording in FR-008 describes the *direction*; the heap is the conservative `v0.1` step on the way there, with the wheel landing in `v0.2` per [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md).
- The heap is in `std`. No vendored code, no hand-rolled cascading, no boot-edge-case suite. The implementation in [`crates/riftgate-core::timers`](../../crates/) will be ~150 lines including tests; the wheel would be ~1000 lines including the cascade-correctness tests.
- The `TimerSubsystem` trait is the abstraction boundary. When `v0.2` lands `HierarchicalWheel`, no caller changes. The conformance suite in `crates/riftgate-core/tests/timers_conformance.rs` runs against every impl, so the wheel "is correct" if and only if it produces the same fire ordering as the heap on the same input.
- Tick resolution is 10 ms by default ([`docs/04-design/lld-timers.md`](../04-design/lld-timers.md)). Configurable to 5 ms / 100 ms; defaults are good for request-deadline use.
- Per-shard ownership: each shard owns its own `BinaryHeapTimers` instance. No locking, no cross-shard coordination on the hot path. Cross-shard timer dispatch (rare; e.g. a timer scheduled by a request that has migrated shards) goes through the same MPMC queue as work tasks, per [`docs/04-design/lld-timers.md`](../04-design/lld-timers.md).
- The `tick` is driven by the per-shard worker loop, which alternates `AsyncIO::poll(timeout = next_deadline - now)` and `TimerSubsystem::tick(now)`. There is no separate "timer thread."

### Conditions under which we'd revisit

- Benchmarks at the open of `v0.2` (`benchmarks/timers/heap_at_100k_1m.rs`) show the heap exceeding the LLD's "tick processing time at peak should be <100 µs" budget under realistic Riftgate load. We promote `HierarchicalWheel` to default.
- The cancelled-set growth metric (`riftgate_timers_cancelled_pending`) shows pathological accumulation in production deployments. We add a tighter compaction trigger or move to wheel-based cancel.
- A use case appears that genuinely needs sub-millisecond precision (none on the roadmap; mentioned for completeness). We add a third impl that wraps `timerfd` for that specific deployment.

### What stays available behind feature flags

- `HierarchicalWheel` lands in `v0.2` as a peer impl of `TimerSubsystem`. It is *opt-in* in `v0.2` (selected via the config model — see Options [`015`](015-config-model.md)) and becomes default in `v0.3` only if the `v0.2` benchmark gate justifies the switch.
- A `#[cfg(test)] DeterministicTimers` impl ships alongside the heap in `riftgate-core` for unit-testing deadline-sensitive code without sleeping. This is the FR-X02 second impl that satisfies the "every trait has at least two implementations" discipline.

## 7. What we explicitly reject

- **Hierarchical wheel as the `v0.1` default.** Implementation cost is too high for a walking-skeleton milestone. Reconsider in `v0.2` per the conditions above.
- **OS `timerfd`-per-timer.** Burns one fd per live timer at our scale; the fd-table cost alone is prohibitive. Reconsider only if a sub-millisecond-precision use case appears on the roadmap (none planned).
- **Tokio's timer driver as a hidden default.** Violates the trait-as-contract discipline ([Vision §3](../00-vision.md), [`AGENTS.md` §5](../../AGENTS.md)). We ship Riftgate timers behind a Riftgate trait; we do not depend on a runtime-internal type for a kernel concern.
- **Single-level hashed wheel.** Strictly dominated by the hierarchical wheel for our deadline range (seconds to minutes). The rounds-remaining decrement work is real CPU we don't need to pay.
- **Per-timer threads.** Would have been the obvious wrong answer in 1995; mentioned only because it occasionally still appears in third-party libraries. Burns one stack per timer; impossible at our scale.
- **A custom hand-rolled tree** (`BTreeMap` or rb-tree). The standard library's `BinaryHeap` is faster on this workload (better cache behavior, no per-node pointer chasing). Reconsider only if we discover a use case that needs efficient *range queries* over deadlines, which we do not.

## 8. References

1. George Varghese and Tony Lauck, *Hashed and Hierarchical Timing Wheels: Data Structures for the Efficient Implementation of a Timer Facility*, SOSP 1987 — <https://www.cs.columbia.edu/~nahum/w6998/papers/sosp87-timing-wheels.pdf>
2. Linux `timerfd_create(2)` man page — <https://man7.org/linux/man-pages/man2/timerfd_create.2.html>
3. Linux `clock_gettime(2)` man page — <https://man7.org/linux/man-pages/man2/clock_gettime.2.html>
4. FreeBSD `kevent(2)` and the `EVFILT_TIMER` filter — <https://man.freebsd.org/cgi/man.cgi?query=kevent>
5. Linux kernel `timer_wheel` rewrite (LWN 2016) — <https://lwn.net/Articles/646950/>
6. Tokio's internal timer driver (source on GitHub) — <https://github.com/tokio-rs/tokio/tree/master/tokio/src/runtime/time>
7. Netty `HashedWheelTimer` (Java reference implementation) — <https://netty.io/4.1/api/io/netty/util/HashedWheelTimer.html>
8. Kafka `TimingWheel` (Scala reference implementation) — <https://github.com/apache/kafka/blob/trunk/server-common/src/main/java/org/apache/kafka/server/util/timer/TimingWheel.java>
9. Ulrich Drepper, *Futexes Are Tricky* — <https://www.akkadia.org/drepper/futex.pdf>
10. Thomas H. Cormen, Charles E. Leiserson, Ronald L. Rivest, Clifford Stein, *Introduction to Algorithms* (CLRS, 4th ed.) — chapter 6 on heaps and priority queues.
11. `std::collections::BinaryHeap` Rust standard library documentation — <https://doc.rust-lang.org/std/collections/struct.BinaryHeap.html>
