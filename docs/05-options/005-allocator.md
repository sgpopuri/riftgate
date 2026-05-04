# 005. Allocator

> **Status:** `recommended` — per-request `BumpArena` on the hot path with the system `malloc` as the global allocator default in `v0.1`; `mimalloc` becomes the opt-in global allocator in `v0.2`. See [ADR 0006](../06-adrs/0006-bump-arena-plus-system-malloc.md).
> **Source-systems chapter:** `Ch14 (memory allocators)`
> **Related options:** [001](001-io-model.md) (IO model), [002](002-async-runtime.md) (async runtime), [003](003-concurrency-model.md) (concurrency model)
> **Related ADR:** [ADR 0006](../06-adrs/0006-bump-arena-plus-system-malloc.md)

## 1. The decision in one sentence

> What combination of allocator strategies (per-request arena vs general-purpose `malloc` replacement vs system default) does Riftgate use to keep request-path allocation predictable and tail-latency-clean?

## 2. Context — what forces this decision

Allocation is the single least-bounded operation a network server does. A single `malloc` call can take anywhere from ~50 ns (fast path) to several milliseconds (in pathological fragmentation cases). For an LLM gateway with [NFR-P02](../01-requirements/non-functional.md) targeting <10 ms P99 overhead, allocation jitter is a real risk.

Forces driving this decision:

- **Per-request memory has known scope.** A request's parser scratch buffer, filter chain state, response framing buffer, and intermediate JSON values are all created during the request and discarded at completion. This is the textbook arena allocator workload.
- **Per-request memory has bounded size.** The vast majority of requests fit within a few KB of working memory. A small per-request arena, recycled across requests, is dramatically cheaper than calling `malloc`/`free` on every allocation inside the request.
- **The general-purpose allocator still matters.** Connection objects, configuration, the metric registry, the timer wheel, the WAL — all of these have lifetimes longer than a request. They allocate through the global allocator. The global allocator's quality affects steady-state RSS, fragmentation, and tail latency on long-lived data structures.
- **NFR-P04** targets <16 KB per idle connection. The global allocator's per-allocation overhead, internal fragmentation, and metadata costs all show up here.
- **NFR-C01** targets <50 MB RSS at idle. The global allocator's working-set discipline (eager-decommit vs lazy-decommit, page caching, retain-vs-return-to-OS heuristics) affects this directly.
- **Pluggability principle.** [`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md) defines an `Allocator` trait with `alloc(layout)` and `reset()`. The trait must accommodate both the bump arena and the global allocator wrapper.
- **No undefined behavior.** The hand-rolled bump arena must respect each `Layout`'s alignment; mistakes are silent and devastating ([`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md) Pitfalls).

The decision is consequential because allocation patterns set in `v0.1` constrain every later subsystem: filter chain memory model, WAL buffer reuse, parser scratch space.

## 3. Candidates

We evaluate five candidates for the combined allocation strategy. The first three swap only the global allocator; the last two combine a per-request arena with a global-allocator choice.

### 3.1. System `malloc` everywhere

**What it is.** Use whatever `malloc` ships with the platform's libc — `ptmalloc2` on glibc Linux, `malloc-ng` on musl, the macOS allocator on Tier-2. No per-request arena; everything goes through the global allocator. This is the default if you do nothing.

**Why it's interesting.**
- Zero engineering cost. `cargo build` produces a working binary; the global allocator is whatever the platform provides.
- No `cargo` features, no extra dependencies, no platform-specific build steps.
- The system allocator on modern glibc is competent: per-thread arenas reduce contention, scan-based GC of unused arenas reduces RSS over time.
- Familiar to operators; `mtrace`, `glibc`'s built-in tcmalloc-equivalent diagnostics, `valgrind`, and so on all work without configuration.

**Where it falls short.**
- **Variability per platform.** glibc, musl, and macOS allocators behave differently under load. A binary that performs well on glibc may regress on musl (which historically had a more conservative allocator until `mallocng`).
- **Per-allocation overhead is real.** A typical `malloc` call costs 50-200 ns on glibc; on the request hot path (where we may do dozens of small allocations per request), this is a large fraction of the total request cost.
- **Tail latency under fragmentation.** Pathological allocation patterns can push individual `malloc` calls into the millisecond range. Modern allocators mitigate this but do not eliminate it.
- **No control over per-request lifecycle.** Every per-request allocation is `free`'d individually; the cost is real even when the allocations are bounded-scope.

### 3.2. `jemalloc` as the global allocator

**What it is.** A drop-in replacement for the system `malloc`, originally written for FreeBSD, then maintained by Facebook for many years. Multi-arena design, size-class buckets, lazy decommit. Linked in via the `jemallocator` crate as `#[global_allocator]`.

**Why it's interesting.**
- Battle-tested at hyperscaler workloads (Facebook, Twitter, Discord historically).
- Excellent multi-thread scaling: per-CPU arenas, lock-free fast paths in most size classes.
- Tunable knobs (`MALLOC_CONF`) for narrow workload adaptation.
- Substantially better than glibc's `ptmalloc2` on highly concurrent allocation-heavy workloads, particularly older glibc versions.
- Predictable memory profile under load (less prone to fragmentation collapse than naive allocators).

**Where it falls short.**
- **Maintenance status is uncertain in 2026.** Facebook archived the upstream jemalloc repository in 2024 with no clear successor. The community fork is alive but slower-paced. New issues take longer to resolve.
- **Larger binary size.** Statically linking jemalloc adds ~500 KB to the binary, which trades off against the [NFR-O06](../01-requirements/non-functional.md) container-size goal (<50 MB image).
- **Build-time complexity.** Cross-compilation and platform support is more brittle than mimalloc; some musl + jemalloc combinations have known issues.
- **Knob tuning is a real cost.** The default config is good but not great; serious users tune `MALLOC_CONF`, which is operator-facing complexity Riftgate should avoid promising.
- **Doesn't help the per-request hot path much.** Still per-allocation overhead, still per-allocation `free`. We get a faster underlying allocator, not a structurally cheaper allocation pattern.

**Real-world systems that use it.** Facebook (Cassandra, RocksDB, HHVM), Twitter (Manhattan, Finagle), Discord (the Rust services historically), Aerospike, parts of the C++ ecosystem.

### 3.3. `mimalloc` as the global allocator

**What it is.** Microsoft's modern general-purpose allocator. Free-list-based small-object allocation, segmented thread-local heaps, sharded large-object handling. Smaller code footprint than jemalloc, simpler to integrate, comparable or better performance on most modern workloads. Linked in via the `mimalloc` crate.

**Why it's interesting.**
- Smaller binary footprint than jemalloc (~150 KB linked statically).
- Simpler to integrate. `#[global_allocator] static GLOBAL: MiMalloc = MiMalloc;` and that's the whole change.
- Active upstream development (Microsoft Research, public GitHub).
- Per-thread heaps with minimal cross-thread coordination on the fast path.
- Performance is in the same ballpark as jemalloc on most workloads; meaningfully better than glibc on highly concurrent allocation patterns.
- Cross-platform support is strong: Windows, Linux, macOS, BSD all first-class.

**Where it falls short.**
- **Less battle-tested at hyperscaler scale than jemalloc.** Microsoft uses it internally; many smaller companies use it; Facebook-scale workloads are not the primary target.
- **Same "doesn't help the hot path structurally" objection as jemalloc.** A faster `malloc` is still slower than no `malloc`.
- **Some known performance edges where jemalloc wins** (very-many-small-allocations workloads have benchmarks that go either way; mimalloc is generally close but occasionally behind).
- **Same build-time consideration as jemalloc** for cross-compilation, though somewhat more forgiving.

**Real-world systems that use it.** Microsoft (.NET runtime in some configurations, internal tools), Bun (the JavaScript runtime), Lemmy (Rust web service), increasingly common in Rust services that want a `malloc` upgrade without jemalloc's maintenance overhead.

### 3.4. Per-request `BumpArena` on the hot path + system `malloc` globally

**What it is.** A bump-pointer arena allocator, instantiated once per request, used for all per-request allocations (parser scratch, filter state, response framing). On request completion, the arena is reset (one pointer set to zero) and returned to a per-worker pool. The global allocator stays as the platform default; only the hot path bypasses it.

**Why it's interesting.**
- **Allocation cost on the hot path is ~5-15 ns per allocation** (a single pointer increment plus an alignment adjustment). Compared to `malloc`'s ~50-200 ns, the per-allocation savings multiply across the request.
- **Free is O(1) for the entire request.** No per-allocation `free` calls; the arena is reset wholesale.
- **Working set stays cache-warm.** Per-worker arena pools mean each worker reuses the same physical memory across requests, with very high L1/L2 cache hit rates.
- **Lifetime safety.** Rust's borrow checker enforces that per-request arena memory does not escape the request scope. Lifetime annotations on the public API provide defense in depth.
- **Bounded per-request memory.** A configured cap (default 1 MB per [`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md)) prevents a single bad request from growing the arena indefinitely; over-cap allocations fall back to the global allocator with a warning.
- **Doesn't change the global allocator.** Operators get the platform default, which is the most familiar shape.

**Where it falls short.**
- **Hand-rolled allocator means correctness work.** Alignment must be respected for every `Layout`; misaligned writes are undefined behavior. Tests must cover the alignment surface.
- **Arena pool sizing is operator-visible.** Per-worker pool size, initial arena size, max arena size — all become tunables. Defaults must be conservative and clearly documented.
- **Mid-request arena growth is a thing.** When a request's working set exceeds the initial arena size, we double until cap. The doubling moves data, which is more expensive than the bump-pointer fast path. Common case is uncommon, but it happens.
- **Doesn't help non-hot-path allocations.** Connection objects, config, timer state, WAL buffers — these still go through the system `malloc`, with all its quirks.
- **Lifetime ergonomics push complexity into the API.** `&'arena str` and `&'arena [u8]` annotations are everywhere. Worth it, but a learning cost for new contributors.

**Real-world systems that use it.** Postgres (per-query memory contexts), Bumpalo (the Rust crate), Servo (per-pipeline arenas), Apache Arrow (columnar memory pools), most compilers (per-pass arenas in LLVM, rustc), high-performance JSON parsers like `simd-json`.

### 3.5. Per-request `BumpArena` on the hot path + `mimalloc` globally

**What it is.** Combines candidate 3.4 with candidate 3.3: per-request arena on the request path, plus `mimalloc` as the global allocator for everything else. The combined memory profile is "best per-request structure plus best general-purpose allocator."

**Why it's interesting.**
- All the wins of 3.4 on the hot path.
- All the wins of 3.3 on the non-hot-path (connection state, WAL buffers, timer wheel, config, metrics).
- Predictable behavior under load: the request path is bump-pointer-fast, the global allocator is multi-thread-fast.
- A single `cargo` feature flag (`--features mimalloc`) toggles the global allocator; the arena is always on.

**Where it falls short.**
- **Two allocator surfaces to debug.** A memory regression now needs to be triaged between the arena and the global allocator. Tracing must distinguish.
- **Build-time + binary-size cost of mimalloc.** Same considerations as 3.3.
- **Cross-platform `mimalloc` quirks.** Same considerations as 3.3.

## 4. Tradeoff matrix

| Property | System `malloc` only | jemalloc global | mimalloc global | Arena + system | Arena + mimalloc | Why it matters |
|----------|----------------------|-----------------|-----------------|----------------|------------------|----------------|
| Hot-path allocation cost | ~50-200 ns/op | ~30-100 ns/op | ~30-100 ns/op | ~5-15 ns/op | ~5-15 ns/op | At our QPS targets, allocation cost is on the critical path. |
| Hot-path tail latency | medium (occasional `malloc` outliers) | medium-good | medium-good | very good | very good | [NFR-P02](../01-requirements/non-functional.md). |
| Per-request lifecycle clarity | poor (intermixed alloc/free) | poor | poor | very good (bulk reset) | very good | Memory leaks become "the arena is wrong-sized." |
| Non-hot-path scaling | medium (glibc per-thread arenas help) | very good | very good | medium (system fallback) | very good | Connection objects, WAL, metrics. |
| Idle RSS ([NFR-C01](../01-requirements/non-functional.md), <50 MB) | medium-good | good | good | good | very good | Important for sidecar / K8s deployment. |
| Per-idle-connection footprint ([NFR-P04](../01-requirements/non-functional.md), <16 KB) | medium | good | good | very good (arena returns to pool) | very good | Memory cost of holding open many conns. |
| Binary size | smallest | +500 KB (jemalloc) | +150 KB (mimalloc) | +20 KB (arena code) | +170 KB | [NFR-O06](../01-requirements/non-functional.md). |
| Engineering cost in `v0.1` | zero | low (one feature flag) | low | medium (write the arena) | medium | One maintainer in `v0.x`. |
| Cross-platform robustness | best (platform default) | medium (build quirks on musl) | good | best (no global change) | medium-good | Linux x86_64 + arm64 + macOS dev. |
| Familiarity to operators | best | high (well-known) | medium | best (no surprises globally) | medium | Pager-friendly. |
| Compatibility with `Allocator` trait | natural | natural | natural | natural | natural | Pluggability. |
| Compatibility with future per-request tracing (alloc-tracking) | poor | poor | poor | very good (arena owns tracking) | very good | Useful for debugging weird requests. |

## 5. What the source-systems chapters say

`Ch14 (memory allocators)` is the single reference here, and it covers the design space from first principles. Three takeaways:

1. **The bump-pointer arena is the simplest non-trivial allocator that exists, and it is unbeatable for known-scope workloads.** Allocation is one pointer-add and an alignment mask. Free is one pointer-set. The chapter is direct: **if your workload has request-scoped memory, use an arena; do not use a general-purpose allocator and expect comparable performance.**
2. **General-purpose `malloc` quality varies dramatically by implementation.** The chapter benchmarks ptmalloc2 (glibc), jemalloc, mimalloc, and tcmalloc on a synthetic threading workload. mimalloc and jemalloc cluster within 10% of each other; tcmalloc is competitive; ptmalloc2 lags meaningfully on highly threaded patterns. The chapter recommends jemalloc or mimalloc when concurrency matters; both are "fine" for less-demanding workloads.
3. **The allocator is the single biggest source of "mysterious tail latency."** The chapter lists scenarios — large-arena rebalancing, cross-arena cache eviction, fragmentation collapse — that produce occasional millisecond-scale `malloc` calls even on healthy systems. **The defense is to remove `malloc` from the hot path.** Arenas are the answer.

The chapter does not strongly discriminate between mimalloc and jemalloc; both are good. It notes that mimalloc is "newer, smaller, and currently has more momentum upstream."

## 6. Recommendation

**`v0.1` ships a per-request `BumpArena` on the hot path (with per-worker arena pooling for reuse) plus the system `malloc` as the global allocator. `v0.2` adds `mimalloc` as an opt-in global allocator behind the `mimalloc` cargo feature.**

The reasoning, restated:

- The arena is where the largest wins are. Hot-path allocation cost drops from ~50-200 ns to ~5-15 ns; bulk reset replaces dozens of `free` calls per request.
- The system `malloc` is fine for the global allocator in `v0.1`. It is the most familiar default; it ships on every platform without a build-time cost; it produces the smallest binary.
- `mimalloc` as opt-in in `v0.2` gives users who care about the global-allocator's tail behavior an upgrade path with a single cargo feature. We wait until `v0.2` because by then we have benchmarks to demonstrate the win on Riftgate's actual workload, not a synthetic benchmark.
- We pick `mimalloc` over `jemalloc` because (a) jemalloc's upstream maintenance posture is uncertain in 2026, (b) mimalloc has a smaller binary footprint, and (c) integration is dead simple. Operators who prefer jemalloc can write their own `#[global_allocator]` block; we do not actively block them.

### Conditions under which we'd revisit

- Benchmarks in `v0.2` show that the global-allocator choice (system vs mimalloc) changes our P99 by a meaningful margin. We would consider promoting `mimalloc` to the default in `v0.3+`.
- Per-request memory profiles show a class of requests for which the bump arena is wrong (e.g. very-long-lived streaming requests with growing buffers). We would document the case and consider a hybrid arena strategy.
- A new allocator (e.g. tcmalloc, snmalloc, scudo) reaches the same level of maturity and ergonomics as mimalloc and has a clear win on our workload.

### What stays available behind feature flags

- `mimalloc` global allocator behind `--features mimalloc` in `v0.2`.
- A future `jemalloc` global allocator option behind `--features jemalloc`, only if there is user demand.
- Per-request arena tracing (which call sites allocated how much, see [`docs/04-design/lld-allocator.md`](../04-design/lld-allocator.md) Open questions) behind `--features arena-tracing`.

## 7. What we explicitly reject

- **No arena (system `malloc` only) as the production default.** Allocation cost on the hot path is a real source of tail latency; the arena is a bounded engineering effort with a measurable win. Reconsider only if profile data shows the arena is harmful for some workload (which would be a surprising and interesting result).
- **`jemalloc` as the default in `v0.1` or `v0.2`.** Maintenance posture in 2026 is uncertain; mimalloc is the more conservative choice. Reconsider if the upstream picks up a clear successor team.
- **`tcmalloc` (Google's allocator).** Has a Rust binding (`tcmalloc` crate) but the upstream's split between `gperftools` and Google's internal version creates ambiguity; we prefer the cleaner `mimalloc` story.
- **`snmalloc` (Microsoft Research's experimental allocator).** Promising but pre-1.0; not the right risk profile for `v0.x`. Reconsider in `v1.0+`.
- **A custom global allocator in `riftgate-core`.** Not the differentiation we are chasing; the engineering cost is enormous; mature options are good enough.
- **No per-request memory cap.** A bad request that grows the arena indefinitely is a real failure mode. The cap is non-negotiable.

## 8. References

1. Daan Leijen, Benjamin Zorn, Leonardo de Moura, *Mimalloc: Free List Sharding in Action* (APLAS 2019) — https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/
2. mimalloc on GitHub — https://github.com/microsoft/mimalloc
3. jemalloc on GitHub (Facebook archive + community fork) — https://github.com/jemalloc/jemalloc
4. Jason Evans, *A Scalable Concurrent malloc(3) Implementation for FreeBSD* (BSDCan 2006) — original jemalloc paper
5. The bumpalo crate (Rust bump-allocator implementation) — https://docs.rs/bumpalo
6. Postgres memory contexts overview — https://www.postgresql.org/docs/current/memorycontextswitch.html
7. Apache Arrow memory pools — https://arrow.apache.org/docs/cpp/memory.html
8. Riftgate source-systems chapter `Ch14 (memory allocators)`
