# 03.c Observability Plane

> Traces, metrics, profiles, and the eBPF integration. Observability is a contract between the data plane (which emits events) and the sinks (which consume them) — never the other way around.
>
> Status: **outline-stage**. Filled out as `v0.4` (eBPF milestone) approaches.

## What lives here

- The OTel exporter (`ObservabilitySink` impl)
- The Prometheus metrics endpoint
- The Aya-based eBPF programs (`v0.4`)
- The token-level metrics aggregator

## The observability contract

The data plane publishes typed events to a **bounded MPSC channel**. Observability sinks consume from the channel. **The data plane never blocks on observability.** If the channel fills:

- The event is dropped.
- A `riftgate_observability_dropped_total` counter is incremented.
- The drop is logged at `warn` level no more than once per minute.

This is the backpressure-as-policy pattern: explicit drop-on-full, count it, do not pretend it did not happen.

## Trace span structure

```
http.request                 (root span, per request)
├── parse                    (parsing phase)
├── filter.request           (request-side filter chain)
│   ├── filter.{name}        (one per filter, optional)
├── route                    (routing decision, names backend chosen)
├── upstream.request         (backend dispatch)
│   ├── tls.handshake        (if first request to backend)
├── upstream.first_token     (TTFT marker)
├── upstream.streaming       (token-by-token, sampled)
├── filter.response          (response-side filter chain)
└── wal.append               (request log write)
```

Spans use OTel semantic conventions where they exist; Riftgate-specific attributes are namespaced `riftgate.*`.

## Prometheus metrics (initial set)

| Metric | Type | Labels | Notes |
|--------|------|--------|-------|
| `riftgate_requests_total` | counter | `method`, `route`, `status` | Standard request count. |
| `riftgate_request_duration_seconds` | histogram | `route`, `status` | End-to-end overhead. |
| `riftgate_upstream_duration_seconds` | histogram | `backend`, `status` | Backend-side latency. |
| `riftgate_ttft_seconds` | histogram | `backend`, `model` | Token-time-to-first-token. New in `v0.4`. |
| `riftgate_inter_token_seconds` | histogram | `backend`, `model` | Inter-token latency. New in `v0.4`. |
| `riftgate_queue_depth` | gauge | `worker` | Per-worker queue depth for backpressure observability. |
| `riftgate_backend_circuit_state` | gauge (0/1/2) | `backend` | 0=closed, 1=half-open, 2=open. |
| `riftgate_observability_dropped_total` | counter | `kind` | Events dropped due to channel saturation. |

## eBPF integration (`v0.4`)

The eBPF programs do **three** things:

1. **Continuous gateway profiling.** CPU on/off-time per worker, syscall counts, NUMA misses, page faults. Surfaced as flame-graph-friendly profiles via OTel.
2. **Backend GPU pressure correlation.** Reads DCGM/NVML signals for known backends, correlates with request dispatches. The signal is exposed to routers via the read-only signal channel.
3. **TCP-level observability.** Retransmits per upstream, RTT histograms, accept queue depth. Useful for diagnosing "is it the network?"

eBPF programs require `CAP_BPF`. They are loaded only when `RIFTGATE_ENABLE_BPF=1` is set explicitly. The default deployment runs without elevated privilege.

The Aya choice (Rust BPF library) and the alternatives are explored in [Options 014](../05-options/014-ebpf-integration.md).

## Token-level SLOs

Standard request-latency metrics undercount what users feel for streaming responses. Riftgate emits:

- **TTFT** — wall-clock time from request received to first token emitted to client.
- **Inter-token latency** — time between consecutive tokens, per backend, per model.
- **Token jitter** — p99 vs median inter-token latency. Surfaces "stuttery streams" that don't show up in p99 of request duration.

These metrics land in `v0.4` because they require coordinated emission from the parser (knows when a token ends) and the IO subsystem (knows when the byte left the kernel).

## Open design questions

- Should we emit one trace span per token, or only sample? Recommend sample (e.g. 1 in 100) with full per-token in WAL for replay.
- Should the BPF profiles be attached as span attributes or emitted separately? Recommend separately (continuous profile stream); span attributes get heavy fast.
- How do we expose the GPU pressure signal to routers? Recommend a read-only `BackendSignal` struct injected into `Router::route`.
