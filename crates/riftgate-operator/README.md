# riftgate-operator

Kubernetes operator for the Riftgate gateway (`riftgate.io/v1alpha1` CRDs).

Reconciles three CRD types into running gateway `Deployment` + `ConfigMap` objects:

| CRD | Short name | Purpose |
|-----|-----------|---------|
| `Riftgate` | `rg` | Gateway instance (image, listen address, drain grace, OTel endpoint) |
| `RiftgateBackend` | `rgb` | Upstream backend (static URL or Service ref, auth Secret, circuit breaker) |
| `RiftgateRoute` | `rgr` | Routing rule (path prefix, backend ref, MCP policy, API key auth) |

See [ADR 0030](../../docs/06-adrs/0030-k8s-operator-crds.md) and
[Options 018](../../docs/05-options/018-deployment.md) for the design rationale.

## Build

```bash
# Stub binary (no Kubernetes headers required):
cargo build -p riftgate-operator

# Live controller binary:
cargo build -p riftgate-operator --features operator
```

## Deploy

```bash
# Install CRDs + operator via Helm:
helm install riftgate-operator deploy/helm/riftgate-operator/

# Apply a gateway instance:
kubectl apply -f deploy/examples/basic-gateway.yaml
```

## CRD examples

```yaml
apiVersion: riftgate.io/v1alpha1
kind: Riftgate
metadata:
  name: prod-gateway
spec:
  image: "ghcr.io/sgpopuri/riftgate:v1.0.0"
  listenAddr: "0.0.0.0:8080"
  drainGraceMs: 30000
  obsEndpoint: "http://otel-collector:4317"
  replicas: 2
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateBackend
metadata:
  name: llm-prod
spec:
  serviceRef:
    name: vllm-service
    port: 8000
  authSecretRef:
    name: llm-api-key
    key: value
  timeoutMs: 30000
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateRoute
metadata:
  name: tenant-acme
spec:
  pathPrefix: "/v1/"
  backendRef: llm-prod
  weight: 100
  mcp:
    enforce: true
    allowedTools: ["search-web", "read-file"]
  multitenancy:
    apiKeySecretRef:
      name: acme-api-key
      key: keys
```

## Status

The controller loop is a stub pending the `kube-runtime` wiring (ADR 0030 Phase A).
The CRD type definitions, config renderer, and reconciler interfaces are complete.
