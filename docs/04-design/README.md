# Low-level design

One LLD per subsystem. Each LLD is the operating theory of that subsystem: trait surface, planned implementations, architecture and dependencies, patterns and conventions, pitfalls, testing strategy, open questions.

| Subsystem | LLD | Implementation crate |
|-----------|-----|----------------------|
| IO runtime | [`lld-io-runtime.md`](lld-io-runtime.md) | `crates/riftgate-io-epoll` (later: `crates/riftgate-io-uring`) |
| Scheduling | [`lld-scheduling.md`](lld-scheduling.md) | `crates/riftgate-core` |
| Parsing | [`lld-parsing.md`](lld-parsing.md) | `crates/riftgate-parser` |
| Storage / WAL | [`lld-storage.md`](lld-storage.md) | `crates/riftgate-replay` (v0.2+) |
| Allocator | [`lld-allocator.md`](lld-allocator.md) | `crates/riftgate-core` |
| Timers | [`lld-timers.md`](lld-timers.md) | `crates/riftgate-core` |
| Routing | [`lld-routing.md`](lld-routing.md) | `crates/riftgate-router` |
| Filter chain | [`lld-filter-chain.md`](lld-filter-chain.md) | `crates/riftgate-filter` (v0.3) |
| Observability | [`lld-observability.md`](lld-observability.md) | `crates/riftgate-obs` |
| Rate limiting | [`lld-rate-limiter.md`](lld-rate-limiter.md) | `crates/riftgate-core` (v0.2 default impl) |
| MCP capability | [`lld-mcp-capability.md`](lld-mcp-capability.md) | `crates/riftgate-mcp` (v0.5) |

See [`AGENTS.md`](../../AGENTS.md) §2 for the load order: read the LLD for the subsystem you intend to touch *before* the implementing crate.
