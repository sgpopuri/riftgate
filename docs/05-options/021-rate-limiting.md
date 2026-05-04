# 021. Rate limiting

> **Status:** `recommended` — trait-based rate limiter with a single in-proc token-bucket impl in `v1.0`; distributed impls catalogued as a future extension of the same trait. See ADR `0009` (reserved).
> **Source-systems chapters:** `systems/ch12 (system design patterns)`, `systems/ch08 (pub/sub messaging — backpressure as policy)`
> **Sibling-book chapters:** `hashing/ch07 (consistent hashing)` (sharding a distributed counter), `trees/ch04 (heaps and priority queues)` (which request gets throttled first when several are at the edge)
> **Related options:** [`011 — circuit breaker`](011-circuit-breaker.md) (same family: protection primitives), [`012 — backpressure`](012-backpressure.md) (the policy complement)
> **Related ADR:** ADR `0009` (reserved)

## 1. The decision in one sentence

> Which rate-limiting algorithm does Riftgate enforce on the hot path, and how do we avoid painting ourselves into a corner if we ever need the limit to span multiple replicas?

## 2. Context — what forces this decision

Riftgate sits in front of LLM backends that enforce their own hard Tokens-Per-Minute (TPM), Requests-Per-Minute (RPM), and concurrency limits. Two failure modes matter:

1. **Upstream-capacity failure.** If the gateway does not meter, it will happily forward requests until the backend returns `429` — at which point we have already paid the parsing, filter-chain, and routing cost, and we now need to propagate or retry. Metering on the way in is cheaper.
2. **Tenant-abuse failure.** A single client (or a single bug in a downstream service) can exhaust the gateway's shared resources — allocator arenas, connection slots, WAL bandwidth — before the backend pushes back. Metering protects the gateway from its own clients.

Rate limiting is the policy primitive that addresses both. It is a small amount of code; it is a large amount of design space. And it is the place where a new Riftgate contributor most needs a clean trait to extend.

Two requirements frame the choice:

- [`FR-108`](../01-requirements/functional.md) — an in-proc token-bucket limiter in `v0.2`.
- [`NFR-P07`](../01-requirements/non-functional.md) — <100 µs enforcement overhead per request at 1k RPS on the `v0.2` impl.

Riftgate's posture, stated in Vision [`§4`](../00-vision.md): we ship per-instance rate limiting only; cross-replica coherence is a future distributed impl of the same trait, not a `v1.0` commitment. This doc catalogues the whole design space even though `v1.0` only ships one impl — *because the Options doc is a teaching artifact and the trait must accommodate the future we are declining to ship today*.

## 3. Candidates

We evaluate seven candidates spanning the entire design space, from the simplest local counter to the most elaborate distributed scheme.

### 3.1. Fixed-window counter

**What it is.** Split time into fixed windows (e.g. 60-second buckets). Keep a counter per (subject, window). Each request increments the counter; if the counter exceeds the limit, reject. At window rollover, the counter resets.

```text
Time:    [0 ------------ 60s ------------ 120s -----------]
                                  ^                        ^
                                  60 requests here         OK
                                  60 requests here         2x burst!
```

**Why it's interesting.**
- Trivial to implement. A single atomic counter per subject. Constant memory per subject.
- Fast. One load, one add, one compare.
- The mental model every operator already has: "60 per minute."

**Where it falls short.**
- **Window-boundary bursts.** A client can fire 60 requests at 59.9s and another 60 at 60.1s — 120 requests in 200ms while staying under "60 per minute" in both windows. The canonical rate-limit failure mode.
- **Sudden reset.** At window rollover the allowance snaps back; there is no smoothing.
- **No burst vs. sustained distinction.** The same number measures both, which is almost never what the operator meant.

**Real-world systems that use it.** Naive first cuts everywhere. Cloudflare's free-tier rate limiter used fixed windows for a long time; most homegrown Redis-based limiters start here before hitting the boundary-burst problem.

**Sketch.**
```rust
fn check(&self, subject: &Key) -> Decision {
    let window = now_secs() / 60;
    let counter = self.counters.entry((subject.clone(), window)).or_insert(0);
    if *counter >= self.limit { Decision::Deny } else { *counter += 1; Decision::Allow }
}
```

### 3.2. Sliding-window log

**What it is.** For each subject, keep a timestamp list of the last N requests. On each call, drop timestamps older than the window and count the remainder. Allow if count < limit.

**Why it's interesting.**
- Exact. No window-boundary bursts, no smoothing fudge. The answer to "did this subject send >N requests in the last 60 seconds" is exactly right.
- Trivial to reason about.

**Where it falls short.**
- **Memory cost is O(limit) per subject.** For 10k subjects at 100 req/min each, you carry 1M timestamps. On the hot path this is the opposite of what we want.
- **GC cost grows with window size.** At each request you scan the tail.
- **In a distributed impl, the log has to be shared — which is a bad cache-line citizen.** High contention on a single list is a microbenchmark anti-pattern waiting to happen.

**Real-world systems that use it.** Used in places where exactness matters more than memory (billing-grade metering, audit-level quotas). Not a common production choice for generic API rate limiting.

### 3.3. Sliding-window counter (sometimes called "weighted window")

**What it is.** Like the fixed-window counter, but at query time also consider the previous window with a time-based weight: `effective_count = prev_window * (1 - elapsed_in_current / window_size) + current_window`.

```text
If window = 60s and we are 15s into the current window:
effective = prev_count * (1 - 15/60) + cur_count
          = prev_count * 0.75 + cur_count
```

**Why it's interesting.**
- Constant memory per subject (two counters).
- Much better boundary behavior than fixed-window; a burst across the boundary is dampened by the weighted contribution of the previous window.
- Cheap: one subtract, one multiply, one compare.

**Where it falls short.**
- Still an approximation. A determined adversary who models the weighting can still get a small super-limit burst at the boundary.
- Not as intuitive to operators as token-bucket. "Why did my 60th request fail 45 seconds into the window?" takes explaining.

**Real-world systems that use it.** Cloudflare's more recent rate-limit design (and a common pattern for "approximate but good enough" at scale). DiscordGo and Cloudflare both document approximations along these lines.

### 3.4. Token bucket

**What it is.** Each subject has a bucket of size `capacity` that refills at `rate` tokens per second. A request consumes `cost` tokens; if the bucket has enough, the request proceeds and the bucket is debited. If not, the request is denied (or enqueued for a configurable duration).

State per subject: `(tokens: f64, last_refill: Instant)`. On each check:

```rust
let elapsed = now - last_refill;
tokens = (tokens + elapsed.as_secs_f64() * rate).min(capacity);
last_refill = now;
if tokens >= cost { tokens -= cost; Allow } else { Deny }
```

**Why it's interesting.**
- **Separate burst and sustained semantics.** `capacity` is the burst allowance; `rate` is the sustained limit. The two knobs match what operators actually want to configure.
- **Constant memory per subject.** One float, one instant. Cache-friendly.
- **No log, no scan, no GC.** O(1) per check.
- **Natural cost parameter.** Cost = 1 for RPM limiting; cost = `tokens_in_prompt` for TPM limiting. This is a killer feature for LLM workloads where "rate" is genuinely multi-dimensional.
- **Lazy refill.** We do not tick; we compute tokens-since-last-check at each check. No per-subject timer.
- **Maps cleanly to a trait.** The state is small; the operation is pure given the state. Easy to provide multiple impls (in-proc, Redis, Dragonfly).

**Where it falls short.**
- **Subtle at boundaries.** If `cost > capacity`, a single request can never succeed — this is usually desired but surprising the first time.
- **Floating-point debates.** `f64` for tokens is precise enough for any practical rate; some codebases prefer integer microtokens to avoid FP drift. Minor.

**Real-world systems that use it.** Nginx's `limit_req_zone` leaks a bit of sophistication on top of this; AWS API Gateway; Stripe's API; countless Redis Lua scripts. The default shape of rate limiting in the industry.

### 3.5. Leaky bucket (as a queue)

**What it is.** A FIFO queue of fixed depth drained at a constant rate. Requests push onto the queue; if the queue is full, they are rejected. Requests leave the queue when dequeued by a worker that runs at the configured drain rate.

**Why it's interesting.**
- Naturally shapes outbound traffic at a constant rate. A spikey input becomes a smooth output.
- Maps well to a protect-the-backend posture: the backend sees a smooth stream regardless of client burstiness.

**Where it falls short.**
- **Adds latency to admitted requests.** A request admitted while the queue is non-empty has to wait its turn. For streaming LLM workloads where TTFT is the headline metric, this is the wrong shape.
- **State is larger.** The queue itself is an object, not a pair of floats.
- **Confusing when "leaky bucket" is colloquially used to mean "token bucket."** The two are different primitives with different semantics; the name is shared because both involve a bucket that drains over time.

**Real-world systems that use it.** Network equipment (traffic shaping on switches and routers); some message queue admission controls. Less common at the application layer.

### 3.6. GCRA (Generic Cell Rate Algorithm)

**What it is.** Originally from ATM networks. Maintains a single `theoretical_arrival_time` per subject. On each request, compare `now` to `theoretical_arrival_time - tau` (where `tau` is the burst tolerance). If `now` is after the threshold, allow and advance `theoretical_arrival_time` by the per-token cost. Otherwise, deny. It is a mathematically equivalent reformulation of the token bucket with one scalar of state instead of two.

```text
Conceptually:
    T (theoretical arrival time) — advances by cost on each accept
    tau (burst tolerance)       — configured
    allow iff  now + tau >= T
```

**Why it's interesting.**
- **Smallest possible state.** One integer timestamp per subject.
- **Equivalent semantics to token-bucket** with a cleaner mathematical formulation — popular in papers and in systems that want to prove properties.
- **Great for distributed impls.** A single timestamp is a tidy CAS target in Redis or any shared log.
- **Clean Lua script.** The reference Redis GCRA (by Brandur Leach at Stripe) is ~40 lines and has become a reference implementation for distributed rate limiting.

**Where it falls short.**
- **Slightly less operator-legible.** "The bucket refills at rate X with capacity Y" is pedagogically simpler than "we maintain a theoretical arrival time."
- **The equivalence with token-bucket is often not appreciated**, leading to ecosystems where both exist in parallel for essentially the same purpose.

**Real-world systems that use it.** Stripe's `throttled` library for Redis; Cloudflare's rate limiter; many systems that cite or vendor Brandur Leach's reference Lua script.

### 3.7. Distributed impls (the future)

A family of extensions of any of the above (most commonly token-bucket or GCRA) across replicas. Four common variants:

#### 3.7.1. Central counter in Redis / Dragonfly

One canonical store for the state; every replica reads/writes it atomically.

- **Redis `INCR` + `EXPIRE`** on a fixed-window key. Simple but suffers from the boundary-burst failure (§3.1).
- **Redis Lua script** implementing token-bucket or GCRA atomically. The industry standard for "we want distributed and we want exact."
- **Dragonfly** for the same (drop-in Redis replacement with better CPU scaling in the sharded case).

Wins: exact, coherent. Costs: every request is now a network hop; contention on a single key under hot load; partial failure (Redis unreachable) is a new failure domain.

#### 3.7.2. Sharded local + periodic gossip

Each replica enforces its share of the limit locally (limit / N_replicas) with occasional gossip to rebalance.

Wins: no per-request network hop; resilient to state-service outages. Costs: *approximate* — at any instant the shares may be off; requires a gossip mechanism; hard to reason about during scale events.

#### 3.7.3. Consistent-hash + sticky subject routing

Route each subject to a single replica; rate limit on that replica only. The limiter itself is local; the *request ingress* is distributed.

Wins: exact, no inter-replica coordination. Costs: requires a front-door LB that consistent-hashes by subject; breaks under replica add/remove unless bounded-load consistent hashing is used. Relevant chapter: `hashing/ch07 (consistent hashing)`.

#### 3.7.4. CRDT-backed counter (research)

Operational-transform or CRDT-style counters that converge. Academic interest; rarely a production choice today.

**Our posture on §3.7.** Not in `v1.0`. The `RateLimiter` trait accepts any of these as a future impl. Options `021` catalogs them so a future contributor can see the full design space and pick the right compromise for their deployment.

## 4. Tradeoff matrix

| Property | Fixed | Slide-log | Slide-ctr | Token-bkt | Leaky | GCRA | Distributed (Redis/Lua) | Why it matters |
|---|---|---|---|---|---|---|---|---|
| State per subject | 1 int | O(limit) | 2 ints | 2 nums | queue | 1 int | same as in-proc + replication | Memory budget at high subject count. |
| Hot-path CPU | O(1) | O(limit) | O(1) | O(1) | O(1) push | O(1) | O(1) + RTT | Directly hits `NFR-P07`. |
| Burst semantics | none | exact | weighted | configurable | shaping | configurable | depends | Operators want "allow bursts up to X". |
| Boundary bursts | yes | no | small | no | no | no | no (if Lua-atomic) | The canonical failure mode. |
| Shapes vs limits | limits | limits | limits | limits (pass-through) | shapes (delays) | limits | either | LLM streaming cares about TTFT — shaping hurts. |
| Cost parameter (TPM) | awkward | awkward | awkward | natural (cost=tokens) | awkward | natural | natural | LLM rate limits are in tokens, not just requests. |
| Distributed extension path | trivial but wrong | painful | possible | Lua-atomic fits | queue-replication hard | Lua-atomic fits | native | Trait must accommodate this future. |
| Operator legibility | high | medium | medium | high | medium | medium-low | inherits | "Can an on-call reason about it at 3am?" |
| Existing reference impls in Rust | everywhere | rare | `governor`-ish | `governor`, `leaky-bucket` | `leaky-bucket` | `governor::quota` | `redis-throttled`, `tonic-limit` | Reduces our implementation cost if we pick one the ecosystem supports. |
| Fits Riftgate trait shape | yes | yes | yes | yes | awkward (queue is stateful) | yes | yes | Pluggability is a Riftgate principle. |

## 5. What the source-systems chapters say

From `systems/ch12 (system design patterns)`, the resilience-patterns section frames rate limiting as a protection primitive in the same family as circuit breakers: both convert a *"downstream failure mode"* into a *"local policy decision"*. The chapter's advice against bolt-on implementations applies directly: if we ever want a rate limiter that collaborates with our circuit breaker (see [Options `011`](011-circuit-breaker.md)), they both need to live inside the same kernel and speak the same event vocabulary.

From `systems/ch08 (pub/sub messaging — backpressure as policy)`, the backpressure-as-policy framing is directly relevant: a rejected request is a form of backpressure. A rate limiter is a synchronous, predictable back-pressure source; we can compose it with the queue-depth-based backpressure of Options [`012`](012-backpressure.md) without inventing a new abstraction.

From `hashing/ch07 (consistent hashing)`, the key insight for a future distributed impl: if we ever want to shard rate-limit state across replicas, consistent hashing by subject is the cleanest approach because it lets us avoid cross-replica coordination for the common case. Bounded-load consistent hashing (Mirrokni et al., Google 2016) handles the replica-add/remove case without thundering herds.

From `trees/ch04 (heaps and priority queues)`, the relevant idea is *which* request gets throttled when multiple are at the limit boundary. A FIFO rejection order is the default; a priority-aware rejection (keep premium requests alive, drop batch first) is the direction Options [`022`](README.md) (fairness scheduling) would take us.

From `systems/ch04 (lock-free structures)` (indirectly): the in-proc token-bucket is a textbook lock-free candidate — a single `AtomicU64` packing `(tokens_scaled, last_refill_nanos)` admits a compare-exchange loop with no mutex. We will implement it exactly this way and cite the chapter in the LLD.

## 6. Recommendation

**For `v1.0`: ship a single impl — an in-proc, lock-free token bucket — behind a `RateLimiter` trait that is shaped to accept the distributed impls of §3.7 without breakage.**

Concretely:

1. The trait is defined in `riftgate-core`:
   ```rust
   pub trait RateLimiter: Send + Sync {
       fn check(&self, subject: &SubjectKey, cost: u32) -> LimitDecision;
   }

   pub enum LimitDecision {
       Allow,
       Deny { retry_after: Duration },
   }

   pub struct SubjectKey {
       pub tenant: TenantId,
       pub route: RouteId,
       pub backend: Option<BackendId>,
   }
   ```
2. The default impl, `TokenBucketLimiter`, uses a sharded `DashMap<SubjectKey, AtomicBucketState>` internally, citing `systems/ch04 (lock-free structures)`. Sharding is by subject hash to avoid a single contended map.
3. The trait signature is deliberately designed so a future `RedisGcraLimiter` can implement it without changing callers. `check` returns `LimitDecision` rather than `bool` so the distributed impl can surface `Retry-After` semantics without bolting on a second method.
4. Configuration knobs (per-route): `rate_per_sec`, `burst`, `cost_fn` (default: `1 per request`; optional: `token_count(prompt)` for TPM limiting).
5. Denied requests return `429 Too Many Requests` with `Retry-After`. Denied requests are counted as first-class in OTel (so operators can see throttle pressure).

### Conditions under which we'd revisit

- If we discover (via user pull, not speculation) that multi-replica rate-limit coherence is a blocker for real deployments, a `RedisGcraLimiter` impl lands behind a `rate-limit-redis` feature flag. It will not become the default — per-instance remains the Riftgate posture.
- If the hot-path CPU of the in-proc impl ever exceeds [`NFR-P07`](../01-requirements/non-functional.md) under realistic load, we re-examine whether a smaller-state GCRA formulation is worth the refactor.
- If an operator persona (e.g. a cloud-gateway-as-a-service deployment) emerges where rate limits are commercial SKUs, the priority of a distributed impl goes up.

## 7. What we explicitly reject

- **Fixed-window counter as the default.** Boundary bursts are the one failure mode a rate limiter exists to prevent. We will not ship this even as a "simpler option" — the cognitive tax of explaining why it's wrong outweighs the implementation savings.
- **Sliding-window log.** O(limit) memory per subject on the hot path is the wrong shape for a gateway that may carry ten thousand subjects.
- **Leaky bucket as a queue.** Shaping requests via an admission delay is incompatible with LLM-streaming TTFT guarantees ([`NFR-P05`](../01-requirements/non-functional.md)). If we ever need traffic shaping, it belongs in a separate subsystem, not in the request-rejection path.
- **Default distributed backend (Redis / Dragonfly).** Preserves [`NFR-C02`](../01-requirements/non-functional.md) ("no third-party paid services in the data path"). Cataloged as a future extension of the same trait.
- **Bolt-on rate limiter as a separate binary.** A gateway whose rate limiter lives outside the kernel cannot collaborate with the circuit breaker (see Options [`011`](011-circuit-breaker.md)) or the backpressure policy (see Options [`012`](012-backpressure.md)). These three primitives must share a common event vocabulary, which means they share a kernel.
- **Priority-aware rejection as part of this Options doc.** That is the scope of Options [`022`](README.md) (fairness scheduling), gated on the `v0.2` retro. If pursued, it layers on top of `RateLimiter` rather than inside it.

## 8. References

1. Poul-Henning Kamp, *The Rules for Building a Rate Limiter* (a collection of practitioner posts) — <https://www.varnish-cache.org/>
2. Brandur Leach, *Rate Limiting with GCRA and Redis* — <https://brandur.org/rate-limiting>
3. Jeff Dean, *Numbers Every Programmer Should Know* — standard reference for latency-budget conversations around rate-limit hot-path cost.
4. Vahab Mirrokni, Mikkel Thorup, Morteza Zadimoghaddam, *Consistent Hashing with Bounded Loads* (Google, 2016) — <https://arxiv.org/abs/1608.01350>
5. Rust crates: [`governor`](https://docs.rs/governor/), [`leaky-bucket`](https://docs.rs/leaky-bucket/), [`tower::limit`](https://docs.rs/tower/latest/tower/limit/).
6. Cloudflare, *How we built rate limiting capable of scaling to millions of domains* (various engineering blog posts).
7. Nginx, [`limit_req_zone` documentation](http://nginx.org/en/docs/http/ngx_http_limit_req_module.html) — practical production reference.
8. Riftgate source-systems chapter `Ch12 (system design patterns)`
9. Riftgate source-systems chapter `Ch8 (pub/sub messaging — backpressure as policy)`
10. Riftgate sibling-book chapter `hashing/ch07 (consistent hashing)`
11. Riftgate sibling-book chapter `trees/ch04 (heaps and priority queues)`
12. Riftgate source-systems chapter `Ch4 (lock-free structures)` (for the lock-free in-proc impl)
