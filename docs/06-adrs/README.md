# Architecture Decision Records (ADRs)

Each ADR captures a decision: context, decision, consequences. Format is Michael Nygard's. Decisions are numbered and immutable — supersede with a new ADR rather than editing in place.

## Conventions

- **Numbered sequentially.** `0001`, `0002`, … Numbering is permanent.
- **Status reflects current.** `proposed` → `accepted`. A superseded ADR keeps `superseded by ADR-NNNN` forever.
- **Decision is a sentence.** If you need a paragraph, you are writing an Options doc, not an ADR.
- **Compliance is explicit.** Say how the decision is enforced — CI, lint, review.

## How to add a new ADR

1. Identify the corresponding Options doc in [`../05-options/`](../05-options/). If none exists, write one first.
2. Copy [`_template.md`](_template.md) to `NNNN-<slug>.md` with the next free number.
3. Fill in. Be brief and decisive.
4. Update both this index and the Options doc's frontmatter to link the ADR.
5. Open a PR. ADR PRs require explicit reviewer signoff (no auto-merge) because the cost of a bad decision compounds.

## Index

| # | Title | Status | Date | Options doc |
|---|-------|--------|------|-------------|
| 0001 | [Rust, not Go or Zig, for the kernel](0001-rust-not-go-or-zig.md) | accepted | 2026-05-02 | n/a (foundational) |
| 0002 | [Start on epoll, add io_uring as feature flag](0002-start-on-epoll.md) | accepted | 2026-05-02 | [001-io-model](../05-options/001-io-model.md) |
| 0003 | [Tokio multi-threaded runtime as the only v0.1 runtime; per-core runtimes revisited at v0.2 retro](0003-tokio-multithread-default.md) | accepted | 2026-05-03 | [002-async-runtime](../05-options/002-async-runtime.md) |
| 0004 | [Shared-nothing per-shard scheduler in v0.1; work-stealing as v0.2 opt-in](0004-per-shard-default-stealing-opt-in.md) | accepted | 2026-05-03 | [003-concurrency-model](../05-options/003-concurrency-model.md) |
| 0005 | [Sharded MPMC queue strategy; crossbeam-channel in v0.1, hand-rolled Vyukov in v0.2](0005-sharded-mpmc-queue.md) | accepted | 2026-05-03 | [004-request-queue](../05-options/004-request-queue.md) |
| 0006 | [Per-request bump arena on the hot path; system malloc globally in v0.1; mimalloc opt-in in v0.2](0006-bump-arena-plus-system-malloc.md) | accepted | 2026-05-03 | [005-allocator](../05-options/005-allocator.md) |
| 0007 | [Hand-rolled table-driven FSM in riftgate-parser; httparse for headers in v0.1; full FSM in v0.2](0007-handrolled-fsm-parser.md) | accepted | 2026-05-03 | [007-protocol-parser](../05-options/007-protocol-parser.md) |
| 0008 | [SSE as the only v0.1 streaming framing; NDJSON optional in v0.2+; gRPC bidi deferred to v1.0+](0008-sse-default-grpc-future.md) | accepted | 2026-05-03 | [008-stream-framing](../05-options/008-stream-framing.md) |
| 0009 | Rate limiter trait + in-proc token-bucket only in `v1.0` | reserved (proposed) | TBD | [021-rate-limiting](../05-options/021-rate-limiting.md) |
| 0015 | MCP as a first-class citizen of the extension plane (gateway-as-broker) | reserved (proposed) | TBD | [026-mcp-orchestration](../05-options/026-mcp-orchestration.md) |

Number `0010`–`0014` and `0016`–`0020` are reserved in order for the remaining Options docs and will be written as each decision lands. Reservations for `0009` and `0015` are called out above because their Options docs are authored (see the index) even though their implementation ships later.

## Status legend

- **proposed** — under discussion. The decision is not yet binding.
- **accepted** — current. Code and docs follow this.
- **superseded by ADR-NNNN** — historical. The new ADR is current.
- **deprecated** — was accepted, no longer applies, and was not superseded by another decision (e.g. the area of concern no longer exists).
