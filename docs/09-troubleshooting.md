# Riftgate Troubleshooting Guide

> A reference for diagnosing and resolving problems in a running Riftgate deployment. Organized by symptom. For configuration details see [`docs/07-operator-handbook.md`](07-operator-handbook.md).

---

## 1. Startup failures

### 1.1 Process exits with code 78 immediately

**Symptom:** `riftgate: <error message>` followed by exit code 78.

**Cause:** Config validation failed. Exit 78 is the config-error sentinel.

**Diagnose:**
```bash
./riftgate --config riftgate.toml --check
# Prints every validation error before exiting
```

**Common sub-causes:**
- `backend.url` is empty (required field)
- `mcp.signing_key_hex` is set but not exactly 64 hex characters
- `[multitenancy.api_keys]` key is not in `sha256:<64-hex>` format
- `[filter] path` points to a file that does not exist

### 1.2 `could not bind <addr>: address in use`

```bash
# Find what's on that port:
ss -tlnp | grep 8080
# Kill it or change listen_addr in config
```

### 1.3 WASM filter startup error: `precompile rejected`

**Cause:** ABI version mismatch — the `.wasm` file was built against a different `riftgate:filter` ABI version than the binary.

**Fix:** Rebuild the filter against the same tag as the running binary:
```bash
cargo build --target wasm32-wasip1 --release
# Ensure riftgate-filter-sdk version matches running binary tag
```

### 1.4 BPF sink logs `CompiledOut`

```
INFO riftgate: BpfSink: CompiledOut (feature not enabled)
```

The binary was compiled without `--features bpf`. Rebuild with:
```bash
cargo build --release -p riftgate --features bpf
```

### 1.5 BPF sink logs `DisabledByEnv`

```
INFO riftgate: BpfSink: DisabledByEnv (set RIFTGATE_ENABLE_BPF=1 to enable)
```

Binary supports BPF but `RIFTGATE_ENABLE_BPF` is not set. Set it at runtime.

### 1.6 BPF sink logs `Loaded` but probes fail to attach

**Cause:** Missing `CAP_BPF` capability (or `CAP_SYS_ADMIN` on pre-5.8 kernels).

**Fix:**
```bash
# Add CAP_BPF to the process (Kubernetes securityContext):
# securityContext:
#   capabilities:
#     add: ["BPF", "PERFMON"]
```

---

## 2. Routing and backend issues

### 2.1 All requests return 502 Bad Gateway

**Diagnose:**
```bash
# Check backend health manually:
curl -v http://<backend-url>/health

# Check gateway logs for upstream error:
RUST_LOG=riftgate=debug ./riftgate --config riftgate.toml 2>&1 | grep "upstream error"
```

**Common causes:**
- `backend.url` points to an unreachable host
- TLS mismatch: backend uses HTTP but config has `https://`
- Backend is up but `/v1/chat/completions` is returning 5xx

### 2.2 Requests return 503 Service Unavailable

**Cause A — backpressure:** The in-flight request count exceeded the high-water mark.

```
WARN riftgate: backpressure: queue full, rejecting request
```

**Fix:** Scale out replicas or increase backend pool. In Kubernetes, add more `RiftgateBackend` objects.

**Cause B — circuit breaker open:** Too many backend errors tripped the circuit breaker.

```
WARN riftgate: circuit breaker OPEN for backend=0
```

**Diagnose:** Check backend health. The breaker will probe in half-open state every 5s (default). Tune with `CircuitBreakerConfig` in code.

**Cause C — router rejected:** A custom router returned `RoutingDecision::Reject`. Check custom router logic.

### 2.3 Requests are routed inconsistently for KV-cache workloads

**Symptom:** Same-prefix prompts go to different backends across replicas.

**Cause:** `KvAwareRouter` trie is per-replica — there is no cross-replica state in v1.0.

**Fix options:**
1. Use a prefix-aware L4 LB in front of Riftgate replicas (hash-on-prompt-prefix)
2. Run a single Riftgate replica with high `worker_threads` for KV-sensitive workloads
3. Wait for the `LmcacheRouter` impl (post-v1.0) which delegates to the vLLM LMCache controller

### 2.4 Hedged requests are not racing

**Cause:** `HedgedRouter` only races when the first backend's estimated latency exceeds the P2 threshold. If all backends are fast, hedging does not trigger.

**Check:** Enable debug logging to see hedge decisions:
```bash
RUST_LOG=riftgate_router=debug ./riftgate --config ...
# Look for: "hedging request backend=[0,1]"
```

### 2.5 GPU-pressure routing not working

**Diagnose:**
```bash
# Verify poller is running:
# LOG: "INFO riftgate: GPU pressure poller enabled source=DcgmScrapeSource endpoint=..."

# Verify DCGM endpoint is reachable:
curl http://dcgm-exporter:9400/metrics | grep DCGM_FI_DEV_GPU_UTIL

# If poller logs "GPU pressure poll failed":
# Check that DCGM exporter is running and port 9400 is accessible
```

---

## 3. MCP broker issues

### 3.1 All MCP requests return 403

**Cause A — broker is not configured for the tenant:**
```
WARN riftgate: MCP authorization denied: tenant_unknown
```
**Fix:** Add the tenant to `[mcp.tenants]`. Check the tenant ID being sent:
```bash
# The x-riftgate-tenant header value is the tenant key:
curl -H "x-riftgate-tenant: acme" -H "Authorization: Bearer sk-..." \
  http://localhost:8080/mcp ...
```
Verify `riftgate.toml` has `[mcp.tenants.acme]` (or whatever name/id maps to the tenant).

**Cause B — tool not in allowlist:**
```
WARN riftgate: MCP authorization denied: not_in_allowlist
```
**Fix:** Add the tool to `allowed_tools` in the tenant's allowlist. Use dry-run mode first:
```toml
[mcp]
enforce = false  # log denials but pass all requests
```
Check the WAL to see what's being denied:
```bash
riftgate-replay dump --segments /var/riftgate/mcp-audit/ | jq 'select(.decision=="deny")'
```

**Cause C — time-bounded grant expired:**
```
WARN riftgate: MCP authorization denied: time_bound_grant_expired
```
**Fix:** Update `until_unix_secs` in the grant or move the tool to `allowed_tools`.

### 3.2 Downstream MCP server rejects attestation headers

**Cause:** The downstream server's HMAC verification is failing.

**Diagnose:**
```bash
# Check the signature bytes in the response header:
# riftgate-mcp-signature: <hex>
# Verify it was signed with the expected key
```

**Fix:** Ensure `mcp.signing_key_hex` in gateway config matches the key the downstream server uses for verification. The key must be exactly 64 hex characters (32 bytes). A restart is required when rotating the key.

### 3.3 MCP audit WAL is growing unbounded

The WAL never compacts automatically. Manage it with:
```bash
# Check size:
du -sh /var/riftgate/mcp-audit/

# Archive old segments (safe to rotate; new events go to new segments):
mv /var/riftgate/mcp-audit/seg-0000-*.wal /archive/

# Or: set a retention cron to delete segments older than 30 days
find /var/riftgate/mcp-audit/ -name "*.wal" -mtime +30 -delete
```

---

## 4. Authentication and multitenancy issues

### 4.1 API key authentication always returns 401

**Cause A — key format wrong:** The `[multitenancy.api_keys]` entry must be `sha256:<64-hex>`, not the raw key.

**Generate the hash:**
```bash
echo -n "my-raw-api-key" | sha256sum
# Copy the hex output and use as: "sha256:<hex>" = "tenant-name"
```

**Cause B — mode mismatch:** `[multitenancy] mode = "trusted-header"` is set but the client is sending `Authorization: Bearer ...` — header mode does not validate the bearer token.

**Fix:** Set `mode = "api-key"`.

**Cause C — unknown key:** The hash doesn't match any entry in `api_keys`. Enable debug logging:
```bash
RUST_LOG=riftgate_config=debug ./riftgate ...
# "tenant resolver: api-key mode" confirms mode is correct
# If still 401, recompute sha256 of the key the client is sending
```

### 4.2 All requests resolve to TenantId(0)

**Cause:** `mode = "trusted-header"` and the client is not sending `x-riftgate-tenant`. The fallback is `TenantId(0)`.

**Fix:** Either send `x-riftgate-tenant: <id>` from the client, or switch to `api-key` mode.

---

## 5. Kubernetes operator issues

### 5.1 Operator pod is running but gateway Deployment is not created

**Diagnose:**
```bash
kubectl logs deployment/riftgate-operator
# Look for: "reconciling Riftgate" and then "ConfigMap patched" / "Deployment patched"
# OR: "API error: ..." which means RBAC is wrong
```

**RBAC check:**
```bash
kubectl auth can-i create configmaps --as=system:serviceaccount:default:riftgate-operator
kubectl auth can-i create deployments --as=system:serviceaccount:default:riftgate-operator
# If "no": check ClusterRole and ClusterRoleBinding created by the Helm chart
```

### 5.2 CRD objects fail to apply with `no kind "Riftgate" is registered`

The CRD schemas were not installed. Install them:
```bash
kubectl apply -f deploy/helm/riftgate-operator/templates/crds.yaml
# OR via Helm:
helm upgrade --install riftgate-operator deploy/helm/riftgate-operator/
```

### 5.3 Operator reconcile loop is fast-failing

```bash
kubectl logs deployment/riftgate-operator | grep "reconcile error"
# Common: "API error: list backends: ..." → RBAC missing for riftgatebackends
```

### 5.4 API key Secret not being read

**Cause:** The Secret referenced by `apiKeySecretRef` doesn't exist or the operator lacks `get secrets` permission.

```bash
# Verify secret exists:
kubectl get secret acme-keys -n default

# Verify operator can read it:
kubectl auth can-i get secret/acme-keys \
  --as=system:serviceaccount:default:riftgate-operator
```

---

## 6. Performance issues

### 6.1 High latency on first token (TTFT)

**What to observe:**
- OTel span `riftgate.request.dispatched → riftgate.request.first_token` duration
- Backend GPU utilization via DCGM/NVML

**Check GPU pressure routing:**
```bash
RUST_LOG=riftgate_router=debug ./riftgate ... 2>&1 | grep gpu_pressure
```

**Common fixes:**
- Enable GPU pressure routing (`RIFTGATE_GPU_DCGM_ENDPOINT`)
- Enable KV-cache routing for repeated-prefix workloads (`KvAwareRouter`)
- Increase backend pool (add more `RiftgateBackend` objects)

### 6.2 Request queue depth is spiking

```
WARN riftgate: backpressure: approaching high-water mark queue_depth=3800 limit=4096
```

**Fix options:**
1. Add replicas or scale up the backend pool
2. Increase `RIFTGATE__OBS_BUS_CAPACITY` if the OTel sink is slow
3. Tune rate limiting (`[rate_limit]` section) to shed load earlier

### 6.3 p99 latency is high despite low backend latency

**Diagnose with BPF (Linux):**
```bash
RIFTGATE_ENABLE_BPF=1 ./riftgate --config ...
# BpfSink collects cpu_sample (on/off time), syscall_stall, tcp_retransmit
# Check OTel for bpf.cpu_off_time and bpf.syscall_stall_ns
```

**Common causes:**
- Scheduler stalls when `worker_threads` is not aligned with physical CPU topology
- GC pressure from large per-request arena allocations (check BumpArena usage)

### 6.4 OTel sink is dropping events

```
WARN riftgate: observability bus drop (bus full); increase obs.bus_capacity
```

**Fix:**
```toml
[obs]
bus_capacity = 16384   # increase from default 4096
```

Or reduce `sample_rate` to emit fewer spans.

---

## 7. What to always check first

When something goes wrong, collect this information before investigating further:

```bash
# 1. Gateway health
curl -i http://localhost:8080/health
curl -i http://localhost:8080/ready

# 2. Last 100 log lines
journalctl -u riftgate -n 100 --no-pager
# or: docker logs riftgate --tail 100

# 3. Config validation
./riftgate --config riftgate.toml --check

# 4. Active backend connectivity
curl -v http://<backend-url>/health

# 5. MCP audit (if using MCP)
riftgate-replay dump --segments /var/riftgate/mcp-audit/ | tail -20

# 6. OTel span data (5-minute window)
# Query OTel collector or Jaeger for riftgate.* spans
# Sort by duration DESC to find slow requests

# 7. Kubernetes: operator status
kubectl get riftgates,riftgatebackends,riftgateroutes -A
kubectl logs deployment/riftgate-operator | tail -50
```
