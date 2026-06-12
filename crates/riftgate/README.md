# riftgate

The Riftgate binary. The v0.1 walking-skeleton entry point that proxies
OpenAI-format `/v1/chat/completions` traffic — including streaming
Server-Sent-Events responses — to one configured upstream backend.

This crate is the integration point. The interesting code is in:

- [`riftgate-core`](../riftgate-core) — trait surfaces and shared types.
- [`riftgate-config`](../riftgate-config) — TOML + env config loader.
- [`riftgate-router`](../riftgate-router) — `RoundRobinRouter` (v0.1 default).
- [`riftgate-obs`](../riftgate-obs) — bounded MPSC bus + sinks.
- [`riftgate-parser`](../riftgate-parser) — `Http1Parser` + `SseFramer`.
- [`riftgate-io-epoll`](../riftgate-io-epoll) — `MioIO` `AsyncIO` impl.

## Architecture (v0.1)

```text
                   tokio multi-thread runtime (ADR 0003)
   ┌────────────────────────────────────────────────────────────┐
   │                                                            │
   │   tokio::net::TcpListener                                  │
   │           │                                                │
   │           v                                                │
   │   accept_loop ── shutdown_signal? ── exit                  │
   │           │                                                │
   │           v   spawn-per-connection                         │
   │   hyper::server::conn::http1                               │
   │           │                                                │
   │           v   service_fn(handle)                           │
   │   ┌───────────────────────────────────────────────┐        │
   │   │  proxy::handle(req)                           │        │
   │   │   1.  match path:                             │        │
   │   │       /health  -> 200 OK                      │        │
   │   │       /ready   -> 200 / 503 (drain state)     │        │
   │   │       *        -> proxy_to_backend            │        │
   │   │   2.  router.route(...) -> BackendId          │        │
   │   │   3.  upstream_client.request(...)            │        │
   │   │   4.  stream body back; emit spans            │        │
   │   └───────────────────────────────────────────────┘        │
   │                                                            │
   └────────────────────────────────────────────────────────────┘

                Observability (riftgate-obs)
   ┌────────────────────────────────────────────────────────────┐
   │  Bus (bounded MPSC, drop-on-full)                          │
        │      ┌─> BpfSink (Linux + `bpf` feature + env gate)        │
   │      ┌─> OtelSink (OTLP/gRPC)                              │
   │      └─> JsonStdoutSink                                    │
   └────────────────────────────────────────────────────────────┘
```

## Why `tokio` + `hyper`, when we wrote our own `AsyncIO` and parser?

The trait surfaces (`AsyncIO`, `StreamParser`) are the load-bearing
contracts; `riftgate-io-epoll` and `riftgate-parser` ship as the v0.1
implementations. The v0.1 *binary* picks tokio + hyper as the
out-of-the-box runtime because:

- Tokio's multi-thread runtime is the v0.1 commitment per
  [ADR 0003](../../docs/06-adrs/0003-tokio-multithread-default.md).
- Hyper is the most battle-tested HTTP/1.1 stack in the Rust ecosystem
  and gives us `keep-alive`, chunked encoding (which our v0.1 parser
  does not yet implement; ADR 0007 §future-work), and TLS termination
  for free.
- Pluggability is preserved: the trait surfaces and the alternative
  impls exist; a future binary that wires `MioIO` + `Http1Parser` end
  to end is a build-flag away.

The `SseFramer` in `riftgate-parser` IS exercised by the binary — on
the upstream response body — to count SSE tokens for the
`request.first_token` span emission.

## CLI

```text
riftgate --config etc/riftgate.toml
riftgate --config etc/riftgate.toml --dev   # pretty logs, lax TLS
riftgate --check  --config etc/riftgate.toml   # validate config + exit
```

Per [ADR 0012](../../docs/06-adrs/0012-static-toml-env-override-v01.md)
the binary exits non-zero with a list of errors if the config is
invalid.

## FR coverage

| FR | Where |
|----|-------|
| FR-001 | `src/server.rs::accept_loop` (`TcpListener::bind` + `accept`) |
| FR-002 | `src/proxy.rs::handle` (`/v1/chat/completions` JSON parse) |
| FR-003 | `src/proxy.rs::handle` + `src/upstream.rs` (hyper-rustls) |
| FR-004 | `src/proxy.rs::handle` (response body forwarded chunk by chunk) |
| FR-005 | `riftgate-config` (loaded by `bootstrap`) |
| FR-006 | `src/proxy.rs` emits `request.received` ... `request.completed` spans |
| FR-007 | `src/proxy.rs::handle` constructs a per-request `BumpArena` and drops it on completion |
| FR-008 | `riftgate-core::timers::BinaryHeapTimers` (benched in `phase J`) |

## Running locally

```bash
# 1. Run an OTel collector (optional; otherwise spans go nowhere):
docker compose -f examples/01-basic-openai-proxy/docker-compose.yml up -d otel-collector

# 2. Run riftgate against api.openai.com:
RIFTGATE_BACKEND_URL=https://api.openai.com \
RIFTGATE_BACKEND_AUTH_HEADER="Bearer $OPENAI_API_KEY" \
cargo run --bin riftgate

# 3. Hit it:
curl -v http://localhost:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}'
```

For a fully self-contained dev loop see
[`examples/01-basic-openai-proxy`](../../examples/01-basic-openai-proxy/README.md).

## Optional features

- `--features bpf` on this crate forwards to `riftgate-obs/bpf` and compiles
  the Linux-only Aya `BpfSink` path.
- `--features gpu-nvml` on this crate forwards to `riftgate-obs/gpu-nvml` and
  enables the Linux-only NVML FFI GPU-pressure source.

The BPF runtime remains opt-in even when compiled in: set
`RIFTGATE_ENABLE_BPF=1` to enable the sink at startup; otherwise it stays in
`DisabledByEnv` state.
