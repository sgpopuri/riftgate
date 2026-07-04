# 018. Deployment model — K8s operator, sidecar, or standalone binary

> **Status:** `recommended` — Kubernetes operator with CRDs as the production path; standalone binary and sidecar manifest as stepping stones. Target milestone: `v1.0`. See [ADR 0030](../06-adrs/0030-k8s-operator-crds.md).
> **Foundational topics:** Kubernetes controller pattern (level-triggered reconciliation, informers, client-go), CRD schema design, sidecar / ambassador deployment patterns (Hohpe, *Enterprise Integration Patterns*; Microsoft *Cloud Design Patterns*)
> **Related options:** [`015-config-model`](015-config-model.md) (the config substrate that CRDs extend), [`017-multitenancy`](017-multitenancy.md) (per-tenant identity sourced from CRDs at v1.1+)
> **Related ADR:** [ADR 0030](../06-adrs/0030-k8s-operator-crds.md)

## 1. The decision in one sentence

> How is Riftgate deployed and configured in production — as a standalone binary managed by an operator's own tooling, as a sidecar alongside each inference pod, or via a Kubernetes operator that owns the lifecycle of Riftgate instances and translates CRDs into running gateway configurations?

## 2. Context — what forces this decision

Riftgate through `v0.5` is distributed as a standalone binary configured by a TOML file. This is correct for development and for early adopters who manage their own infrastructure, but it creates three operational gaps as Riftgate targets production:

1. **Configuration drift.** Multiple Riftgate instances (one per cluster zone, one per tenant segment) will diverge unless config is managed externally. Without a Kubernetes-native config surface, operators have no GitOps-compatible way to reconcile desired state.
2. **Backend registration.** In a Kubernetes cluster, inference backends (`vllm`, `llm-d`, `triton`) are pods with ephemeral IPs. Riftgate's static `[backend.url]` config cannot track live pod IPs without an operator watching the Kubernetes API.
3. **Tenant policy as code.** The MCP allowlist (`[mcp.tenants.*]`) and API key registry (`[multitenancy.api_keys]`) in a TOML file are not version-controlled alongside the cluster state that a platform team manages via GitOps.

The non-functional requirement [`NFR-OPS01`](../01-requirements/non-functional.md) requires a Kubernetes-native deployment path; [`FR-401`](../01-requirements/functional.md) names the CRDs explicitly.

Three deployment shapes are in scope. The decision is which ships first, in what form, and what the long-term primary path is.

## 3. Candidates

### 3.1. Standalone binary (current)

**What it is.** A single binary launched with `--config riftgate.toml` (or Helm-managed ConfigMap). No Kubernetes awareness. Operators manage instances, scaling, and config via their existing tooling (systemd, Docker Compose, raw Kubernetes Deployments + ConfigMaps).

**Why it's interesting.**
- Zero additional code beyond what exists. The binary already runs.
- Simple mental model: one process, one config file.
- Correct for: bare-metal deployments, local development, air-gapped environments, operators who prefer to own their config pipeline.
- Can be "Kubernetes-native" with a Helm chart + ConfigMap without a custom operator.

**Where it falls short.**
- Backend discovery is static. Dynamic backend pools (pods coming and going) require external tooling (e.g. a custom ConfigMap generator) to update Riftgate's config.
- Tenant policy and API key management are outside Kubernetes RBAC; no admission webhooks.
- Multi-instance consistency requires external coordination (Consul, etcd, shared ConfigMap) for per-instance state.
- Not the path that platform teams expect for production gateway infrastructure.

**Real-world analogy.** Envoy deployed as a raw binary without xDS, or nginx with a static `nginx.conf`.

### 3.2. Sidecar per inference pod

**What it is.** One Riftgate instance runs as a sidecar container in each inference pod (alongside `vllm` or similar). The sidecar intercepts traffic to the inference container on `localhost`. The sidecar is injected automatically by a MutatingWebhook or included in the pod template.

**Why it's interesting.**
- Eliminates the "pod IP discovery" problem: the sidecar and inference container share the pod network namespace.
- Per-pod config (e.g. a different MCP allowlist per model deployment) is expressed as pod annotations or a sidecar config mounted from a ConfigMap.
- Matches the Istio/Linkerd sidecar model, which many platform teams already understand.
- Supports per-pod isolation: one Riftgate instance failure does not affect other pods.

**Where it falls short.**
- Every inference pod carries Riftgate's binary footprint and CPU/memory overhead. For high-density clusters this adds up.
- Rate limiting (`TokenBucketLimiter`) is per-pod, not cluster-wide. Operators who need cluster-wide token quotas must aggregate in-process state across sidecars (requires a distributed rate limiter — a future ADR).
- The sidecar model complicates upgrades: Riftgate version bumps require rolling restart of every inference pod, not just the gateway.
- The MCP audit WAL is per-pod: correlating audit events across the cluster requires log aggregation.

**Real-world analogy.** Istio sidecar proxy (Envoy), Linkerd proxy, NGINX as a sidecar in multi-container pods.

### 3.3. Kubernetes operator with CRDs (recommended)

**What it is.** A Kubernetes operator (`riftgate-operator`) watches three CRD types:

- **`Riftgate`** — one instance of the gateway: config (listen address, TLS, drain grace), observability endpoint, rate-limit policy.
- **`RiftgateBackend`** — a named upstream backend: URL, auth secret ref, circuit-breaker config, health check.
- **`RiftgateRoute`** — a routing rule: path prefix, backend selector, MCP allowlist, weight.

The operator reconciles `Riftgate` + `RiftgateBackend` + `RiftgateRoute` objects into a running gateway Deployment with the appropriate TOML config mounted as a ConfigMap. It also watches `Service` objects to keep backend URLs current as pods are rescheduled.

**Why it's interesting.**
- **GitOps-native.** CRDs are YAML manifests that platform teams commit, review, and apply with standard Kubernetes tooling (kubectl, ArgoCD, Flux).
- **Dynamic backend registration.** The operator watches `Endpoints` objects and updates `RiftgateBackend` URLs when pods are rescheduled — no manual config update.
- **Kubernetes RBAC for policy.** Tenant allowlists and API key registries live in `RiftgateRoute` and `Secret` objects, protected by Kubernetes RBAC and audited by the Kubernetes audit log.
- **Upgrade path.** Updating the Riftgate binary version is a CRD field change (`spec.image`); the operator rolls it out as a Deployment rolling update, decoupled from inference pod restarts.
- **Single-gateway topology.** One `Riftgate` instance can serve the whole cluster, with `RiftgateRoute` objects differentiating tenants. No per-pod overhead.

**Where it falls short.**
- The operator is significant new code: a Kubernetes controller in Rust (using `kube-rs`) or Go. This is a multi-week implementation.
- Adds a Kubernetes API server dependency: Riftgate cannot run without a live API server when the operator manages its lifecycle (mitigated: the binary still runs standalone; the operator is layered on top).
- Operator patterns introduce complexity (leader election, finalizers, status sub-resources) that must be implemented correctly to avoid split-brain.

**Code sketch (CRD shape):**
```yaml
apiVersion: riftgate.io/v1alpha1
kind: Riftgate
metadata:
  name: prod-gateway
spec:
  listenAddr: "0.0.0.0:8080"
  drainGraceMs: 30000
  image: "ghcr.io/sgpopuri/riftgate:v1.0.0"
  obsEndpoint: "http://otel-collector:4317"
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateBackend
metadata:
  name: llm-prod
spec:
  url: "http://vllm-service:8000"
  authSecretRef: { name: llm-api-key, key: value }
  timeoutMs: 30000
---
apiVersion: riftgate.io/v1alpha1
kind: RiftgateRoute
metadata:
  name: tenant-acme
spec:
  pathPrefix: "/v1/"
  backendRef: llm-prod
  mcp:
    enforce: true
    allowedTools: ["search-web", "read-file"]
  multitenancy:
    apiKeySecretRef: { name: acme-api-key, key: value }
```

**Real-world analogy.** The Gateway API (Kubernetes SIG-Network), cert-manager, external-secrets-operator, Prometheus Operator — all use the same pattern: CRDs express desired state; a controller reconciles it.

### 3.4. Helm chart only (no operator)

**What it is.** A Helm chart that renders a Kubernetes Deployment + ConfigMap from chart values. No custom CRDs; no controller. Operators use `helm upgrade` to change config.

**Why it's interesting.**
- Familiar to most Kubernetes operators (Helm is the de-facto package manager).
- Much less code than a full operator: just Go templates.
- Correct as a first step toward Kubernetes deployment, and as a packaging layer on top of the operator.

**Where it falls short.**
- No dynamic backend discovery: the Helm chart renders the same static TOML as the standalone binary, just packaged.
- No CRD-based RBAC or admission webhook for policy changes.
- Upgrades require `helm upgrade`, not a controller-managed rolling strategy.

**Verdict.** A Helm chart is the packaging layer for the operator, not a standalone deployment model. Rejected as the primary path; accepted as the distribution mechanism for the operator itself.

## 4. Tradeoff matrix

| Property | Standalone binary | Sidecar | K8s operator + CRDs | Helm chart only |
|---|---|---|---|---|
| Dynamic backend discovery | No (static config) | Yes (pod localhost) | Yes (operator watches Endpoints) | No (static config) |
| GitOps compatibility | Via ConfigMap (manual) | Via pod annotations | Yes (CRDs are K8s objects) | Via values.yaml (partial) |
| Cluster-wide rate limiting | Yes (single process) | No (per-pod bucket) | Yes (single process) | Yes (single process) |
| Pod resource overhead | Single gateway | Per-pod (multiplied) | Single gateway | Single gateway |
| Upgrade decoupling from pods | N/A | No (tied to pod lifecycle) | Yes (operator rolls Deployment) | Partial |
| Kubernetes RBAC for policy | No | Via annotations | Yes (CRD RBAC) | No |
| Implementation cost | Zero | Low (webhook + config) | High (controller) | Low (templates) |
| Standalone / air-gapped support | Yes | Yes | Yes (binary still works) | Yes (chart works offline) |

## 5. Foundational principles

**Level-triggered reconciliation** (the Kubernetes controller model): a controller watches desired state (CRDs) and actual state (Deployments, ConfigMaps) and continuously drives them toward convergence. This is more robust than edge-triggered approaches (webhooks that fire on change) because it heals from missed events. Reference: *Programming Kubernetes* (Hausenblas & Schimanski, 2019); `kube-rs` ([github.com/kube-rs/kube](https://github.com/kube-rs/kube)).

**Ambassador pattern** (Hohpe, *Enterprise Integration Patterns*; Microsoft *Cloud Design Patterns*): a dedicated gateway component in front of backend services — exactly what Riftgate is. The operator pattern is how the ambassador is managed in a Kubernetes cluster.

**Operator pattern** (CoreOS, 2016): encode domain-specific operational knowledge in a controller. The first operators (etcd-operator, prometheus-operator) demonstrated that Kubernetes CRDs + a controller produce a more reliable operational surface than manual YAML + Helm for stateful or policy-rich systems. Reference: Brandon Philips, "Introducing Operators" (2016, CoreOS blog).

**CRD schema versioning** (Kubernetes API machinery): CRD fields are versioned (`v1alpha1`, `v1beta1`, `v1`). Riftgate starts at `v1alpha1` (no stability guarantee) and promotes to `v1` when the schema is stable. This matches the graduation path of every major Kubernetes extension (Gateway API, cert-manager, etc.).

## 6. Recommendation

**K8s operator with CRDs** (`3.3`) is the v1.0 target. The implementation proceeds in phases:

- **Phase A (v1.0):** `riftgate-operator` crate using `kube-rs`; reconciles `Riftgate` + `RiftgateBackend` objects into a ConfigMap + Deployment. `RiftgateRoute` with MCP allowlist and API key secret ref. Helm chart packages the operator for distribution.
- **Phase B (v1.1+):** Dynamic backend discovery via `Endpoints` watch; MutatingWebhook for sidecar injection; `RiftgateRoute` weight-based routing.

The standalone binary is not deprecated; the operator is layered on top. An operator that cannot reach the API server falls back to reading the TOML config file.

The Helm chart is the distribution mechanism for the operator itself. It is not a substitute for the operator.

**Implementation language for the operator:** Rust, using `kube-rs` (`kube = "0.98"`). This keeps the entire Riftgate surface in Rust and avoids a Go dependency. The `kube-rs` crate is production-grade and used by multiple Kubernetes operators. Reference: `kube-rs` [docs.rs/kube](https://docs.rs/kube/latest/kube/).

## 7. What we explicitly reject

**Sidecar as the default.** Per-pod overhead and per-pod rate limiting buckets are the wrong defaults for a single-operator deployment. The sidecar path is available (the binary runs anywhere) but is not the primary documented model.

**Helm chart only (no operator).** A Helm chart alone does not solve dynamic backend discovery or CRD-based policy. It is the packaging layer, not the deployment model.

**Go operator.** Introducing a Go codebase alongside Rust would split the implementation language, add a second toolchain, and fragment documentation. `kube-rs` provides the same controller machinery in Rust.

## 8. References

1. Hausenblas, M., Schimanski, S. (2019). *Programming Kubernetes.* O'Reilly Media.
2. Philips, B. (2016). "Introducing Operators: Putting Operational Knowledge into Software." CoreOS Blog. [web.archive.org/web/20170129131616/https://coreos.com/blog/introducing-operators.html](https://web.archive.org/web/20170129131616/https://coreos.com/blog/introducing-operators.html)
3. `kube-rs`. Kubernetes client and controller runtime for Rust. [github.com/kube-rs/kube](https://github.com/kube-rs/kube) / [docs.rs/kube](https://docs.rs/kube)
4. Kubernetes. *Custom Resources.* [kubernetes.io/docs/concepts/extend-kubernetes/api-extension/custom-resources](https://kubernetes.io/docs/concepts/extend-kubernetes/api-extension/custom-resources/)
5. Kubernetes SIG-Network. *Gateway API.* [gateway-api.sigs.k8s.io](https://gateway-api.sigs.k8s.io) (reference pattern for CRD-based gateway config)
6. Hohpe, G., Woolf, B. (2003). *Enterprise Integration Patterns.* Addison-Wesley. (Ambassador pattern)
7. Microsoft. *Ambassador Pattern.* [learn.microsoft.com/en-us/azure/architecture/patterns/ambassador](https://learn.microsoft.com/en-us/azure/architecture/patterns/ambassador)
