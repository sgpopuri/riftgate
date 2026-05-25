# 04.d LLD — Storage (Request Log / WAL)

> Append-only request log capturing every (request, response) pair for replay, eval generation, and post-mortem debugging.
>
> Status: **v0.1 ships the trait only** — `WAL` lives in `crates/riftgate-core/src/wal.rs` with no production impl. **v0.2 ships `FileWal` in `crates/riftgate-replay`** per [Options `009`](../05-options/009-request-log.md) and [ADR `0013`](../06-adrs/0013-append-only-file-wal.md): per-shard segment files, length-prefixed framed records, group-commit `fdatasync`, mixed `Async`/`Fdatasync`/`Fsync` durability per entry. The v1.0 replay framework extends the same crate's CLI.

## Purpose

Durably record every request and response so that:

- A crashed instance can be diagnosed by replaying the log.
- Test cases can be generated from production traffic (anonymized).
- A new filter or routing strategy can be evaluated against a captured trace.
- Tail-latency outliers can be reproduced offline.

## Trait surface

```rust
// Sketch
pub trait WAL: Send + Sync {
    fn append(&self, record: Record) -> Result<Lsn>;
    fn flush(&self) -> Result<()>;  // fsync
    fn replay(&self, from: Lsn) -> impl Iterator<Item = Record>;
    fn truncate_before(&self, lsn: Lsn) -> Result<()>;
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `FileWal` | `v0.2` | `riftgate-replay` | Per-shard segment files (`seg-{shard:04}-{seqno:020}.wal`); length-prefixed framing; group-commit `fdatasync` flusher per shard; mixed durability per entry. |
| `RocksWal` | rejected | n/a | Wrong shape for our access pattern; see [ADR `0013`](../06-adrs/0013-append-only-file-wal.md). |
| `NullWal` | `v0.1` | `riftgate-core` | No-op for benchmarks and dev. |

Decision rationale: [Options `009` (request log)](../05-options/009-request-log.md) and [ADR `0013`](../06-adrs/0013-append-only-file-wal.md).

Foundational principles: write-ahead logging and ARIES-style crash recovery (Mohan et al., *ARIES*, ACM TODS 1992); group-commit fsync (Hagmann 1987; Mohan & Lindsay 1983); per-shard segment files (Kafka log-segment lineage); `fdatasync` over `fsync` for append-only WAL (Pillai et al., 2014).

## Component context

### Architecture and dependencies

The WAL writer is a dedicated thread (or a dedicated worker on the per-core scheduler). The data plane publishes records to the WAL via the same bounded MPSC pattern as observability — never blocking the request path on WAL writes by default.

### Durability modes

Configurable per entry via `Durability::{Async, FdataSync, Fsync}`; the binary picks a `default_durability` at config-load time:

- `Async` — entry returns after the per-shard ring-buffer write; the flusher writes and (optionally) `fdatasync`s in the background. Crash may lose entries between the last flush and the crash. Hot-path default for request envelopes.
- `FdataSync` — entry parks until the next group-commit completes a per-shard `fdatasync(2)`. **`v0.2` default** for `default_durability`.
- `Fsync` — same as `Fdatasync` but uses `fsync(2)`. Reserved for audit-grade entries (v0.5 `McpAuditEvent`); not the default for the v0.2 binary.

The flusher cadence is per-shard: every `flush_interval_ms` (default 5 ms) or when the per-shard ring buffer crosses `flush_buffer_bytes` (default 1 MiB). Segment rollover at `segment_size_max_mib` (default 64 MiB).

### Patterns and conventions

- **Append-only.** No in-place updates. New records always go at the tail.
- **Self-describing records.** Each record has a length prefix, a record-type byte, a CRC32 checksum, and a timestamp.
- **Segment-based.** The log is split into segments (e.g. 1 GB each) so old segments can be archived or deleted independently.
- **No record references the WAL itself.** Records are self-contained for replay portability.

### Pitfalls

- **`fsync` does not always do what you think.** Disk caches may lie about durability. We document this and let users opt for `O_DIRECT` if they need a stronger guarantee.
- **Directory `fsync` for atomic file rename** on segment rotation. Easy to forget; well-documented in the ARIES paper and in the Postgres / SQLite WAL implementations.
- **Replay must be idempotent at the record level.** Each record carries enough information that replaying it twice has the same effect as once.
- **PII in logs.** The WAL captures full request/response bodies. Production deployments must encrypt the log, control access, and apply retention policies.

### Standards and review gates

- WAL changes require a crash test: `dd if=/dev/zero` over the file mid-write, verify the parser handles truncation cleanly.
- Performance impact (write amplification, throughput hit) measured against the `NullWal` baseline.
- Replay round-trip tested: write N records, read N records, verify identity.

## Testing strategy

- Crash injection during write.
- Truncated-file recovery (read from a partial file).
- Long-running soak with rotation.
- Replay determinism: same log, two replays, identical event sequences.

## Open questions

- Should the WAL be encrypted at rest by default? Recommend opt-in; encryption is a non-trivial dependency.
- How do we handle very large bodies in the log? Configurable max-body-size; over-large records get a placeholder.
- Cross-instance replay (replay a log from instance A on instance B) is interesting; defer until `v1.x`.
