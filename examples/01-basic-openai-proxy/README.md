# 01 — basic OpenAI proxy (v0.1)

A self-contained development loop:

```text
   curl -> riftgate (host) -> https://api.openai.com (or any compatible)
              \-> OTel collector (docker) -> stdout
```

This example is the smallest thing that exercises every v0.1 functional
requirement (FR-001 through FR-008). Use it as the "hello world" for
every contributor's first session.

## What this example shows

| FR | Where to look |
|----|---------------|
| FR-001 | `riftgate.toml` ↦ `[server] listen_addr = "127.0.0.1:8080"` |
| FR-002 | Try a malformed JSON body — the gateway responds `400 Bad Request` |
| FR-003 | `riftgate.toml` ↦ `[backend] url = "https://api.openai.com"` |
| FR-004 | Pass `"stream": true` and watch SSE chunks land in `curl -N` |
| FR-005 | Edit `riftgate.toml` and restart; invalid configs exit `78` |
| FR-006 | OTel collector logs print one trace per request (received → completed) |
| FR-007 | A 10k-request loop shows no per-request memory growth in `/proc/$pid/status` |
| FR-008 | Exercised by `cargo bench -p riftgate-core --bench timers` |

## Prerequisites

- `cargo` (the rest is in this folder)
- `docker compose` (only for the OTel collector — optional)
- An OpenAI-compatible API key in `OPENAI_API_KEY`

## Run it

```bash
# Terminal 1: an OTel collector that prints to stdout (optional).
docker compose -f examples/01-basic-openai-proxy/docker-compose.yml up

# Terminal 2: the gateway. Auth header is read from $OPENAI_API_KEY.
RIFTGATE_BACKEND_AUTH_HEADER="Bearer $OPENAI_API_KEY" \
  cargo run --release -p riftgate -- \
    --config examples/01-basic-openai-proxy/riftgate.toml

# Terminal 3: try it.
curl -v http://localhost:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}'

# SSE streaming (watch chunks arrive incrementally):
curl -N http://localhost:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"write a haiku"}]}'

# Liveness / readiness:
curl -v http://localhost:8080/health
curl -v http://localhost:8080/ready
```

## What you should see

In the gateway terminal, one structured JSON log line per startup
phase, then one `span_end` line per checkpoint of every request:

```json
{"kind":"span_end","request_id":"req-1","name":"request.received","duration_ms":0}
{"kind":"span_end","request_id":"req-1","name":"request.queued","duration_ms":0}
{"kind":"span_end","request_id":"req-1","name":"request.dispatched","duration_ms":2}
{"kind":"span_end","request_id":"req-1","name":"request.first_token","duration_ms":340}
{"kind":"span_end","request_id":"req-1","name":"request.completed","duration_ms":1820}
```

In the OTel collector terminal (if running): the same five spans
plus their attributes, formatted as OTLP debug output.

## Without an OTel collector

If you skip the docker compose step the gateway logs a one-time
warning and falls back to JSON-stdout-only sink. Everything else
works identically.

## Stopping cleanly

`Ctrl-C` (or `kill -TERM`) starts the drain:

1. `/ready` flips to `503 DRAINING`. Load balancers stop sending new traffic.
2. The accept loop stops accepting.
3. In-flight requests get up to `--drain-grace-ms` (default 30 s) to
   finish.
4. The OTel SDK flushes pending spans and the process exits.
