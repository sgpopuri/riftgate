# ADR 0030. Kubernetes operator with CRDs as the v1.0 deployment model; `kube-rs` as the operator runtime

> **Date:** 2026-07-04
> **Status:** accepted
> **Options doc:** [018-deployment](../05-options/018-deployment.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate through `v0.5` deploys as a standalone binary with a TOML config file. This works for development and early adopters but creates operational gaps for production: static backend URLs cannot track ephemeral Kubernetes pod IPs, tenant policy lives outside Kubernetes RBAC, and there is no GitOps-compatible surface for config changes. `FR-401` requires a Kubernetes operator with CRDs. Full exploration of four deployment shapes (standalone, sidecar, operator + CRDs, Helm-only) lives in [Options `018`](../05-options/018-deployment.md).

## Decision

**`v1.0` ships a Kubernetes operator (`riftgate-operator`) that reconciles three CRD types (`Riftgate`, `RiftgateBackend`, `RiftgateRoute`) into a running gateway Deployment + ConfigMap, implemented in Rust using `kube-rs`.**

The specifics:

- New crate `crates/riftgate-operator`, depending on `kube = "0.98"` (workspace dep) and `kube-runtime` for the controller runtime.
- Three CRD types at `apiVersion: riftgate.io/v1alpha1`:
  - **`Riftgate`** — gateway instance spec (listen address, image, drain grace, observability endpoint, rate-limit defaults).
  - **`RiftgateBackend`** — upstream backend (URL or service ref, auth secret ref, circuit-breaker config, timeout).
  - **`RiftgateRoute`** — routing rule (path prefix, backend ref, weight, MCP allowlist, API key secret ref).
- The operator reconciles these objects into a `ConfigMap` (rendered TOML from CRD fields) and a `Deployment` (Riftgate binary with the ConfigMap mounted as `--config`).
- The operator watches `Endpoints` objects for backends defined by a `serviceRef` (instead of a static URL) to keep backend URLs current as pods reschedule.
- A Helm chart in `deploy/helm/riftgate-operator/` packages the operator for distribution. The chart installs the CRD schemas, the operator Deployment, and the required RBAC (`ClusterRole`, `ClusterRoleBinding`, `ServiceAccount`).
- The standalone binary is not deprecated. An operator that cannot reach the Kubernetes API server logs a warning and reads `--config` as a fallback.
- `kube-rs` is chosen over a Go operator because it keeps the entire Riftgate surface in Rust, avoids a second toolchain, and `kube-rs` is production-grade (used by multiple real-world operators).
- The CRD API group is `riftgate.io`; the initial version is `v1alpha1` (no stability guarantee until promoted to `v1`).

## Consequences

- **Positive:**
  - Platform teams can manage Riftgate config via GitOps (kubectl, ArgoCD, Flux) — CRDs are first-class Kubernetes objects.
  - Dynamic backend discovery: the operator watches `Endpoints` and updates the ConfigMap without a manual restart.
  - Kubernetes RBAC gates who can create/modify `RiftgateRoute` objects (and therefore tenant policy and MCP allowlists).
  - Operator-managed rolling upgrades: bumping `spec.image` in the `Riftgate` CRD triggers a Deployment rolling update, decoupled from inference pod restarts.
  - The standalone binary path is preserved for non-Kubernetes deployments and air-gapped environments.
- **Negative / accepted tradeoffs:**
  - `riftgate-operator` is a significant new crate (~1000–2000 lines for the controller skeleton, CRD types, and reconciler). This is multi-week work.
  - Leader election and finalizer logic must be implemented correctly to avoid split-brain; `kube-runtime`'s `Controller` handles most of this but still requires careful design.
  - The `kube-rs` and Kubernetes API machinery are new dependencies in the workspace. Adding `kube`, `kube-runtime`, and `k8s-openapi` adds ~50 transitive dependencies.
  - `v1alpha1` CRD schema may break between v1.0 and v1.1. Documented explicitly; migration tooling (conversion webhooks) deferred until `v1beta1` promotion.
- **Future work this enables:**
  - `v1alpha1` → `v1beta1` → `v1` CRD graduation as the schema stabilizes.
  - MutatingWebhook for sidecar injection (Riftgate as a per-pod sidecar, alongside the operator-managed single-gateway topology).
  - CRD-driven API key registry (move `[multitenancy.api_keys]` from TOML to `RiftgateRoute.spec.multitenancy.apiKeySecretRef`), completing ADR 0029's v1.1 roadmap item.
  - `cargo publish` decision at v1.0 retrospective — the operator crate is part of that scope.
- **Future work this forecloses (until superseded):**
  - The Helm-chart-only (no operator) path is not the production default. It remains available as a distribution mechanism for the operator itself.
  - A Go operator is explicitly rejected; any future operator work stays in Rust.

## Compliance

- `crates/riftgate-operator/` exists with at minimum: CRD type definitions (`Riftgate`, `RiftgateBackend`, `RiftgateRoute` as `kube::CustomResource` structs), a stub reconciler, and a `main.rs` that starts the controller against a live or mock API server.
- `deploy/helm/riftgate-operator/` contains a functional Helm chart that installs CRD schemas and operator RBAC.
- A smoke test starts a `k3s` or `envtest` API server in CI and verifies that creating a `Riftgate` + `RiftgateBackend` CRD object causes the operator to create the expected `ConfigMap` and `Deployment`.
- Adding a new CRD type requires a new ADR superseding this one (or extending it) and a demonstrated migration path.
- The `v1alpha1` API group must be documented with an explicit stability disclaimer in the operator README.

## Notes

- The API group `riftgate.io` reserves a domain. This is consistent with the pattern used by `cert-manager.io`, `gateway.networking.k8s.io`, etc. The domain does not need to be a live website; it is a namespace identifier.
- `kube = "0.98"` (or the current stable at v1.0 implementation time) is the workspace dep. The `kube-rs` project follows Kubernetes release cadence (~4 releases/year); pinning is handled via the workspace `Cargo.toml`.
- The operator crate uses the same `clippy --deny warnings` and `cargo fmt --check` gates as every other crate in the workspace.
