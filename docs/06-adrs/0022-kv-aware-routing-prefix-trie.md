# ADR 0022. KV-cache-aware routing via an in-tree prefix trie with xxHash3-64 byte-hashing

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [025-v03-routing-strategies](../05-options/025-v03-routing-strategies.md)
> **Deciders:** Sriram Popuri

## Context

[Options `010`](../05-options/010-routing-strategy.md) catalogued KV-cache-aware routing as a v0.3 deliverable; [Options `025`](../05-options/025-v03-routing-strategies.md) revisits the candidates with the v0.3 milestone open. Three shapes were evaluated: in-tree prefix trie with byte-hash keys, LMCache delegation, and bounded-load consistent hashing. LMCache delegation introduces a ~ms LAN round-trip on the routing hot path (defeating `NFR-P11`'s 50µs budget) and creates a hard dependency on a non-Riftgate service; consistent hashing loses the longest-prefix property that makes KV-cache hits work. The in-tree trie satisfies the hot-path budget, has no external dependency, and matches the longest-prefix semantics the cache-hit objective requires.

## Decision

**`v0.3` ships `KvAwareRouter<R>` in `crates/riftgate-router` as a decorator over an inner `Router` (typically `WeightedRandomRouter`), using a prefix trie keyed by chunked xxHash3-64 hashes of the request's prompt bytes, with an LRU-bounded entry count and configurable prefix normalisation. LMCache delegation and bounded-load consistent hashing are catalogued and rejected for v0.3.**

- Hash function: xxHash3-64 (`xxhash-rust` crate, no_std-friendly).
- Default chunk size: 64 bytes per trie level.
- Default LRU capacity: 100,000 entries.
- Default prefix normalisation: `trim_trailing_whitespace` (alternative options: `nfkc`, `none`).
- Default `min_prefix_bytes_to_route`: 256 (shorter prompts skip KV routing and delegate to the fallback router).
- The trie is shared across shards behind an `Arc<RwLock<PrefixTrie>>`; reads are wait-free on the hot path, writes (LRU touches and inserts on `on_response`) take the write lock briefly.
- Composes under `CircuitBreakerArbiter`: the binary wires `CircuitBreakerArbiter::new(KvAwareRouter::new(WeightedRandomRouter::new(...)))`. When the trie picks a backend that the breaker has marked Open, `KvAwareRouter` falls back to its inner router.
- The `Router` trait surface in `riftgate-core` is unchanged.

## Consequences

- **Positive:**
  - No external dependency; operators with mixed-backend fleets (vLLM + TGI + OpenAI proxy chains) benefit equally.
  - Hot-path cost stays inside `NFR-P11` (50µs at p99): hash is O(prefix_length / 64); trie walk is O(depth).
  - Real cache-hit semantics via longest-prefix matching (the property a hash-only consistent-hash scheme cannot deliver).
  - Composes cleanly with the breaker decorator and `HedgedRouter` ([ADR `0023`](0023-hedged-requests-p99-triggered.md)) — both new routers stack on top of `WeightedRandomRouter`.
  - The trait surface remains stable; a future `LmcacheRouter` impl can land without breaking callers.
- **Negative / accepted tradeoffs:**
  - Whitespace and Unicode normalisation sensitivity is documented; default `trim_trailing_whitespace` covers the common case. Operators with non-ASCII workloads enable `nfkc`.
  - Hash-collision residual risk at xxHash3-64 is negligible at 100k entries; documented in the LLD with the back-of-envelope math.
  - No cross-replica state in v0.3: two Riftgate replicas behind an L4 LB have independent tries. Operators wanting cross-replica consistency front Riftgate with a prefix-aware L4 LB or wait for the future LMCache-delegated impl.
  - Tokenizer-accurate routing (which would slightly improve hit rates) is rejected for v0.3 because tokenising on the hot path exceeds the latency budget. Revisit at v1.0 with measured data.
- **Future work this enables:**
  - `LmcacheRouter` lands as an additional impl behind the same trait if operator demand grows; both routers coexist; operators pick one.
  - Tokenizer-accurate KV routing becomes a clean v1.0+ option with measurement-driven justification.
  - Cross-replica consistency can be added as a future option once a session-affinity layer ([`docs/04-design/lld-routing.md`](../04-design/lld-routing.md) open question) is selected.
- **Future work this forecloses (until superseded):**
  - Riftgate will not bind to LMCache as the only KV-aware path.
  - Riftgate will not tokenise on the routing hot path in v0.3.
  - Riftgate will not implement KV-aware routing as a WASM filter (routing is too hot-path for the WASM dispatch cost; routing remains in-tree Rust).

## Compliance

- `KvAwareRouter` lives in `crates/riftgate-router/src/kv_aware.rs` and implements the existing `Router` trait.
- `crates/riftgate-router/tests/kv_aware_hit_rate.rs` asserts the trie hit-rate on a synthetic prompt-prefix workload meets a documented floor (≥ 60% on a 50%-shared-prefix synthetic distribution).
- `crates/riftgate-router/tests/kv_aware_lru_eviction.rs` asserts the LRU bound is respected and eviction order is correct.
- `crates/riftgate-router/tests/kv_aware_breaker_interaction.rs` asserts that an Open backend named by the trie falls back to the inner router.
- A criterion bench at `crates/riftgate-router/benches/kv_aware_route.rs` measures per-`route()` cost at N ∈ {2, 8, 32} eligible backends; CI fails if p99 exceeds 50µs on the reference host.
- Trie configuration changes via TOML (`prefix_chunk_bytes`, `max_trie_entries`, `prefix_normalisation`, `min_prefix_bytes_to_route`) do **not** require a new ADR; changing the hash function does.

## Notes

- The decision to hash bytes rather than tokens is the load-bearing trade. We give up a few percentage points of hit-rate (some tokenizer-divergent inputs would route to the same trie node and a tokenizer-accurate scheme would split them) in exchange for staying inside the 50µs hot-path budget. The trade is documented in [Options `025` §3.A.1](../05-options/025-v03-routing-strategies.md).
- The LRU capacity default (100k entries) is sized for a single Riftgate process handling on the order of millions of distinct prefixes per day; operators with larger prefix cardinalities raise the cap. Memory cost at 100k entries with 64-byte hash chunks is ~10MB, comfortably below the allocator footprint envelope.
- `RwLock<PrefixTrie>` is chosen over a lock-free design for v0.3 because the LRU touch path is short and the read path dominates by orders of magnitude. If contention becomes measurable, a lock-free shape (per-shard tries with periodic merge, or a concurrent radix tree like the one in `crossbeam-skiplist`) is a future option.
