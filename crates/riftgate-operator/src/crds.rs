//! CRD type definitions for `riftgate.io/v1alpha1`.
//!
//! Each spec struct is a plain `serde` struct so it compiles without the
//! `operator` feature. When `operator` is enabled, the `CustomResource` derive
//! macro from `kube` generates the full Kubernetes resource wrapper type
//! (e.g. `Riftgate` from `RiftgateSpec`).
//!
//! API group: `riftgate.io`  
//! Version:   `v1alpha1` (no stability guarantee until promoted to `v1`)

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Shared sub-types
// ---------------------------------------------------------------------------

/// Reference to a Kubernetes Secret key (namespace is always the CR's namespace).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretKeyRef {
    /// Name of the Secret.
    pub name: String,
    /// Key within the Secret's `data` map.
    pub key: String,
}

/// Reference to a Kubernetes Service (in the same namespace as the CR).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRef {
    /// Service name.
    pub name: String,
    /// Port number.
    pub port: u16,
}

/// Per-backend circuit-breaker tuning.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CircuitBreakerConfig {
    /// Error rate threshold (0.0–1.0) that trips the breaker. Default `0.5`.
    #[serde(default = "default_error_threshold")]
    pub error_threshold: f32,
    /// Probe interval in milliseconds when in half-open state. Default `5000`.
    #[serde(default = "default_probe_interval_ms")]
    pub probe_interval_ms: u32,
}

fn default_error_threshold() -> f32 {
    0.5
}
fn default_probe_interval_ms() -> u32 {
    5000
}

// ---------------------------------------------------------------------------
// Riftgate CRD — gateway instance
// ---------------------------------------------------------------------------

/// Spec for the `Riftgate` custom resource.
///
/// A `Riftgate` object describes one running gateway instance. The operator
/// reconciles it into a Kubernetes `Deployment` + `ConfigMap`.
///
/// ```yaml
/// apiVersion: riftgate.io/v1alpha1
/// kind: Riftgate
/// metadata:
///   name: prod-gateway
/// spec:
///   image: "ghcr.io/sgpopuri/riftgate:v1.0.0"
///   listenAddr: "0.0.0.0:8080"
///   drainGraceMs: 30000
///   obsEndpoint: "http://otel-collector:4317"
///   replicas: 2
/// ```
#[cfg_attr(
    feature = "operator",
    derive(kube::CustomResource),
    kube(
        group = "riftgate.io",
        version = "v1alpha1",
        kind = "Riftgate",
        namespaced,
        status = "RiftgateStatus",
        shortname = "rg"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RiftgateSpec {
    /// Container image for the Riftgate binary.
    /// Example: `"ghcr.io/sgpopuri/riftgate:v1.0.0"`
    pub image: String,
    /// TCP listen address inside the container. Default `"0.0.0.0:8080"`.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    /// SIGTERM drain grace window in milliseconds. Default `30000`.
    #[serde(default = "default_drain_grace_ms")]
    pub drain_grace_ms: u64,
    /// OTLP/gRPC endpoint for the OTel sink. Default `"http://127.0.0.1:4317"`.
    #[serde(default = "default_obs_endpoint")]
    pub obs_endpoint: String,
    /// Number of gateway replicas. Default `1`.
    #[serde(default = "default_replicas")]
    pub replicas: i32,
    /// Log level (`error`, `warn`, `info`, `debug`, `trace`). Default `"info"`.
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_listen_addr() -> String {
    "0.0.0.0:8080".to_owned()
}
fn default_drain_grace_ms() -> u64 {
    30_000
}
fn default_obs_endpoint() -> String {
    "http://127.0.0.1:4317".to_owned()
}
fn default_replicas() -> i32 {
    1
}
fn default_log_level() -> String {
    "info".to_owned()
}

/// Status written back to the `Riftgate` object by the operator.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RiftgateStatus {
    /// Number of ready replicas observed by the operator.
    pub ready_replicas: Option<i32>,
    /// Human-readable reconciliation message.
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// RiftgateBackend CRD — upstream backend
// ---------------------------------------------------------------------------

/// Spec for the `RiftgateBackend` custom resource.
///
/// Describes one upstream backend (LLM inference server, vllm pod, etc.).
///
/// ```yaml
/// apiVersion: riftgate.io/v1alpha1
/// kind: RiftgateBackend
/// metadata:
///   name: llm-prod
/// spec:
///   serviceRef:
///     name: vllm-service
///     port: 8000
///   authSecretRef:
///     name: llm-api-key
///     key: value
///   timeoutMs: 30000
/// ```
#[cfg_attr(
    feature = "operator",
    derive(kube::CustomResource),
    kube(
        group = "riftgate.io",
        version = "v1alpha1",
        kind = "RiftgateBackend",
        namespaced,
        shortname = "rgb"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RiftgateBackendSpec {
    /// Static upstream URL (mutually exclusive with `serviceRef`).
    /// Example: `"https://api.openai.com"`
    #[serde(default)]
    pub url: Option<String>,
    /// Kubernetes Service reference (mutually exclusive with `url`).
    /// The operator watches `Endpoints` to keep the URL current.
    #[serde(default)]
    pub service_ref: Option<ServiceRef>,
    /// Reference to a Secret containing the `Authorization` header value.
    #[serde(default)]
    pub auth_secret_ref: Option<SecretKeyRef>,
    /// Per-request upstream timeout in milliseconds. Default `30000`.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    /// Circuit-breaker configuration.
    #[serde(default)]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
}

fn default_timeout_ms() -> u32 {
    30_000
}

// ---------------------------------------------------------------------------
// RiftgateRoute CRD — routing rule
// ---------------------------------------------------------------------------

/// MCP allowlist config embedded in a `RiftgateRoute`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpRouteConfig {
    /// Enforce denials (`true`) or dry-run (`false`). Default `true`.
    #[serde(default = "default_true")]
    pub enforce: bool,
    /// Tools this route permits.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied.
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// Resource URI prefix globs permitted.
    #[serde(default)]
    pub allowed_resource_prefixes: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Multitenancy config embedded in a `RiftgateRoute`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MultitenancyRouteConfig {
    /// Reference to a Secret containing API keys (one key per line: `sha256:<hex>=<tenant>`).
    #[serde(default)]
    pub api_key_secret_ref: Option<SecretKeyRef>,
}

/// Spec for the `RiftgateRoute` custom resource.
///
/// A routing rule: path prefix → backend, optionally with MCP policy and
/// per-tenant API key authentication.
///
/// ```yaml
/// apiVersion: riftgate.io/v1alpha1
/// kind: RiftgateRoute
/// metadata:
///   name: tenant-acme
/// spec:
///   pathPrefix: "/v1/"
///   backendRef: llm-prod
///   weight: 100
///   mcp:
///     enforce: true
///     allowedTools: ["search-web"]
///   multitenancy:
///     apiKeySecretRef:
///       name: acme-api-key
///       key: keys
/// ```
#[cfg_attr(
    feature = "operator",
    derive(kube::CustomResource),
    kube(
        group = "riftgate.io",
        version = "v1alpha1",
        kind = "RiftgateRoute",
        namespaced,
        shortname = "rgr"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RiftgateRouteSpec {
    /// Path prefix that this rule matches (e.g. `"/v1/"`).
    pub path_prefix: String,
    /// Name of the `RiftgateBackend` this rule sends traffic to.
    pub backend_ref: String,
    /// Routing weight (for weighted-random selection). Default `100`.
    #[serde(default = "default_weight")]
    pub weight: u32,
    /// MCP capability policy for this route.
    #[serde(default)]
    pub mcp: Option<McpRouteConfig>,
    /// Per-tenant API key authentication for this route.
    #[serde(default)]
    pub multitenancy: Option<MultitenancyRouteConfig>,
}

fn default_weight() -> u32 {
    100
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riftgate_spec_defaults_are_sane() {
        let spec: RiftgateSpec =
            serde_json::from_str(r#"{"image":"ghcr.io/sgpopuri/riftgate:v1.0.0"}"#).unwrap();
        assert_eq!(spec.listen_addr, "0.0.0.0:8080");
        assert_eq!(spec.drain_grace_ms, 30_000);
        assert_eq!(spec.replicas, 1);
        assert_eq!(spec.log_level, "info");
    }

    #[test]
    fn riftgate_backend_spec_roundtrips() {
        let spec = RiftgateBackendSpec {
            service_ref: Some(ServiceRef {
                name: "vllm-svc".to_owned(),
                port: 8000,
            }),
            timeout_ms: 15_000,
            ..Default::default()
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: RiftgateBackendSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timeout_ms, 15_000);
        assert!(back.service_ref.is_some());
    }

    #[test]
    fn riftgate_route_spec_defaults() {
        let spec: RiftgateRouteSpec =
            serde_json::from_str(r#"{"pathPrefix":"/v1/","backendRef":"llm-prod"}"#).unwrap();
        assert_eq!(spec.weight, 100);
        assert!(spec.mcp.is_none());
    }
}
