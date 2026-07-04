//! # riftgate-operator
//!
//! Kubernetes operator for the Riftgate gateway.
//!
//! Reconciles three CRD types at `apiVersion: riftgate.io/v1alpha1`:
//!
//! - [`RiftgateSpec`] / `Riftgate` — a gateway instance (listen address, image,
//!   drain grace, observability endpoint).
//! - [`RiftgateBackendSpec`] / `RiftgateBackend` — an upstream backend (URL or
//!   Kubernetes Service ref, auth Secret ref, circuit-breaker config).
//! - [`RiftgateRouteSpec`] / `RiftgateRoute` — a routing rule (path prefix,
//!   backend ref, weight, MCP allowlist, API key Secret ref).
//!
//! The operator translates these objects into a `ConfigMap` (rendered TOML) and
//! a `Deployment` (Riftgate binary with `--config` pointing at the ConfigMap).
//!
//! ## Feature flags
//!
//! - `operator` — enables the live Kubernetes controller runtime (`kube`,
//!   `kube-runtime`, `k8s-openapi`). Without this flag the crate compiles to
//!   stub types only, which allows `cargo check --workspace` to succeed without
//!   Kubernetes headers.
//!
//! See [ADR 0030](../../docs/06-adrs/0030-k8s-operator-crds.md) and
//! [Options 018](../../docs/05-options/018-deployment.md).

pub mod crds;
pub mod reconciler;
pub mod render;
