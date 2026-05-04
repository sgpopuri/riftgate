# 04.d LLD — Storage (Request Log / WAL)

> Append-only request log capturing every (request, response) pair for replay, eval generation, and post-mortem debugging.
>
> Status: **outline-stage**. Filled out as `v0.2` (basic WAL) and `v1.0` (replay framework) land.

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
| `FileWal` | `v0.2` | `riftgate-replay` | Append-only file with length+CRC framing. Fastest, simplest. |
| `RocksWal` | future | TBD | Embedded RocksDB. Adds compaction, range queries, but more dependencies. |
| `NullWal` | `v0.1` | `riftgate-core` | No-op for benchmarks and dev. |

Decision rationale: [Options 009 (request log)](../05-options/009-request-log.md).

Source-systems chapters: `Ch9 (LSM trees and storage engines)`, `Ch11 (WAL and recovery)`.

## Component context

### Architecture and dependencies

The WAL writer is a dedicated thread (or a dedicated worker on the per-core scheduler). The data plane publishes records to the WAL via the same bounded MPSC pattern as observability — never blocking the request path on WAL writes by default.

### Durability modes

Configurable per Riftgate instance:

- `none` — WAL is disabled. Fastest. No replay capability.
- `async` — WAL writes are best-effort; data plane does not wait. Crash may lose the last few seconds of records. **Default.**
- `batched_fsync` — WAL writes accumulate in memory; fsync on a configurable interval (e.g. 100ms). Lower data loss.
- `sync` — Every WAL append blocks the response until fsync completes. Strongest durability, lowest throughput.

Mode is per-instance, not per-request. A single config knob.

### Patterns and conventions

- **Append-only.** No in-place updates. New records always go at the tail.
- **Self-describing records.** Each record has a length prefix, a record-type byte, a CRC32 checksum, and a timestamp.
- **Segment-based.** The log is split into segments (e.g. 1 GB each) so old segments can be archived or deleted independently.
- **No record references the WAL itself.** Records are self-contained for replay portability.

### Pitfalls

- **`fsync` does not always do what you think.** Disk caches may lie about durability. We document this and let users opt for `O_DIRECT` if they need a stronger guarantee.
- **Directory `fsync` for atomic file rename** on segment rotation. Easy to forget; covered in `Ch11 (WAL and recovery)`.
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
