# 023. Token bucket parameters

> **Status:** `recommended` — fix the `TokenBucketLimiter` knob set, internal representation, and defaults for v0.2. See [ADR `0018`](../06-adrs/0018-token-bucket-parameters.md).
> **Foundational topics:** token-bucket algorithm (Tanenbaum *Computer Networks*; Nginx `limit_req_zone`), packed atomic state (Vyukov-style CAS), sharded hash maps, fixed-point arithmetic vs `f64` on the hot path
> **Related options:** [`021 — rate limiting`](021-rate-limiting.md) (the parent design space), [`012 — backpressure`](012-backpressure.md) (sibling rejection vocabulary)
> **Related ADR:** [ADR `0018`](../06-adrs/0018-token-bucket-parameters.md), depends on [ADR `0009`](../06-adrs/0009-rate-limiter-trait-in-proc-only.md)

## 1. The decision in one sentence

> Given that [Options `021`](021-rate-limiting.md) and [ADR `0009`](../06-adrs/0009-rate-limiter-trait-in-proc-only.md) commit us to an in-proc token-bucket impl, *exactly* which knobs does an operator turn, what shape is the internal state, and what defaults do we ship?

## 2. Context — what forces this decision

[Options `021`](021-rate-limiting.md) explored the seven-candidate rate-limiting design space and landed on token-bucket. [ADR `0009`](../06-adrs/0009-rate-limiter-trait-in-proc-only.md) committed v0.2 to an in-proc impl behind the `RateLimiter` trait. Neither document names the *implementation* parameters: refill arithmetic, state packing, shard count, default rate/burst values, cost-function shape. Those parameters are too detailed for `021` (which is a survey) and too narrow for an LLD section (which is operating theory, not a knob table).

This Options doc closes that gap. It is short on purpose — three to four candidate implementations of one already-chosen algorithm, with a recommendation on each parameter.

Two requirements bind:

- [`NFR-P07`](../01-requirements/non-functional.md): <100 µs enforcement overhead per request at 1k RPS.
- [`NFR-A03`](../01-requirements/non-functional.md): bounded worst-case allocator footprint — so the subject map cannot grow without bound and is sized at config-validation time.

## 3. Candidates

The candidates here are not "which rate limiter" — that is settled — but "how do we *represent* the token-bucket state on the hot path."

### 3.1. `Mutex<BucketState>` with `f64` tokens

**What it is.** Per subject, `Mutex<{ tokens: f64, last_refill: Instant }>` inside a `DashMap`.

**Why it's interesting.**
- Trivial to write and reason about.
- `f64` is precise enough for any practical rate.

**Where it falls short.**
- A mutex on the hot path is a contention magnet under burst arrival on a single key. The textbook "noisy-neighbor amplification" pattern.
- `Instant` is not `Copy` to a packed integer; serializing for an atomic CAS requires conversion.
- Allocates a small per-subject `Mutex` header.

### 3.2. `AtomicU64` packed state, fixed-point tokens

**What it is.** Per subject, one `AtomicU64` packing `(tokens_scaled: u32, last_refill_nanos: u32)`. `tokens_scaled` is fixed-point with `SCALE = 1 << 16` (one token = 65536 microtokens). `last_refill_nanos` wraps around modulo `2^32` ≈ 4.3 seconds, which is more than enough resolution because we anchor against an `AtomicU64` epoch nanos timestamp computed on first-touch and stored in a small per-shard array.

The check is a CAS loop:

```rust
loop {
    let cur = state.load(Acquire);
    let (tokens, last) = unpack(cur);
    let now = elapsed_nanos();
    let refilled = (tokens + (now - last) * rate / NANOS_PER_SEC).min(burst_scaled);
    if refilled < cost_scaled { return Deny { retry_after: refill_eta(cost_scaled - refilled, rate) }; }
    let next = pack(refilled - cost_scaled, now);
    if state.compare_exchange_weak(cur, next, Release, Acquire).is_ok() { return Allow; }
}
```

**Why it's interesting.**
- Lock-free on the fast path. The CAS loop converges in one or two iterations under realistic contention.
- One word of state per subject — cache-line friendly.
- Cleanly maps to a future Redis-Lua or Dragonfly impl: the Lua script is *exactly* this CAS loop.

**Where it falls short.**
- Fixed-point arithmetic requires the operator to understand SCALE and burst limits — masked behind a config validator.
- 32-bit nanos wraparound requires the per-shard epoch trick; one more concept in the implementation.

### 3.3. `AtomicU64` packed state, integer microtokens

**What it is.** Same as 3.2 but with `SCALE = 1_000_000` (one token = one microtoken) and `last_refill_micros`.

**Why it's interesting.**
- Microtoken resolution makes the cost-function arithmetic cleaner (`cost_micros = prompt_tokens * 1_000_000`).
- Wraparound window is ~71 minutes — no per-shard epoch trick.

**Where it falls short.**
- 20-bit-ish microtoken range means burst >~4000 token-equivalent requests overflows the packed field. Not enough headroom for the TPM use case where bursts in tokens can be much larger.
- Same fixed-point conceptual cost as 3.2.

### 3.4. Two `AtomicU64` per subject (tokens + last_refill), unpacked

**What it is.** Two atomics per subject, updated separately.

**Why it's interesting.** No packing, no fixed-point.

**Where it falls short.**
- Cannot be CAS'd atomically — the two atomics can be out of phase, which produces over-allowance (a torn refill that double-counts).
- Larger state footprint per subject.

**Real-world systems that use it.** Some naive Rust crates. Not a serious candidate.

## 4. Tradeoff matrix

| Property | 3.1 Mutex+f64 | 3.2 Packed scaled (SCALE 65536) | 3.3 Packed micros | 3.4 Two atomics | Why it matters |
|---|---|---|---|---|---|
| Hot-path cost | mutex acquire | CAS loop | CAS loop | torn (incorrect) | NFR-P07 |
| Contention behaviour | bad on hot keys | 1-2 CAS retries | 1-2 CAS retries | n/a (torn) | LLM workloads concentrate on few subjects |
| State per subject | 24-32 B + Mutex header | 8 B | 8 B | 16 B | Allocator footprint |
| Burst headroom | unlimited | up to `u32::MAX >> 16` ≈ 65k token-equivalents | ~4k token-equivalents | unlimited but torn | TPM bursts in tokens |
| Time resolution | `Instant` | 4.3s wrap (per-shard epoch) | 71min wrap | n/a | Implementation simplicity |
| Maps to Redis-Lua impl | poorly | directly | directly | n/a | Future distributed impl path |
| Implementation surface area | smallest | medium | medium | small | Code we have to maintain |

## 5. Foundational principles

**Vyukov-style packed atomic CAS.** The pattern of packing two related fields into a single atomic word so they update together is canonical in the lock-free literature. Dmitry Vyukov's writings on it; Paul McKenney's *Is Parallel Programming Hard* §4. The two fields must update together because the refill calculation depends on *both* `tokens` and `last_refill`; tearing them produces double-refill.

**Sharded hash map (DashMap, partition-by-hash).** A single map under contention is a microbenchmark anti-pattern. `DashMap` (or any partition-by-hash scheme) shards the map into N independent locks; for our use case where the lock protects only insert/remove (not the hot CAS path), the contention is low.

**Fixed-point on the hot path.** `f64` is precise but slower to update atomically (no `AtomicF64` without bit-casting). Fixed-point with a power-of-two scale (`SCALE = 65536`) makes the arithmetic shift-and-add and the packed atomic representation natural.

**Subject key cardinality bounding (config validator).** [`NFR-A03`](../01-requirements/non-functional.md) requires bounded memory. The config validator computes `max_subjects = tenants × routes × backends` from declared `[[backend]]`/`[[route]]`/`[[tenant]]` entries and enforces it as the `DashMap`'s `with_capacity`. Subjects beyond the declared cardinality are rejected at config time, not at runtime.

## 6. Recommendation

**Adopt candidate 3.2: `AtomicU64` packed state with `tokens_scaled` (SCALE = `1 << 16`) and `last_refill_nanos` (mod 2^32) plus a per-shard epoch anchor. CAS-loop fast path. `DashMap<SubjectKey, AtomicU64>` outer container with 64 shards (default).**

Knob set the operator sees:

```toml
[[rate_limit]]
# Subject scope: any subset of (tenant, route, backend) — omitted fields are wildcards.
tenant  = "team-platform"
route   = "openai-chat"

# Algorithmic knobs.
rate_per_sec = 100       # sustained allowance
burst        = 200       # bucket capacity; default = 2 * rate_per_sec
cost         = "request" # or "prompt_tokens" for TPM limiting

# Behaviour on denial.
retry_after_floor_ms = 50
```

Internal defaults (not operator-tunable in v0.2):

- `SCALE = 1 << 16` (65,536 microtokens per token).
- 64 shards in the outer `DashMap`. Tuned at v0.2 retro if bench shows hot-shard contention.
- Per-shard epoch nanos resolves 32-bit-nanos wraparound (~4.3 s window).
- Max subject cardinality computed at config-validation time from declared scopes; refusal at config-load if exceeded.

Telemetry:

- `riftgate.ratelimit.checked` (counter, labelled by subject scope and decision).
- `riftgate.ratelimit.denied` (counter, labelled by `DenialReason::RateLimit`).
- `riftgate.ratelimit.bucket_depth` (histogram, sampled per subject for top-K hot subjects).
- `riftgate.ratelimit.cas_retries` (histogram) — debugging signal for hot-key contention.

### Conditions under which we'd revisit

- If criterion-benched p99 of `RateLimiter::check` exceeds 100 µs at 1k RPS (NFR-P07), we revisit shard count and/or move to candidate 3.3.
- If TPM bursts in real deployments exceed the ~65k token-equivalent headroom of SCALE 65536, we revisit either the scale or move to a `u128`-packed variant.
- If `cas_retries` histograms in real deployments show p99 > 8 retries, we revisit the per-key contention strategy (per-key spinlock fallback, or shard-by-(subject, time-window) for time-skewed bursts).

## 7. What we explicitly reject

- **Candidate 3.1 (Mutex + f64).** The contention failure mode on hot subjects is exactly the LLM-workload pattern (a few power-user tenants). We will not ship it.
- **Candidate 3.4 (two atomics).** Torn refills produce silent over-allowance. Will not ship.
- **Operator-tunable SCALE or shard count.** Internal performance knobs do not belong in operator config. We benchmark-tune them and leave them as compile-time defaults.
- **An adaptive cost function.** Cost is `request` (=1) or `prompt_tokens` (=`token_count`) in v0.2. A user-defined cost lambda is a v0.3+ extension behind WASM filters (Options `016`).
- **Per-subject TTL eviction in v0.2.** Subject cardinality is bounded at config-validation time; entries persist for the process lifetime. Eviction is a v0.3 concern once cardinality patterns are observed.

## 8. References

1. Andrew S. Tanenbaum, *Computer Networks* (5th ed.), §5.4.3 — token-bucket algorithm.
2. Dmitry Vyukov, *Bounded MPMC queue* and related lock-free writings — <https://www.1024cores.net/>
3. Paul E. McKenney, *Is Parallel Programming Hard, And, If So, What Can You Do About It?* — §4 on atomics and memory ordering.
4. Maurice Herlihy, Nir Shavit, *The Art of Multiprocessor Programming* (2nd ed.) — §10 on lock-free data structures.
5. Nginx, [`limit_req_zone` documentation](http://nginx.org/en/docs/http/ngx_http_limit_req_module.html) — production-grade token bucket parameters.
6. Brandur Leach, *Rate Limiting with GCRA and Redis* — <https://brandur.org/rate-limiting> — the reference for the future distributed impl.
7. Rust crate [`dashmap`](https://docs.rs/dashmap/) — the sharded hash map we use.
8. Rust crate [`governor`](https://docs.rs/governor/) — reference impl using GCRA; we deliberately pick token-bucket framing for operator legibility (per [Options `021` §6](021-rate-limiting.md)).
