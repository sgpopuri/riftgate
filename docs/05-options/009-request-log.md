# 009. Request log

> **Status:** `recommended` — `v0.2` ships a per-shard append-only file WAL behind the `WAL` trait already declared in `riftgate-core`; embedded LSM stores (RocksDB / sled) and SQLite-WAL are catalogued and rejected for v0.2. See [ADR `0013`](../06-adrs/0013-append-only-file-wal.md).
> **Foundational topics:** write-ahead logging (ARIES — Mohan et al., 1992), LSM trees (O'Neil et al., 1996), group-commit fsync, append-only file design (Kafka log segments, BookKeeper), `fsync` / `fdatasync` semantics (Pillai et al., *Crash Consistency*, 2014)
> **Related options:** [`019 — replay-eval`](README.md) (the replay tooling that reads this log), [`013 — observability sink`](013-observability-sink.md) (the WAL is *not* a metrics sink), [`026 — MCP orchestration`](026-mcp-orchestration.md) (audit events land in the WAL)
> **Related ADR:** [ADR `0013`](../06-adrs/0013-append-only-file-wal.md)

## 1. The decision in one sentence

> What is the on-disk shape of Riftgate's request log — the durable record that powers replay, audit, and post-incident analysis — and which storage substrate do we use to implement it?

## 2. Context — what forces this decision

The `WAL` trait is declared in `riftgate-core` since v0.1 (per [ADR `0009`](../06-adrs/0009-rate-limiter-trait-in-proc-only.md)'s sibling pattern for impl-deferred traits), with no impl shipped. Three v0.2 requirements force the impl:

- **[`FR-105`](../01-requirements/functional.md)** — the gateway records every request to a durable log so failed runs can be replayed.
- **[`NFR-OBS06`](../01-requirements/non-functional.md)** — every request envelope (method, path, headers minus secrets, body framed by SSE events where applicable) is recoverable from the WAL for at least 24h of operation.
- **[`NFR-A04`](../01-requirements/non-functional.md)** — WAL write cost on the hot path is bounded by the `Durability::Async` budget; `Fsync` is opt-in for audit-grade entries (e.g. [MCP](026-mcp-orchestration.md) `tools/call` decisions in v0.5).

The v0.5 MCP capability broker also lands its `McpAuditEvent` entries in this WAL — the audit log is the same log; the trait does not bifurcate. Whatever shape we pick must accommodate audit-grade `Fsync` entries alongside the bulk `Async` request envelopes without a separate codepath.

The forces summarised: low-overhead `Async` appends on the hot path; opt-in `Fsync` for audit; group-commit batching to amortise the `fsync` cost; per-shard segment files to keep the `MpmcQueue::push` of a WAL entry contention-free; readable from a separate process (`riftgate-replay`) without touching the gateway.

## 3. Candidates

### 3.1. Per-shard append-only file with group-commit fsync

**What it is.** One segment file per shard, opened with `O_APPEND`. Writes batch into a per-shard ring buffer that a dedicated WAL-flusher thread drains every `flush_interval_ms` (or when the buffer crosses a high-water byte threshold), calling `write()` then optionally `fdatasync()`. Segment rolls over at `segment_size_max`. Filename convention `seg-{shard}-{seqno:020}.wal`.

```text
crates/riftgate/wal/
├── seg-0-00000000000000000001.wal     (active, shard 0)
├── seg-0-00000000000000000000.wal     (rolled, shard 0)
├── seg-1-00000000000000000001.wal     (active, shard 1)
└── ...
```

Each entry is a length-prefixed framed record:

```text
[u32 length][u8 durability][u8 entry_kind][u64 timestamp_nanos][u64 entry_id][bytes payload]
```

**Why it's interesting.**
- **Linear writes, sequential disk.** Cheapest possible IO pattern; modern NVMe sees this as a single large stream.
- **Per-shard segment files** mean no cross-shard contention on the WAL write path; each shard's WAL is independent.
- **Group-commit fsync** amortises the syscall cost — Kafka, BookKeeper, and LMDB all use this pattern.
- **Mixed durability is natural.** `Async` entries return after the ring-buffer write; `Fsync` entries return after the next group-commit. The trait's `Durability` parameter is the API surface.
- **Replayable by a separate process.** The replay CLI opens the segment dir read-only; no IPC with the gateway.
- **Operationally legible.** `ls -la wal/` shows the on-disk shape; `xxd | less` shows the framing. No magic.
- **No external dependency.** Pure `std::fs` + `nix` for `fdatasync`.

**Where it falls short.**
- No indexing — finding entry `42` requires a scan from the segment start. Acceptable: replay is sequential; audit query patterns are "give me last hour" which is segment-level granularity.
- Segment rollover policy is its own knob (`segment_size_max` bytes).
- Operator must manage retention (delete old segments) — we ship a small `riftgate-replay prune` command for this.

**Real-world systems that use it.** Kafka log segments; Apache BookKeeper; etcd's WAL; PostgreSQL WAL segments; LMDB.

### 3.2. Embedded RocksDB

**What it is.** Use RocksDB (or sled) as the WAL store. Each entry becomes a key-value pair with the entry-id as the key.

**Why it's interesting.**
- Battle-tested at scale.
- Comes with compaction, retention, and indexing primitives.
- Has a column-family abstraction that maps cleanly to "request envelopes" vs "audit events."

**Where it falls short.**
- **Heavy.** RocksDB is ~30 MB of compiled C++; the `rust-rocksdb` crate brings in a non-trivial build dependency.
- **Wrong shape for our access pattern.** We almost never read individual entries by id; we replay segments sequentially. LSM compaction is buying us nothing and costing us write amplification.
- **Operational footprint.** RocksDB has its own background threads, its own tunables (block cache, write-buffer size, compaction levels), its own crash-recovery story. Each is a thing to learn.
- **WAL inside a WAL.** RocksDB itself has an internal WAL; we'd be writing through two of them.

**Real-world systems that use it.** TiKV's WAL; CockroachDB's storage layer. Production-grade, but for *workloads that need point lookups and range scans*, which the request log does not.

### 3.3. SQLite in WAL mode

**What it is.** A single SQLite database file in WAL journaling mode. Each request becomes a row.

**Why it's interesting.**
- Ubiquitous. Every operator has `sqlite3` on their box.
- SQL queries are nice for ad-hoc audit ("which tenant sent the most 429s yesterday?").
- ACID, including row-level integrity.

**Where it falls short.**
- **Single-writer.** SQLite serialises writes; multi-shard appends contend on a single mutex. Defeats the per-shard isolation that the rest of the gateway works hard to preserve.
- **Variable per-row cost.** A B-tree insert has a long worst-case tail; the WAL hot path benefits from constant-time appends.
- **Body storage as BLOBs** in SQLite is awkward at scale; large bodies inflate the file and slow scans.

**Real-world systems that use it.** Plenty of single-writer audit logs. Not a multi-shard gateway shape.

### 3.4. Custom LSM tree

**What it is.** Implement an LSM-like store ourselves (in-memory memtable, sealed memtables, sorted on-disk segments, periodic compaction).

**Why it's interesting.** Maximum control; production-grade LSM patterns are well documented (O'Neil et al., 1996).

**Where it falls short.**
- The complexity-per-payoff ratio is awful for our access pattern. We do not need point lookups or range scans; we need sequential append + sequential read.
- It is a v1.0-or-later commitment in implementation cost; not a v0.2 deliverable.

**Real-world systems that use it.** Cassandra, ScyllaDB, RocksDB itself. Not gateway-WAL workloads.

### 3.5. No WAL (don't ship one)

**What it is.** Defer the request log; rely on the upstream's logs and OTel traces for post-incident analysis.

**Why it's interesting.** No code.

**Where it falls short.**
- Violates `FR-105` and `NFR-OBS06`. The v0.2 milestone gates on `riftgate-replay` working end-to-end.
- Upstream logs do not capture filter-chain or routing decisions; OTel spans do not carry full bodies. Neither is replayable.

## 4. Tradeoff matrix

| Property | 3.1 Per-shard append | 3.2 RocksDB | 3.3 SQLite WAL | 3.4 Custom LSM | 3.5 None | Why it matters |
|---|---|---|---|---|---|---|
| Hot-path cost (Async) | ring-buffer write | LSM put + internal WAL | mutex + B-tree insert | ring + memtable | n/a | NFR-P07-adjacent |
| `Fsync` cost (per-batch) | one syscall per shard per interval | inherits group-commit | one syscall (serialised) | implement ourselves | n/a | NFR-A04 |
| Per-shard isolation | yes | shared (mutex inside) | no (serialised) | implementation-dep. | n/a | Per-shard scheduler stays coherent |
| Sequential read for replay | trivial | engine API | SQL | implement | n/a | `riftgate-replay` simplicity |
| Build dependency | std + nix | librocksdb (C++) | libsqlite3 | std + nix | none | Build matrix |
| Operator legibility | high | medium-low | high (sqlite3 CLI) | low | n/a | "What's in the WAL?" |
| Mixed `Async`/`Fsync` natural | yes (per-entry flag) | yes (RocksDB sync write) | no (write mode global) | yes | n/a | Audit + request envelope share one log |
| Disk format stability | trivial (we own it) | engine-versioned | sqlite-versioned | we own it | n/a | Forward-compat for `riftgate-replay` |
| Implementation cost in v0.2 | medium | low-to-medium | low | very high | none | v0.2 capacity |

## 5. Foundational principles

**ARIES write-ahead logging (Mohan et al., 1992).** The canonical reference for write-ahead logs in transactional systems. The two load-bearing ideas Riftgate inherits: (1) the log is the source of truth — durable before the in-memory state is acknowledged; (2) recovery is replay from the log. The Riftgate WAL is simpler than ARIES (no UNDO records; no per-page LSNs) because we are logging request envelopes, not B-tree page updates. But the discipline — log first, ack second — is the same.

**Group-commit fsync (Hagmann 1987; Mohan & Lindsay 1983).** Calling `fsync` once per request is wasteful when many requests can commit in a single batch. The group-commit pattern — let a coordinator thread `fsync` the union of recent writes and then notify all the waiters — is canonical. Kafka and BookKeeper both rely on it; we adopt the same pattern.

**Per-shard segment files (Kafka log segment lineage).** The decision to write one segment file per shard rather than a single file is the same shared-nothing principle that drives the per-shard scheduler ([ADR `0004`](../06-adrs/0004-per-shard-default-stealing-opt-in.md)). One mutex on a shared WAL file would re-introduce cross-shard contention that the scheduler works to eliminate.

**`fdatasync` over `fsync` for hot-path durability (Pillai et al., 2014).** `fdatasync` skips inode metadata updates that we do not depend on. The Linux file-system literature is consistent: `fdatasync` is the right primitive for append-only WAL on a file whose metadata is not changing per entry. We use `Durability::Fsync` only for audit-grade entries that require directory-entry durability.

**Length-prefixed framed records (RFC 4180-style framing patterns; Kafka, Protocol Buffers).** A `[length][header][payload]` framing is forward-compatible by construction: new header fields are additive within the payload; old readers can skip records they do not understand.

## 6. Recommendation

**For `v0.2`: ship per-shard append-only file WAL with group-commit `fdatasync` per shard. The `FileWal` impl of the `WAL` trait lives in a new `crates/riftgate-replay` crate alongside the replay CLI. Embedded LSM stores and SQLite are catalogued and explicitly rejected for v0.2.**

Concretely:

1. New crate `crates/riftgate-replay/`. Public items:
   - `FileWal` — implements `riftgate-core::wal::WAL`.
   - `riftgate-replay` binary — the CLI (`record`, `replay`, `list`, `prune`).
2. On-disk layout: `wal_dir/seg-{shard:04}-{seqno:020}.wal`. One active segment per shard; rolled segments are read-only.
3. Entry frame: `[u32 length][u8 durability][u8 entry_kind][u64 timestamp_nanos][u64 entry_id][bytes payload]`. Length excludes the length prefix itself. `entry_kind` enumerates `RequestEnvelope`, `ResponseEnvelope`, `SseEventBoundary`, `RouteDecision`, `RateLimitDenial`, `CircuitTransition`, `McpAuditEvent` (the latter lands in v0.5).
4. Per-shard flusher thread: bounded MPSC ring (`flush_buffer_bytes`, default 1 MiB) drains every `flush_interval_ms` (default 5 ms) or when the buffer crosses high-water. `Async` entries return immediately after the ring-buffer write; `Fsync` and `FdataSync` entries park until the next group-commit completes the syscall.
5. Segment rollover at `segment_size_max` (default 64 MiB).
6. Retention: `riftgate-replay prune --older-than 24h` or `--max-total-bytes 10GiB`.
7. Config (per [Options `015`](015-config-model.md)):

   ```toml
   [wal]
   dir = "/var/lib/riftgate/wal"
   segment_size_max_mib = 64
   flush_interval_ms    = 5
   flush_buffer_bytes   = 1048576
   default_durability   = "fdatasync"   # one of "async", "fdatasync", "fsync"
   ```

8. Telemetry: `riftgate.wal.appended` (counter labelled by `entry_kind`, `durability`), `riftgate.wal.bytes_appended` (counter), `riftgate.wal.fsync_latency` (histogram), `riftgate.wal.segment_rolled` (counter), `riftgate.wal.flusher_buffer_depth` (gauge).

### Conditions under which we'd revisit

- If a real deployment shows `fsync_latency` p99 above the budget under realistic write rates, we revisit the per-shard flush-interval default and / or move to async io_uring submission for the fsync ([ADR `0002`](../06-adrs/0002-start-on-epoll.md) opt-in).
- If audit query patterns ("give me all 429s for tenant X yesterday") become a recurring operator need, we add a sidecar indexer (separate process; reads segments and builds a queryable index in DuckDB or similar) rather than changing the WAL format.
- If on-disk format evolution becomes painful, the `[length][header]` framing already supports forward-compatible additions; only a true breaking change requires a new ADR.

## 7. What we explicitly reject

- **RocksDB or sled (3.2).** Wrong shape for our access pattern (we don't need point lookups or range scans); heavy build dependency.
- **SQLite in WAL mode (3.3).** Single-writer serialisation defeats per-shard isolation; awkward for large bodies.
- **Custom LSM (3.4).** Complexity-per-payoff ratio is wrong for v0.2.
- **No WAL (3.5).** Violates `FR-105` and `NFR-OBS06`.
- **A shared single-file WAL across shards.** Reintroduces cross-shard contention.
- **Synchronous `fsync` per request as the default.** `fdatasync` group-commit is the v0.2 default; `Fsync` per-entry is opt-in for audit-grade entries.
- **Pluggable WAL formats in v0.2.** The trait is already pluggable; v0.2 ships one impl.

## 8. References

1. C. Mohan, Don Haderle, Bruce Lindsay, Hamid Pirahesh, Peter Schwarz, *ARIES: A Transaction Recovery Method Supporting Fine-Granularity Locking and Partial Rollbacks Using Write-Ahead Logging* (ACM TODS, 1992) — <https://www.vldb.org/conf/1989/P017.PDF>.
2. Patrick O'Neil, Edward Cheng, Dieter Gawlick, Elizabeth O'Neil, *The Log-Structured Merge-Tree (LSM-Tree)* (Acta Informatica, 1996).
3. Robert Hagmann, *Reimplementing the Cedar File System Using Logging and Group Commit* (SOSP 1987).
4. C. Mohan, Bruce Lindsay, *Efficient Commit Protocols for the Tree of Processes Model of Distributed Transactions* (PODC 1983).
5. Thanumalayan Sankaranarayana Pillai et al., *All File Systems Are Not Created Equal: On the Complexity of Crashing the Application File System Interface* (OSDI 2014).
6. Apache Kafka, [storage internals](https://kafka.apache.org/documentation/#log) — log segment design.
7. Apache BookKeeper, [architecture](https://bookkeeper.apache.org/docs/getting-started/concepts) — ledger / journal design.
8. etcd, [WAL package source](https://github.com/etcd-io/etcd/tree/main/server/storage/wal).
9. PostgreSQL, [WAL internals chapter](https://www.postgresql.org/docs/current/wal-internals.html).
10. LMDB, [design overview](http://www.lmdb.tech/doc/).
11. `fdatasync(2)` Linux man page — <https://man7.org/linux/man-pages/man2/fdatasync.2.html>.
