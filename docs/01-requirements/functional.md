# 01.a Functional Requirements

> What Riftgate must DO. Read [`00-vision.md`](../00-vision.md) first for the *why*.

This document is structured by phase. Each requirement is identified as `FR-NNN`, has a target milestone (`v0.1`, `v0.2`, ...), and is acceptance-testable.

## v0.1 — Walking skeleton

The minimum useful gateway: a single Rust binary that proxies OpenAI-format traffic to one backend with streaming.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-001 | Accept HTTP/1.1 requests on a configurable TCP port | v0.1 | `curl -v http://localhost:8080/v1/chat/completions` returns a valid response |
| FR-002 | Parse OpenAI-format `/v1/chat/completions` request bodies | v0.1 | Rejects malformed JSON; accepts spec-conformant requests |
| FR-003 | Forward requests to one configured upstream OpenAI-compatible backend | v0.1 | Backend receives the request; client sees the response |
| FR-004 | Support `stream: true` with Server-Sent Events response framing | v0.1 | Tokens stream to the client incrementally; SSE `data:` framing matches the OpenAI spec |
| FR-005 | Configurable upstream URL, auth header, and timeout via TOML config | v0.1 | Config changes take effect on restart; invalid configs fail loudly at startup |
| FR-006 | Emit OpenTelemetry traces for each request (received, queued, dispatched, first-token, completed) | v0.1 | Traces visible in a local OTel collector |
| FR-007 | Per-request arena allocator releases all per-request memory at completion | v0.1 | Memory profile after 10k requests shows no per-request growth |
| FR-008 | Hierarchical timer wheel handles per-request deadlines without per-tick scan | v0.1 | 100k concurrent timers cost less than O(n) per tick |

## v0.2 — The systems showpiece

Honest performance, multi-backend routing, durable request log, circuit breakers, work-stealing.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-101 | `io_uring` IO backend behind a feature flag, with `epoll` as the default | v0.2 | `cargo build --features io-uring` produces a binary that runs on Linux 5.10+ |
| FR-102 | Multiple upstream backends with round-robin and weighted-random routing | v0.2 | Config supports a list of backends; traffic distributes per the configured policy |
| FR-103 | Circuit breaker per upstream (closed/open/half-open) | v0.2 | A backend that errors above threshold is removed from the pool until half-open succeeds |
| FR-104 | Adaptive backpressure: return 503 when local queue depth exceeds a configured high-water mark | v0.2 | Load tests show graceful 503s rather than OOM under overload |
| FR-105 | Append-only WAL-style request log capturing (request, response) pairs | v0.2 | After a crash, the log can be replayed to reconstruct request history |
| FR-106 | Lock-free MPMC request queue between accept and worker threads | v0.2 | Microbenchmark shows linear scaling to 8 cores |
| FR-107 | Work-stealing scheduler option | v0.2 | Heterogeneous request mix benchmarks show better tail latency than the default per-core scheduler |
| FR-108 | In-proc token-bucket rate limiter implementing the `RateLimiter` trait, per-route and per-backend | v0.2 | A configured rate is enforced; excess requests receive `429 Too Many Requests` with a `Retry-After` header. No cross-replica coherence (see [NFR-P07](non-functional.md)). |

## v0.3 — Programmability

WASM filter chain, plugin-based routing strategies, hedged requests.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-201 | WASM filter chain on request and response paths | v0.3 | A user can write a Rust filter, compile to WASM, and load it via config without rebuilding Riftgate |
| FR-202 | Starter filter library: PII redaction, prompt template substitution, output schema validation, cost guard | v0.3 | Each filter has end-to-end tests that fail when the filter is removed |
| FR-203 | Routing strategies as plugins implementing the `Router` trait | v0.3 | A custom routing strategy compiles, registers via config, and is exercised by integration tests |
| FR-204 | KV-cache-aware routing strategy as a built-in plugin (integrates with `vllm-router` LMCache controller OR uses an internal prefix trie) | v0.3 | Identical-prefix requests are routed to the same backend more often than random |
| FR-205 | Hedged requests: send to two backends, accept the first, cancel the slower mid-stream | v0.3 | Tail latency improves on a slow-backend mix; cancellation is visible in upstream logs |
| FR-206 | (Optional, post-`v0.2`-retro) Priority / tier scheduling in the request queue (premium / standard / batch / system) | v0.3 | Under load, premium-tier tail latency is unaffected by batch-tier backlog. Gated by the `v0.2` retro deciding whether Options `022` is worth pursuing. |

## v0.4 — eBPF and the depths

Gateway-internal continuous profiling and backend GPU pressure observability.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-301 | Aya-based eBPF programs profile the gateway process continuously | v0.4 | Profiles surface in OTel format; CPU on/off, syscall stalls, NUMA misses are visible |
| FR-302 | Backend GPU pressure signal via DCGM/NVML correlation | v0.4 | When a backend GPU saturates, the signal reaches the routing strategy and influences traffic distribution |
| FR-303 | Token-level SLO metrics emitted: TTFT, inter-token latency, p99/p99.9 token jitter | v0.4 | Dashboard query returns these metrics per backend, per model, per route |

## v0.5 — Agentic capability plane

First-class [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) support inside the extension plane. The gateway becomes a capability broker: it understands MCP requests, enforces per-tenant tool/resource allowlists, and audits every invocation. Not a new plane — a first-class citizen of the extension plane.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-501 | Parse and proxy MCP requests (`tools/list`, `tools/call`, `resources/*`, `prompts/*`) alongside HTTP chat-completions | v0.5 | An MCP client connects through Riftgate to an MCP server and performs a full tool-call round trip; negative tests verify malformed MCP requests are rejected cleanly. |
| FR-502 | Tenant-scoped capability allowlist: the `CapabilityBroker` trait enforces which tools / resources a request identity is permitted to reach | v0.5 | A tenant with `tool: search-web` allowed and `tool: filesystem-write` denied sees the former succeed and the latter rejected with `403` and a `riftgate.mcp.reason` header. |
| FR-503 | MCP invocation audit log — every `tools/call` decision (allow / deny) written to the WAL with tenant, tool, argument hash, and outcome | v0.5 | After 1k invocations, the WAL contains 1k audit entries round-trippable via `riftgate-replay`. |
| FR-504 | Attestation headers surfacing who-called-what for downstream policy engines (`riftgate-mcp-caller`, `riftgate-mcp-tool`, `riftgate-mcp-decision`) | v0.5 | A downstream MCP server receives the attestation headers and can cross-check them against its own audit pipeline. |

## v1.0 — Production-ready and mesh-native

K8s operator, CRDs, sidecar deployment, comprehensive tests.

| ID | Requirement | Target | Acceptance |
|----|-------------|--------|------------|
| FR-401 | Kubernetes operator with CRDs for Riftgate config (`Riftgate`, `RiftgateBackend`, `RiftgateRoute`) | v1.0 | Helm chart installs; CRD changes are picked up by the operator and pushed to data-plane pods |
| FR-402 | Sidecar deployment manifest verified against Istio and Linkerd | v1.0 | Test deployments in both meshes pass smoke tests |
| FR-403 | Replay framework: re-run any logged request against a different backend or filter set | v1.0 | `riftgate-replay` CLI runs a captured log against a config and produces a diff |
| FR-404 | Property-based tests on the parser, fuzz tests on the wire format | v1.0 | `cargo fuzz` runs cleanly for >1 hour; property tests have ≥80% case coverage |
| FR-405 | Documented upgrade path from each prior `vN.M` release | v1.0 | Each minor release ships with `UPGRADING.md` listing breaking changes |

## Cross-cutting requirements

Apply to every milestone.

| ID | Requirement | Acceptance |
|----|-------------|------------|
| FR-X01 | Every load-bearing change traces back to an Options doc and an ADR | PRs without this linkage are closed |
| FR-X02 | Every public trait in `riftgate-core` has at least two implementations (or a documented reason for one) | Compile-time verification via `#[cfg(test)]` impls |
| FR-X03 | Public-facing API uses semver | `cargo semver-checks` runs in CI for `0.x.y` releases |
| FR-X04 | Documentation builds with `mdbook` from `docs/` | CI fails if `mdbook build` errors |
| FR-X05 | All examples in `examples/` build and run as part of CI | A broken example fails the build |
