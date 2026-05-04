# 01.b Non-Functional Requirements

> The qualities Riftgate must have. Performance, scalability, security, reliability, operability, observability, portability.
>
> A note on honesty up front: where these targets are aspirational, they are stated as such. We do not make number claims we cannot defend with a reproducible benchmark in this repo.

## 1. Performance

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-P01 | Median request overhead | <2 ms at 1k QPS on a c7i.xlarge equivalent (`v0.1`) | Excludes upstream backend latency. Reproducible via `benchmarks/`. |
| NFR-P02 | P99 request overhead | <10 ms at 1k QPS (`v0.1`); <5 ms at 5k QPS (`v0.2`) | Honest about being well above TensorZero's claimed <1 ms P99 at 10k QPS — we do not compete on this axis. |
| NFR-P03 | Throughput per core | ≥3k QPS sustained on the `v0.1` epoll path | Soft target; actual depends on filter chain depth. |
| NFR-P04 | Memory per idle connection | <16 KB | Per-request arena returns to a free pool on completion. |
| NFR-P05 | TTFT (time-to-first-token) overhead | <5 ms on streaming requests | Includes parsing, routing, and SSE framing. |
| NFR-P06 | Inter-token latency overhead | <500 µs per token | Excludes upstream tokenization time. |
| NFR-P07 | Rate-limit enforcement overhead | <100 µs per request at 1k RPS on the `v0.2` in-proc impl | Covers the hot-path token-bucket check. Distributed impls are a future extension of the `RateLimiter` trait and carry their own latency budgets; not in scope for `v1.0`. |

**Where we may diverge from this:** when a routing strategy or filter chain inherently costs more (e.g. KV-cache-aware lookups, WASM filter execution), targets above are the *kernel* overhead, not the *configured* overhead. Configured benchmarks live in `benchmarks/` and are honest about the cost of each plugin.

## 2. Scalability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-S01 | Concurrent connections | ≥50k on a single `v0.2` instance | Driven by epoll/io_uring choice; see [Options 001](../05-options/001-io-model.md). |
| NFR-S02 | Concurrent in-flight streaming requests | ≥10k on a single `v0.2` instance | Bounded by per-request arena memory, not by file descriptor count. |
| NFR-S03 | Linear scaling to N cores | ≥80% of single-core throughput per added core, up to 16 cores | Driven by lock-free MPMC queue + work-stealing scheduler. |
| NFR-S04 | Backend pool size | ≥1000 backends in the routing table | Routing strategy lookup is O(log N) or O(1) amortized. |
| NFR-S05 | Filter chain depth | ≥16 filters before measurable overhead | Each WASM filter adds <100 µs to the request path. |

## 3. Reliability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-R01 | Crash recovery | Replay log can rebuild request history from any consistent point | See [`docs/04-design/lld-storage.md`](../04-design/lld-storage.md) for WAL semantics. |
| NFR-R02 | Backend failure isolation | One failing backend does not affect routing to others | Circuit breaker per backend (`v0.2`). |
| NFR-R03 | Graceful degradation under overload | Returns 503 with `Retry-After` rather than OOM-kills | Adaptive backpressure based on local queue depth (`v0.2`); GPU pressure signal (`v0.4`). |
| NFR-R04 | No data loss on graceful shutdown | All in-flight requests complete or return a clean error before exit | SIGTERM handler drains the queue; SIGKILL is acknowledged as data-loss risk. |
| NFR-R05 | No silent corruption of request bodies | Parser is FSM-based and round-trip-tested via property tests | See [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md). |

## 4. Security

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-SEC01 | No RCE via WASM filters | wasmtime runs filters in a sandbox with no host filesystem or network access | Filter capabilities are explicitly granted via config. |
| NFR-SEC02 | Backend credentials never logged | Test harness greps logs for known credential patterns | Fail-closed behavior on credential leaks. |
| NFR-SEC03 | TLS for backend connections | Required by default; `insecure: true` is allowed only with explicit opt-in | Certificate pinning available per backend. |
| NFR-SEC04 | mTLS for client connections | Optional, configurable per listener | For mesh deployments where clients present certificates. |
| NFR-SEC05 | eBPF programs require explicit privilege grant | Aya BPF programs are loaded only when `RIFTGATE_ENABLE_BPF=1` and `CAP_BPF` is held | Riftgate runs without elevated privilege by default. |
| NFR-SEC06 | No request body in default-level logs | Bodies log only at `trace` level with explicit opt-in | PII protection by default. |
| NFR-SEC07 | Dependency audit | `cargo audit` runs in CI; advisories block merges | Cleanly tracked in `Cargo.lock`. |

## 5. Operability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-O01 | Single static binary | `cargo build --release` produces a self-contained binary | Optional dynamic linking only via explicit feature flag. |
| NFR-O02 | Configuration via TOML or env vars | Both supported; env wins in conflicts | See [Options 015](../05-options/015-config-model.md). |
| NFR-O03 | Hot config reload (where safe) | Backend additions/removals do not require restart | Trait-changing config (e.g. swapping IO model) requires restart by design. |
| NFR-O04 | Graceful shutdown ≤30s | SIGTERM drains in-flight requests; force-exits at deadline | Configurable drain timeout. |
| NFR-O05 | Health and readiness endpoints | `/health` (always 200 if process is up); `/ready` (200 only if at least one backend is healthy) | Standard K8s probe pattern. |
| NFR-O06 | Container-friendly | Distroless base image; <50 MB image size | Multi-arch builds (amd64, arm64). |

## 6. Observability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-OBS01 | OpenTelemetry traces for every request | Spans for receive, queue, dispatch, first-token, complete | OTel exporter configurable per common backends (Tempo, Jaeger, vendor). |
| NFR-OBS02 | Prometheus-format metrics | `/metrics` endpoint with request counts, latencies, queue depths, backend health | Standard Prom labels: `method`, `route`, `backend`, `status`. |
| NFR-OBS03 | Structured logs | JSON output with consistent field schema | Configurable level; tracing-subscriber backend. |
| NFR-OBS04 | Token-level SLO metrics | TTFT, inter-token latency, jitter — per backend, per model | New in `v0.4`. |
| NFR-OBS05 | eBPF-derived profiles | Continuous CPU on/off-time profiles emitted to OTel | New in `v0.4`. |
| NFR-OBS06 | Replayable request log | Every (request, response) pair is appended to a WAL | New in `v0.2`; queryable via `riftgate-replay`. |
| NFR-OBS07 | MCP capability audit log | Every MCP `tools/call` invocation emits an audit event with correlation id, tenant, tool, argument hash, and allow/deny outcome | New in `v0.5`; events are written to the WAL and surfaced as structured OTel logs. Required so tenants can reconstruct which tools were reached on their behalf. |

## 7. Portability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-PT01 | Linux x86_64 | Tier 1 — full feature set | Primary target. |
| NFR-PT02 | Linux aarch64 | Tier 1 — full feature set | Apple Silicon dev, ARM cloud. |
| NFR-PT03 | macOS (kqueue) | Tier 2 — epoll path replaced by kqueue; eBPF features disabled | Dev convenience; not production. |
| NFR-PT04 | Windows | Not supported in `v1.0` | IOCP backend is a future possibility but not committed. |
| NFR-PT05 | MSRV (Minimum Supported Rust Version) | Stable, current −2 minor versions | Tracked in `rust-toolchain.toml`. |

## 8. Maintainability

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-M01 | Test coverage | ≥75% line coverage on `riftgate-core` and parser | Tracked in CI; no hard-block on regression. |
| NFR-M02 | Public API stability | `0.x` minor versions can break with `UPGRADING.md` notes; `1.x` follows semver | Enforced by `cargo-semver-checks` from `v1.0`. |
| NFR-M03 | Documentation completeness | Every public item in `riftgate-core` has rustdoc | `cargo doc --document-private-items` builds clean. |
| NFR-M04 | Onboarding time | A new contributor can find their way to a first useful PR in <2 hours of reading | Validated via the [`AGENTS.md`](../../AGENTS.md) loading protocol working as documented. |

## 9. Cost / efficiency

| ID | Quality | Target | Notes |
|----|---------|--------|-------|
| NFR-C01 | Idle resource footprint | <50 MB RSS, <1% CPU on a 4-core host | Important for sidecar deployment patterns. |
| NFR-C02 | No third-party paid services in the data path | All required dependencies are OSS | Riftgate as a binary should run with zero external paid services. |
