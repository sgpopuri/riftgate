# 03.d Control Plane

> Configuration, CRDs, hot reload, backend health management. Lightest of the four planes by design.
>
> Status: **outline-stage**. Production maturity in `v1.0`.

## What lives here

- Configuration parsing and validation
- Hot reload for safe-to-change config
- The Kubernetes operator and CRDs (`v1.0`)
- Backend registration and health-check loops
- Admin API (read-only inspection of routing tables, circuit-breaker states, etc.)

## What does NOT live here

- Per-request behavior. That is the [data plane](data-plane.md).
- Filter or routing-strategy choice. That is the [extension plane](extension-plane.md).
- Metrics or trace emission. That is the [observability plane](observability-plane.md).

The control plane only *configures* what the data, extension, and observability planes do. It must not become a place where logic accretes.

## Configuration model

`v0.1`: static TOML at startup, environment variables override, validation happens at boot. Invalid configs fail loudly — Riftgate exits with a structured error and a non-zero exit code.

`v0.2-v0.3`: hot reload for the safe subset:

- Backend pool changes (add/remove/weight)
- Route table changes (which backends serve which model names)
- Filter chain composition (load order)

Trait-changing config (e.g. swap the `AsyncIO` impl) requires restart **by design**. Hot-swapping IO models would invite race conditions and saving the cost of a graceful restart is not worth the complexity.

`v1.0`: CRD-driven config via the Kubernetes operator. The operator reads CRDs (`Riftgate`, `RiftgateBackend`, `RiftgateRoute`) and pushes config changes to the data-plane pods.

## CRD sketch (`v1.0`)

```yaml
apiVersion: riftgate.io/v1
kind: Riftgate
metadata:
  name: gateway-prod
spec:
  ioModel: epoll                # or io_uring (requires Linux 5.10+)
  scheduler: per-core
  observability:
    otelEndpoint: "http://otel-collector:4317"
    enableBpf: false
---
apiVersion: riftgate.io/v1
kind: RiftgateBackend
metadata:
  name: vllm-cluster-a
spec:
  url: "http://vllm-a:8000/v1"
  weight: 100
  circuitBreaker:
    failureThreshold: 5
    halfOpenAfter: 30s
---
apiVersion: riftgate.io/v1
kind: RiftgateRoute
metadata:
  name: gpt-4-route
spec:
  match:
    model: "gpt-4*"
  router: kv-aware
  backends:
    - vllm-cluster-a
    - vllm-cluster-b
  filters:
    - pii-redactor
    - cost-guard
```

These CRDs are sketches — the actual schemas will iterate during `v1.0` design.

## Backend health management

- Each backend has a configurable health check (HTTP `/health` or a TCP connection).
- Health-check loop runs once per `health_interval` (default 5s).
- Health state feeds the routing decision: unhealthy backends are excluded from selection.
- Circuit breaker state (per backend) is also part of health — see [Options 011](../05-options/011-circuit-breaker.md).

## Admin API (read-only)

`/admin/routes` — current route table.
`/admin/backends` — current backend pool with health and circuit state.
`/admin/filters` — current filter chain composition.
`/admin/config` — effective config (sensitive values redacted).

All admin endpoints are read-only. Mutating admin actions are deliberately not in scope; mutation goes through config change → restart or hot reload, not API calls.

## Open design questions

- Should the operator support multi-cluster routing? Recommend deferring beyond `v1.0`.
- Should config validation be a separate `riftgate config check <path>` command? Recommend yes, with the same validator as the runtime.
- Should we support config diffs ("dry run") to preview the effect of a change? Recommend yes for `v1.0`.
