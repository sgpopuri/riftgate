# ADR 0013. Per-shard append-only file WAL with group-commit fdatasync; RocksDB and SQLite rejected

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [009-request-log](../05-options/009-request-log.md)
> **Deciders:** Sriram Popuri

## Context

`FR-105` and `NFR-OBS06` commit `v0.2` to a durable, replayable request log. The `WAL` trait has lived in `riftgate-core` since v0.1 with no impl. Full exploration of candidates (per-shard append-only file, embedded RocksDB, SQLite WAL mode, custom LSM, none) and the tradeoff matrix live in [Options `009`](../05-options/009-request-log.md).

The forces summarised: low-overhead `Async` appends on the hot path; opt-in `Fsync`/`FdataSync` for audit-grade entries (including the v0.5 [MCP](../05-options/026-mcp-orchestration.md) `tools/call` decisions); per-shard isolation matching the per-shard scheduler ([ADR `0004`](0004-per-shard-default-stealing-opt-in.md)); sequential read by a separate `riftgate-replay` process; no heavy external dependencies.

## Decision

**`v0.2` ships `FileWal` in a new `crates/riftgate-replay` crate: per-shard segment files under a configured directory, length-prefixed framed records, a per-shard flusher thread that group-commits with `fdatasync`, mixed `Async`/`Fdatasync`/`Fsync` durability honored per entry. The same crate ships the `riftgate-replay` CLI (`record`, `replay`, `list`, `prune`).**

- On-disk layout: `wal_dir/seg-{shard:04}-{seqno:020}.wal`; one active segment per shard; rolled segments read-only.
- Frame: `[u32 length][u8 durability][u8 entry_kind][u64 timestamp_nanos][u64 entry_id][bytes payload]`.
- `entry_kind` enumerates `RequestEnvelope`, `ResponseEnvelope`, `SseEventBoundary`, `RouteDecision`, `RateLimitDenial`, `CircuitTransition`; reserves `McpAuditEvent` for v0.5.
- Group-commit cadence: every `flush_interval_ms` (default 5 ms) or when the per-shard ring buffer crosses `flush_buffer_bytes` (default 1 MiB).
- Segment rollover at `segment_size_max_mib` (default 64 MiB).
- Config block (per [ADR `0012`](0012-static-toml-env-override-v01.md)):

  ```toml
  [wal]
  dir = "/var/lib/riftgate/wal"
  segment_size_max_mib = 64
  flush_interval_ms    = 5
  flush_buffer_bytes   = 1048576
  default_durability   = "fdatasync"
  ```

## Consequences

- **Positive:**
  - Linear writes / sequential disk; cheapest possible IO pattern on modern NVMe.
  - Per-shard segment files preserve the shared-nothing principle that drives the per-shard scheduler; no cross-shard contention on WAL writes.
  - Group-commit `fdatasync` amortises syscall cost across batches; `Async` entries pay only the ring-buffer write.
  - One log carries both request envelopes and (in v0.5) `McpAuditEvent` entries — no separate audit-log codepath. `Durability::Fsync` is the per-entry opt-in for audit-grade entries.
  - Replay CLI reads segments read-only without any IPC with the gateway; format is operationally legible (`xxd`, `ls -la`).
  - Pure `std::fs` + `nix` for `fdatasync`; no heavy C++ build dependency.
- **Negative / accepted tradeoffs:**
  - No indexing — `WAL::find(entry_id)` would require a segment scan. Acceptable: the v0.2 replay use-case is sequential.
  - Operator must manage retention; `riftgate-replay prune` ships as the tool.
  - Forward-compatible framing requires discipline on the `payload` schema; we document this in the LLD and gate on round-trip tests in CI.
  - `fdatasync` is Linux-first; on macOS the `nix` crate maps to `F_FULLFSYNC` (slower) — dev-only, documented per [ADR `0002`](0002-start-on-epoll.md) macOS-is-dev-only posture.
- **Future work this enables:**
  - The same WAL absorbs `McpAuditEvent` entries in v0.5 without a format break (`entry_kind` is already reserved).
  - A sidecar indexer (DuckDB or similar) can read segments and produce a queryable view without changing the WAL format.
  - `io_uring` submission of WAL writes (per [ADR `0002`](0002-start-on-epoll.md) opt-in) is additive.
- **Future work this forecloses (until superseded):**
  - Riftgate will not embed RocksDB or sled as a WAL backend.
  - Riftgate will not ship a SQLite-based WAL.
  - Riftgate will not ship a shared single-file WAL across shards.
  - Riftgate will not default to `Fsync` on the hot path; `Fdatasync` is the bulk default and `Fsync` is opt-in.

## Compliance

- `crates/riftgate-replay/src/file_wal.rs` is the only `WAL` impl shipped in v0.2.
- `crates/riftgate-replay/tests/file_wal_conformance.rs` covers append, group-commit, segment rollover, mixed-durability ordering, and crash-recovery (open a partial segment, verify last valid entry).
- `crates/riftgate-replay/tests/round_trip.rs` records 100 requests through the v0.2 binary then replays them and asserts byte equality on the replayable envelope.
- `benchmarks/wal/` measures `Async` and `Fdatasync` append cost; budgets named in `docs/04-design/lld-storage.md`.
- Adding a new `WAL` impl with a different on-disk format requires a new ADR; format evolution within the framing is additive and does not.

## Notes

- The decision to put `FileWal` in the same crate as the replay CLI (rather than a separate `riftgate-wal` crate) is a deliberate boundary choice: the WAL impl and the replay tool evolve together; splitting them would mean an internal API between two crates that have one user.
- `Durability::Fsync` is reserved for v0.5 MCP audit entries and any v1.0+ operator opt-in; the v0.2 binary does not use it on its own. We document this so an operator doesn't see `Fsync` as a config option and assume it's the right default.
- The `[u64 entry_id]` field uses a per-shard monotonic counter shifted into a global 64-bit ID space (`shard_id << 56 | local_seqno`). This keeps the global `WalEntryId` unique without a cross-shard counter.
- Retention defaults (24h / 10 GiB) are operator-tunable but not auto-managed in v0.2; a running gateway will not delete its own segments. The retention tool is the operator's lever.
