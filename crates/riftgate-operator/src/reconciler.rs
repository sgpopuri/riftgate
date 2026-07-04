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
    use crate::crds::{
        Riftgate, RiftgateBackend, RiftgateBackendSpec, RiftgateRoute, RiftgateRouteSpec,
    };
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

        // 1. List all RiftgateBackend objects in the namespace.
        let backends_api: Api<RiftgateBackend> = Api::namespaced(ctx.client.clone(), &ns);
        let backend_list = backends_api
            .list(&kube::api::ListParams::default())
            .await
            .map_err(|e| ReconcileError::ApiError(format!("list backends: {e}")))?;

        // 2. List all RiftgateRoute objects in the namespace.
        let routes_api: Api<RiftgateRoute> = Api::namespaced(ctx.client.clone(), &ns);
        let route_list = routes_api
            .list(&kube::api::ListParams::default())
            .await
            .map_err(|e| ReconcileError::ApiError(format!("list routes: {e}")))?;

        // 3. Render the TOML config from all three CRD objects.
        let backends: Vec<(String, &RiftgateBackendSpec)> = backend_list
            .items
            .iter()
            .map(|b| (b.name_any(), &b.spec))
            .collect();
        let routes: Vec<(String, &RiftgateRouteSpec)> = route_list
            .items
            .iter()
            .map(|r| (r.name_any(), &r.spec))
            .collect();

        let backend_refs: Vec<(&str, &RiftgateBackendSpec)> =
            backends.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let route_refs: Vec<(&str, &RiftgateRouteSpec)> =
            routes.iter().map(|(n, s)| (n.as_str(), *s)).collect();

        let toml_config = render::render_config(&obj.spec, &backend_refs, &route_refs);
        tracing::debug!(gateway = %name, bytes = toml_config.len(), "rendered config");

        // 4. Patch the ConfigMap via server-side apply.
        let cm_name = format!("{name}-config");
        let cm_api: Api<k8s_openapi::api::core::v1::ConfigMap> =
            Api::namespaced(ctx.client.clone(), &ns);
        let cm_patch = serde_json::json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": cm_name, "namespace": ns },
            "data": { "riftgate.toml": toml_config }
        });
        cm_api
            .patch(
                &cm_name,
                &kube::api::PatchParams::apply("riftgate-operator"),
                &kube::api::Patch::Apply(&cm_patch),
            )
            .await
            .map_err(|e| ReconcileError::ApiError(format!("patch ConfigMap: {e}")))?;
        tracing::info!(gateway = %name, configmap = %cm_name, "ConfigMap patched");

        // 5. Patch the Deployment via server-side apply.
        let deploy_api: Api<k8s_openapi::api::apps::v1::Deployment> =
            Api::namespaced(ctx.client.clone(), &ns);
        let deploy_patch = serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": name,
                "namespace": ns,
                "labels": { "app.kubernetes.io/managed-by": "riftgate-operator" }
            },
            "spec": {
                "replicas": obj.spec.replicas,
                "selector": { "matchLabels": { "app": name } },
                "template": {
                    "metadata": { "labels": { "app": name } },
                    "spec": {
                        "containers": [{
                            "name": "riftgate",
                            "image": obj.spec.image,
                            "args": ["--config", "/etc/riftgate/riftgate.toml"],
                            "ports": [{ "containerPort": 8080, "name": "http" }],
                            "volumeMounts": [{
                                "name": "config",
                                "mountPath": "/etc/riftgate",
                                "readOnly": true
                            }]
                        }],
                        "volumes": [{
                            "name": "config",
                            "configMap": { "name": cm_name }
                        }]
                    }
                }
            }
        });
        deploy_api
            .patch(
                &name,
                &kube::api::PatchParams::apply("riftgate-operator"),
                &kube::api::Patch::Apply(&deploy_patch),
            )
            .await
            .map_err(|e| ReconcileError::ApiError(format!("patch Deployment: {e}")))?;
        tracing::info!(gateway = %name, deployment = %name, "Deployment patched");

        // 6. Update the Riftgate status subresource.
        let status_patch = serde_json::json!({
            "status": { "message": "reconciled" }
        });
        cm_api
            .patch_status(
                &cm_name,
                &kube::api::PatchParams::apply("riftgate-operator"),
                &kube::api::Patch::Merge(&status_patch),
            )
            .await
            .ok(); // best-effort; ignore errors

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
