//! Reconciler stubs for `Riftgate`, `RiftgateBackend`, and `RiftgateRoute`.
//!
//! Without the `operator` feature these are no-op stubs so the crate
//! compiles in a plain `cargo check --workspace`. With the feature enabled,
//! the full `kube-runtime` controller loop is wired.

/// Placeholder reconciliation outcome.
///
/// In the live controller (`operator` feature) this maps to `kube_runtime::controller::Action`.
/// In stub mode it is a plain enum so callers can handle both paths uniformly.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReconcileAction {
    /// Reconciliation succeeded; requeue after the default interval.
    Ok,
    /// Reconciliation encountered a transient error; requeue after `backoff_secs`.
    Requeue {
        /// Seconds before the next attempt.
        backoff_secs: u64,
    },
}

/// Error type for reconcile failures.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    /// A required CRD field was missing or invalid.
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
    /// Kubernetes API call failed.
    #[error("API error: {0}")]
    ApiError(String),
    /// TOML config rendering failed.
    #[error("config render error: {0}")]
    RenderError(String),
}

/// Stub reconciler called by the controller loop.
///
/// In stub mode: logs the event and returns `Ok`.
/// In `operator` mode: reads the gateway spec, resolves backends and routes,
/// renders the TOML config, and patches the `ConfigMap` + `Deployment`.
///
/// `name` is the name of the `Riftgate` CR being reconciled.
pub fn reconcile_riftgate(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(gateway = name, "reconciling Riftgate");
    // TODO(v1.0): fetch Riftgate, RiftgateBackend, and RiftgateRoute objects
    // via the Kubernetes API; call render::render_config; patch ConfigMap
    // and Deployment; update status sub-resource.
    Ok(ReconcileAction::Ok)
}

/// Stub reconciler for `RiftgateBackend`.
pub fn reconcile_backend(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(backend = name, "reconciling RiftgateBackend");
    // TODO(v1.0): if serviceRef is set, watch Endpoints and patch the owning
    // Riftgate's ConfigMap when pod IPs change.
    Ok(ReconcileAction::Ok)
}

/// Stub reconciler for `RiftgateRoute`.
pub fn reconcile_route(name: &str) -> Result<ReconcileAction, ReconcileError> {
    tracing::info!(route = name, "reconciling RiftgateRoute");
    // TODO(v1.0): validate route spec, resolve the API key Secret if
    // multitenancy.apiKeySecretRef is set, regenerate the Riftgate ConfigMap.
    Ok(ReconcileAction::Ok)
}

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
