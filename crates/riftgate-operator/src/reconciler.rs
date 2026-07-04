//! Reconcilers for `Riftgate`, `RiftgateBackend`, and `RiftgateRoute`.
//!
//! The stub types (`ReconcileAction`, `ReconcileError`) compile without the
//! `operator` feature so `cargo check --workspace` succeeds without Kubernetes
//! headers. The real kube-runtime controller wiring is behind
//! `#[cfg(feature = "operator")]`.

use thiserror::Error;

// ---------------------------------------------------------------------------
// Shared types (always compiled)
// ---------------------------------------------------------------------------

/// Placeholder reconciliation outcome (stub mode, no kube dep).
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReconcileAction {
    /// Reconciliation succeeded; requeue after the default interval.
    Ok,
    /// Transient error; requeue after `backoff_secs`.
    Requeue {
        /// Seconds before the next attempt.
        backoff_secs: u64,
    },
}

/// Error type for reconcile failures.
#[derive(Debug, Error)]
pub enum ReconcileError {
    /// A required CRD field was missing or invalid.
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
    /// Kubernetes API call failed.
    #[error("Kubernetes API error: {0}")]
    ApiError(String),
    /// TOML config rendering failed.
    #[error("config render error: {0}")]
    RenderError(String),
}

// Map kube::Error into ReconcileError for the live controller path.
#[cfg(feature = "operator")]
impl From<kube::Error> for ReconcileError {
    fn from(e: kube::Error) -> Self {
        Self::ApiError(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Stub reconcilers (always compiled; used in tests without kube)
// ---------------------------------------------------------------------------

/// Stub reconciler — used in tests and when the `operator` feature is off.
pub fn reconcile_riftgate(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(gateway = name, "reconciling Riftgate (stub)");
    Ok(ReconcileAction::Ok)
}

/// Stub reconciler for `RiftgateBackend`.
pub fn reconcile_backend(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(backend = name, "reconciling RiftgateBackend (stub)");
    Ok(ReconcileAction::Ok)
}

/// Stub reconciler for `RiftgateRoute`.
pub fn reconcile_route(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(route = name, "reconciling RiftgateRoute (stub)");
    Ok(ReconcileAction::Ok)
}

// ---------------------------------------------------------------------------
// Live kube-runtime controller (operator feature only)
// ---------------------------------------------------------------------------

#[cfg(feature = "operator")]
pub mod live {
    //! Live controller loop using `kube-runtime`.
    //!
    //! Call [`run_controllers`] from `main.rs` when the `operator` feature is
    //! enabled. It starts one `kube_runtime::Controller` per CRD type and drives
    //! them to completion (which is `Future::pending` in steady state).

    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;
    use kube::runtime::controller::{Action, Controller};
    use kube::runtime::watcher;
    use kube::{Api, Client, ResourceExt};

    use super::ReconcileError;
    use crate::crds::{Riftgate, RiftgateBackend, RiftgateRoute};
    use crate::render;

    /// Shared context injected into every reconcile call.
    pub struct Ctx {
        /// Live Kubernetes API client.
        pub client: Client,
    }

    // ------------------------------------------------------------------
    // Riftgate reconciler
    // ------------------------------------------------------------------

    async fn reconcile_riftgate(
        obj: Arc<Riftgate>,
        ctx: Arc<Ctx>,
    ) -> Result<Action, ReconcileError> {
        let name = obj.name_any();
        let ns = obj.namespace().unwrap_or_else(|| "default".to_owned());
        tracing::info!(gateway = %name, namespace = %ns, "reconciling Riftgate");

        // Render TOML config from spec.
        let toml = render::render_config(&obj.spec, &[], &[]);
        tracing::debug!(gateway = %name, bytes = toml.len(), "rendered gateway config");

        // TODO(v1.0 Phase B): patch ConfigMap + Deployment in the namespace.
        // The ConfigMap name convention: "<gateway-name>-config".
        // The Deployment name convention: "<gateway-name>".
        let _ = ctx.client.clone(); // keep reference live until Phase B

        Ok(Action::requeue(Duration::from_secs(300)))
    }

    fn error_policy_riftgate(
        _obj: Arc<Riftgate>,
        error: &ReconcileError,
        _ctx: Arc<Ctx>,
    ) -> Action {
        tracing::warn!(error = %error, "Riftgate reconcile error; backing off");
        Action::requeue(Duration::from_secs(30))
    }

    // ------------------------------------------------------------------
    // RiftgateBackend reconciler
    // ------------------------------------------------------------------

    async fn reconcile_backend(
        obj: Arc<RiftgateBackend>,
        _ctx: Arc<Ctx>,
    ) -> Result<Action, ReconcileError> {
        let name = obj.name_any();
        tracing::info!(backend = %name, "reconciling RiftgateBackend");
        // TODO(v1.0 Phase B): if serviceRef is set, watch Endpoints and patch
        // the owning Riftgate's ConfigMap when pod IPs change.
        Ok(Action::requeue(Duration::from_secs(300)))
    }

    fn error_policy_backend(
        _obj: Arc<RiftgateBackend>,
        error: &ReconcileError,
        _ctx: Arc<Ctx>,
    ) -> Action {
        tracing::warn!(error = %error, "RiftgateBackend reconcile error");
        Action::requeue(Duration::from_secs(30))
    }

    // ------------------------------------------------------------------
    // RiftgateRoute reconciler
    // ------------------------------------------------------------------

    async fn reconcile_route(
        obj: Arc<RiftgateRoute>,
        _ctx: Arc<Ctx>,
    ) -> Result<Action, ReconcileError> {
        let name = obj.name_any();
        tracing::info!(route = %name, "reconciling RiftgateRoute");
        // TODO(v1.0 Phase B): resolve API key Secret, regenerate owning
        // Riftgate's ConfigMap.
        Ok(Action::requeue(Duration::from_secs(300)))
    }

    fn error_policy_route(
        _obj: Arc<RiftgateRoute>,
        error: &ReconcileError,
        _ctx: Arc<Ctx>,
    ) -> Action {
        tracing::warn!(error = %error, "RiftgateRoute reconcile error");
        Action::requeue(Duration::from_secs(30))
    }

    // ------------------------------------------------------------------
    // Controller startup
    // ------------------------------------------------------------------

    /// Start all three controllers and drive them to completion.
    ///
    /// This future resolves only if all controllers terminate, which in practice
    /// means the process exits. Intended to be the outermost `await` in `main`.
    pub async fn run_controllers(client: Client, namespace: Option<&str>) {
        let watcher_config = watcher::Config::default();
        let ctx = Arc::new(Ctx {
            client: client.clone(),
        });

        let (gateways, backends, routes): (
            Api<Riftgate>,
            Api<RiftgateBackend>,
            Api<RiftgateRoute>,
        ) = match namespace {
            Some(ns) => (
                Api::namespaced(client.clone(), ns),
                Api::namespaced(client.clone(), ns),
                Api::namespaced(client.clone(), ns),
            ),
            None => (
                Api::all(client.clone()),
                Api::all(client.clone()),
                Api::all(client.clone()),
            ),
        };

        let riftgate_ctrl = Controller::new(gateways, watcher_config.clone())
            .run(reconcile_riftgate, error_policy_riftgate, ctx.clone())
            .for_each(|res| async move {
                match res {
                    Ok((obj, _)) => tracing::info!(name = %obj.name, "Riftgate reconciled"),
                    Err(e) => tracing::warn!(error = ?e, "Riftgate controller error"),
                }
            });

        let backend_ctrl = Controller::new(backends, watcher_config.clone())
            .run(reconcile_backend, error_policy_backend, ctx.clone())
            .for_each(|res| async move {
                match res {
                    Ok((obj, _)) => tracing::info!(name = %obj.name, "RiftgateBackend reconciled"),
                    Err(e) => tracing::warn!(error = ?e, "RiftgateBackend controller error"),
                }
            });

        let route_ctrl = Controller::new(routes, watcher_config)
            .run(reconcile_route, error_policy_route, ctx)
            .for_each(|res| async move {
                match res {
                    Ok((obj, _)) => tracing::info!(name = %obj.name, "RiftgateRoute reconciled"),
                    Err(e) => tracing::warn!(error = ?e, "RiftgateRoute controller error"),
                }
            });

        // Drive all three controllers concurrently.
        tokio::join!(riftgate_ctrl, backend_ctrl, route_ctrl);
    }
}

// ---------------------------------------------------------------------------
// Tests (always compiled, no kube dep)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_riftgate_returns_ok() {
        assert_eq!(
            reconcile_riftgate("prod-gateway").unwrap(),
            ReconcileAction::Ok
        );
    }

    #[test]
    fn reconcile_backend_returns_ok() {
        assert_eq!(reconcile_backend("llm-prod").unwrap(), ReconcileAction::Ok);
    }

    #[test]
    fn reconcile_route_returns_ok() {
        assert_eq!(reconcile_route("tenant-acme").unwrap(), ReconcileAction::Ok);
    }
}
