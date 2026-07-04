# Riftgate Operator Handbook

> Hands-on guide for deploying, configuring, extending, and operating Riftgate. For architecture theory see [`docs/03-architecture/`](../03-architecture/); for decision rationale see [`docs/05-options/`](../05-options/) and [`docs/06-adrs/`](../06-adrs/).

---

## 1. What Riftgate does (capabilities overview)

Riftgate is a programmable Rust gateway for AI inference traffic. It sits between clients and model backends (vLLM, TGI, llama.cpp, OpenAI API, Anthropic via adapter) and provides:

| Capability | Description | Since |
|---|---|---|
| HTTP/1.1 + SSE proxy | OpenAI-compatible `/v1/chat/completions`; full SSE streaming | v0.1 |
| Multi-backend routing | Round-robin, weighted-random, circuit-breaker | v0.2 |
| KV-cache-aware routing | Prefix-trie affinity; routes same-prefix prompts to the same backend | v0.3 |
| Hedged requests | Races two backends; cancels the slower mid-stream | v0.3 |
| WASM filter chain | Request/response transforms in sandboxed WASM; full plugin API | v0.3 |
| Token-level SLO metrics | TTFT, inter-token latency, p99/p99.9 jitter per (tenant, model, route) | v0.4 |
| GPU pressure routing | DCGM/NVML telemetry → route away from saturated backends | v0.4 |
| eBPF gateway profiling | CPU on/off, syscall stalls, TCP retransmits — in-process, not sidecar | v0.4 |
| MCP capability brokering | Authorizes tool/resource calls; per-tenant allowlists; WAL audit | v0.5 |
| API key authentication | SHA-256 hashed key registry; per-tenant identity resolution | v1.0 |
| Kubernetes operator | `Riftgate`/`RiftgateBackend`/`RiftgateRoute` CRDs; Helm chart | v1.0 |

**What Riftgate is NOT**: a universal multi-provider translator (use LiteLLM for that), a vLLM-native KV router (see `vllm-router`), or a globally-coherent rate limiter (in-proc only; v1.0).

---

## 2. Quick start

```bash
git clone https://github.com/sgpopuri/riftgate
cd riftgate
cargo build --release -p riftgate

# Minimal config
cat > riftgate.toml <<'EOF'
[server]
listen_addr = "127.0.0.1:8080"

[backend]
url         = "https://api.openai.com"
auth_header = "Bearer sk-..."
timeout_ms  = 30000
EOF

./target/release/riftgate --config riftgate.toml

# Smoke test
curl -s http://127.0.0.1:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"hi"}]}'
```

---

## 3. Configuration reference

### 3.1 File layout

```toml
# riftgate.toml

[server]
listen_addr    = "0.0.0.0:8080"   # default: 127.0.0.1:8080
worker_threads = 4                 # default: auto-detect (logical cores)

[backend]
url         = "https://api.openai.com"  # required
auth_header = "Bearer sk-..."           # injected as Authorization header
timeout_ms  = 30000                     # default: 30000
tls_verify  = true                      # default: true

[obs]
otel_endpoint = "http://otel-collector:4317"  # default: http://127.0.0.1:4317
sample_rate   = 0.01                          # fraction of requests to sample
bus_capacity  = 4096                          # MPSC bus drop-on-full capacity

[log]
level  = "info"                    # error|warn|info|debug|trace
format = "json"                    # json|pretty

[timer]
tick_resolution_ms = 10            # default: 10ms

[multitenancy]
mode = "api-key"                   # api-key (default) or trusted-header

[multitenancy.api_keys]
"sha256:<64-hex>" = "<tenant-name>"

[mcp]
enforce         = true             # false = dry-run
signing_key_hex = "<64-hex>"       # HMAC key; random ephemeral if absent
wal_path        = "/var/riftgate/mcp-audit"  # omit = NoopWal

[mcp.tenants."<name-or-numeric-id>"]
allowed_tools             = ["search-web"]
denied_tools              = ["filesystem-write"]
allowed_resource_prefixes = ["s3://acme/*"]
time_bounded_grants       = [{ tool = "send-email", until_unix_secs = 1780000000 }]

[[filter]]
path         = "/etc/riftgate/filters/my.wasm"
capabilities = ["read-body", "write-body", "read-headers", "log"]
[filter.config]
key = "value"
```

### 3.2 Environment variable overrides

Every `[section]` field maps to `RIFTGATE__<SECTION>_<FIELD>` (double-underscore, uppercase). Env wins over file. Examples:

```bash
RIFTGATE__BACKEND_URL=http://vllm:8000
RIFTGATE__LOG_LEVEL=debug
RIFTGATE__SERVER_LISTEN_ADDR=0.0.0.0:9090
```

### 3.3 Config validation

```bash
./riftgate --config riftgate.toml --check
# exits 0 on valid config, 78 on config error
```

---

## 4. Ecosystem and dependencies

### 4.1 Required (always needed)

| Dependency | Role | Notes |
|---|---|---|
| Linux 5.15+ or macOS 12+ | Runtime OS | epoll on Linux, kqueue on macOS |
| TLS-capable upstream | Riftgate uses rustls + webpki-roots | No OpenSSL |

### 4.2 Optional (feature-gated)

| Dependency | Feature flag | Role |
|---|---|---|
| Linux 5.10+ kernel | `io-uring` feature | io_uring IO backend |
| `RIFTGATE_ENABLE_BPF=1` + `CAP_BPF` | `bpf` feature | Aya eBPF programs |
| DCGM exporter at `RIFTGATE_GPU_DCGM_ENDPOINT` | none needed | GPU pressure routing |
| NVML / NVIDIA driver | `gpu-nvml` feature | Alternative GPU source |

### 4.3 Observability stack (recommended)

```
OTel collector  ←  OTLP/gRPC on port 4317  ←  Riftgate OtelSink
Prometheus      ←  scrape /metrics (todo: v1.1+)
Grafana         ←  Prometheus + OTel data source
```

The `examples/01-basic-openai-proxy/` directory has a Docker Compose stack that starts the OTel collector and a mock backend:

```bash
cd examples/01-basic-openai-proxy
docker compose up -d
./../../target/release/riftgate --config riftgate.toml
```

### 4.4 Kubernetes operator dependencies

```bash
# Install CRDs + operator
helm install riftgate-operator deploy/helm/riftgate-operator/

# Operator RBAC requirements:
# - read riftgates, riftgatebackends, riftgateroutes
# - get/create/patch/update configmaps, deployments
# - get secrets (for apiKeySecretRef)
# - get/list/watch endpoints (for serviceRef dynamic discovery)
# - get/create/patch/update leases (leader election)
```

---

## 5. Deployment topologies

### 5.1 Standalone binary (simplest)

```
Clients → [Riftgate binary] → [vLLM / TGI / OpenAI]
```

- Single process; config file; no external dependencies
- Scale out: multiple replicas behind an L4 LB
- Use when: development, bare-metal, small-scale

### 5.2 Kubernetes operator (recommended for production)

```
kubectl apply (RiftgateRoute CRDs)
         ↓
riftgate-operator reconciler
         ↓
ConfigMap (rendered riftgate.toml) + Deployment (riftgate binary)
```

```yaml
# Apply a gateway + backend + route
apiVersion: riftgate.io/v1alpha1
kind: Riftgate
metadata: { name: prod-gw, namespace: default }
spec:
  image: ghcr.io/sgpopuri/riftgate:v1.0.0
  replicas: 3
  obsEndpoint: "http://otel-collector:4317"
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateBackend
metadata: { name: vllm-prod, namespace: default }
spec:
  serviceRef: { name: vllm-service, port: 8000 }
  timeoutMs: 30000
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateRoute
metadata: { name: tenant-acme, namespace: default }
spec:
  pathPrefix: "/v1/"
  backendRef: vllm-prod
  mcp:
    enforce: true
    allowedTools: ["search-web"]
  multitenancy:
    apiKeySecretRef: { name: acme-keys, key: api-keys }
```

### 5.3 Sidecar / ambassador (optional)

Run Riftgate as a sidecar container alongside each inference pod. Traffic from `localhost:<port>` is intercepted. Useful when pod-level policy isolation is required. See [`docs/05-options/018-deployment.md`](../05-options/018-deployment.md) for the tradeoff analysis.

---

## 6. Plugin development

### 6.1 WASM filter (full example)

See [`docs/03-architecture/extension-plane.md §1.4`](../03-architecture/extension-plane.md) for the step-by-step walkthrough.

Quick scaffold:

```bash
cargo new --lib my-riftgate-filter
cd my-riftgate-filter
# Add riftgate-filter-sdk dep, set crate-type = ["cdylib"]
cargo build --target wasm32-wasip1 --release
```

Test your filter locally:

```bash
cat > test.toml <<'EOF'
[server]
listen_addr = "127.0.0.1:8081"
[backend]
url = "http://127.0.0.1:9999"  # mock

[[filter]]
path = "target/wasm32-wasip1/release/my_filter.wasm"
capabilities = ["read-body", "write-body"]
EOF
./riftgate --config test.toml
curl -s http://127.0.0.1:8081/v1/chat/completions -d '{"model":"test","messages":[]}'
```

### 6.2 Custom routing strategy

```rust
// 1. Implement the Router trait (see extension-plane.md §2.4)
// 2. Wire in main.rs:
let router: Arc<dyn Router> = Arc::new(
    CircuitBreakerArbiter::new(MyRouter::new(inner), CircuitBreakerConfig::default())
);
// 3. cargo build --release -p riftgate
```

No config DSL yet — custom routers are compiled into the binary. A plugin-loader for router WASM is a post-v1.0 item.

### 6.3 Custom observability sink

```rust
// 1. Implement ObservabilitySink in a new crate
// 2. Register in crates/riftgate/src/bootstrap.rs:
let my_sink: Arc<dyn ObservabilitySink> = Arc::new(MySink::new());
multi = multi.with(my_sink);
```

---

## 7. Operations

### 7.1 Health and readiness probes

```bash
curl http://localhost:8080/health   # 200 while process is alive
curl http://localhost:8080/ready    # 200 in steady state; 503 while draining
```

Use `/ready` as the Kubernetes readiness probe. Use `/health` as the liveness probe.

### 7.2 Graceful drain (SIGTERM)

```bash
kill -TERM $(pgrep riftgate)
# /ready immediately returns 503 (LB stops routing new connections)
# In-flight requests are allowed to complete
# Process exits after drain_grace_ms (default: 30s)
```

```toml
# Tune drain window
[server]
drain_grace_ms = 30000
```

### 7.3 Rolling restart (Kubernetes)

The operator manages rolling restarts when `spec.image` changes. For standalone:

```bash
# Blue-green: start new instance on a different port, then swap LB target
./riftgate --config riftgate.toml &   # new instance on :8081
# Shift LB, then SIGTERM old instance
```

### 7.4 Config reload

**v1.0**: restart required for config changes (SIGTERM + restart). Hot reload is a post-v1.0 feature.

**Kubernetes**: edit the CRD — the operator reconciles within seconds.

```bash
kubectl patch riftgateroute tenant-acme --type merge \
  -p '{"spec":{"mcp":{"allowedTools":["search-web","read-file"]}}}'
```

### 7.5 Log levels and structured output

```bash
# Runtime log level override (restart not required with env-filter):
RUST_LOG=riftgate=debug,riftgate_mcp=trace ./riftgate --config riftgate.toml

# Pretty format for development:
# [log]
# format = "pretty"
```

Structured JSON log fields: `timestamp`, `level`, `target`, `gateway`, `tenant`, `backend`, `request_id`, `duration_ms`.

### 7.6 MCP audit log inspection

```bash
# Dump the audit WAL to JSON:
riftgate-replay dump --segments /var/riftgate/mcp-audit/
# Output: NDJSON records per authorize() call
# { "tenant": 1, "subject": "search-web", "decision": "allow", ... }
```

### 7.7 BPF programs (Linux + CAP_BPF)

```bash
# Enable BPF observability:
RIFTGATE_ENABLE_BPF=1 ./riftgate --config riftgate.toml
# Startup log: INFO riftgate: BpfSink loaded programs=["cpu_sample","syscall_stall","tcp_retransmit"]

# Verify programs are attached:
sudo bpftool prog list | grep riftgate
```

---

## 8. Security hardening checklist

- [ ] Set `tls_verify = true` (default) — never disable in production
- [ ] Store API keys as SHA-256 hashes in `[multitenancy.api_keys]` — never plaintext
- [ ] Set `mcp.signing_key_hex` to a pinned random 32-byte hex value (not ephemeral)
- [ ] Rotate the MCP signing key by updating config + restarting (cross-replica coordination required)
- [ ] Restrict the gateway port (8080) at the firewall; expose only via an ingress/LB
- [ ] Run as a non-root user; drop all capabilities except `CAP_BPF` if using eBPF
- [ ] Set `enforce = true` in `[mcp]` — only use `false` to calibrate allowlists
- [ ] Use `trusted-header` multitenancy mode **only** on fully trusted internal networks (service mesh with mTLS)
- [ ] Pin `RIFTGATE_ENABLE_BPF` off in environments where `CAP_BPF` is not available or not audited
