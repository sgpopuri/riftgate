//! Typed schema for the v0.1 config.
//!
//! Each field carries its `serde` deserialization metadata, a default
//! value (so partial TOML files work), and (in v0.2/v0.3 once hot
//! reload lands) a `#[reload = "safe" | "restart"]` annotation that the
//! diff-based reload code consumes.
//!
//! See [Options 015](../../../docs/05-options/015-config-model.md) and
//! [ADR 0012](../../../docs/06-adrs/0012-static-toml-env-override-v01.md).

use crate::secret::Secret;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;

/// Top-level configuration root.
///
/// Each section is `#[serde(default)]` so a partial TOML file (or no
/// file at all) deserialises cleanly using the per-section `Default`
/// impls below.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// HTTP server (where Riftgate listens for inbound traffic).
    #[serde(default)]
    pub server: ServerConfig,
    /// Default upstream backend.
    #[serde(default)]
    pub backend: BackendConfig,
    /// Per-shard timer subsystem tuning.
    #[serde(default)]
    pub timer: TimerConfig,
    /// Observability sinks and bus tuning.
    #[serde(default)]
    pub obs: ObsConfig,
    /// Log subscriber tuning.
    #[serde(default)]
    pub log: LogConfig,
    /// MCP capability broker configuration (v0.5,
    /// [ADR 0015](../../../docs/06-adrs/0015-mcp-extension-plane-broker.md)).
    #[serde(default)]
    pub mcp: McpConfig,
    /// Multitenancy and per-request tenant identity resolution (v1.0,
    /// [ADR 0029](../../../docs/06-adrs/0029-api-key-tenant-resolver.md)).
    #[serde(default)]
    pub multitenancy: MultitenancyConfig,
}

/// HTTP server configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Listen address and port.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: SocketAddr,
    /// Number of Tokio worker threads. `None` autodetects (number of
    /// logical cores).
    #[serde(default)]
    pub worker_threads: Option<usize>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            worker_threads: None,
        }
    }
}

fn default_listen_addr() -> SocketAddr {
    "127.0.0.1:8080".parse().expect("valid default socket addr")
}

/// Default upstream backend.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    /// Upstream URL (e.g. `https://api.openai.com`). Required for the
    /// proxy to do anything useful; the default is the empty string,
    /// which the validator rejects.
    #[serde(default)]
    pub url: String,
    /// Authorization header value to inject on upstream calls (e.g. the
    /// OpenAI API key as `Bearer sk-...`). Stored as `Secret<String>`
    /// so it cannot leak through any logging surface.
    #[serde(default)]
    pub auth_header: Secret<String>,
    /// Per-request upstream timeout, in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    /// Verify the upstream's TLS certificate against the platform trust
    /// store. Default `true`; set `false` only for local development.
    #[serde(default = "default_true")]
    pub tls_verify: bool,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            auth_header: Secret::default(),
            timeout_ms: default_timeout_ms(),
            tls_verify: true,
        }
    }
}

const fn default_timeout_ms() -> u32 {
    30_000
}
const fn default_true() -> bool {
    true
}

/// Per-shard timer subsystem tuning.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimerConfig {
    /// Tick resolution in milliseconds. Default 10 ms; configurable
    /// 5 ms / 100 ms per [`docs/04-design/lld-timers.md`](../../../docs/04-design/lld-timers.md).
    #[serde(default = "default_tick_resolution_ms")]
    pub tick_resolution_ms: u32,
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            tick_resolution_ms: default_tick_resolution_ms(),
        }
    }
}

const fn default_tick_resolution_ms() -> u32 {
    10
}

/// Observability sinks and bus tuning.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObsConfig {
    /// OTLP/gRPC endpoint for `OtelSink`. Default `http://127.0.0.1:4317`
    /// (OTel collector convention).
    #[serde(default = "default_otel_endpoint")]
    pub otel_endpoint: String,
    /// Sampling rate for per-token spans (0.0..=1.0). 0.01 = 1-in-100.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f32,
    /// Bounded MPSC bus capacity (events). Drop-on-full per [ADR 0011](../../../docs/06-adrs/0011-otel-default-sink-multisink-fanout.md).
    #[serde(default = "default_bus_capacity")]
    pub bus_capacity: usize,
}

impl Default for ObsConfig {
    fn default() -> Self {
        Self {
            otel_endpoint: default_otel_endpoint(),
            sample_rate: default_sample_rate(),
            bus_capacity: default_bus_capacity(),
        }
    }
}

fn default_otel_endpoint() -> String {
    "http://127.0.0.1:4317".to_string()
}
const fn default_sample_rate() -> f32 {
    0.01
}
const fn default_bus_capacity() -> usize {
    4096
}

/// Log subscriber configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    /// Log level (`error`, `warn`, `info`, `debug`, `trace`). Validated
    /// at startup.
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Log format.
    #[serde(default)]
    pub format: LogFormat,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: LogFormat::default(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Log output format.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Structured JSON one-line-per-event. Default for production.
    #[default]
    Json,
    /// Pretty multi-line for human reading. Default for `--dev` mode.
    Pretty,
}

// ---------------------------------------------------------------------------
// MCP capability broker config (v0.5)
// ---------------------------------------------------------------------------

/// MCP capability broker configuration.
///
/// Controls whether MCP (`tools/call`, `resources/read`, etc.) requests are
/// authorized against a per-tenant allowlist before forwarding. Configured
/// under `[mcp]` in the gateway TOML file.
///
/// ```toml
/// [mcp]
/// enforce = true                  # false = dry-run (log denials, always pass)
///
/// [mcp.tenants."1"]               # numeric tenant id as string key
/// allowed_tools             = ["search-web", "read-file"]
/// denied_tools              = ["filesystem-write"]
/// allowed_resource_prefixes = ["s3://acme-datasets/*"]
/// time_bounded_grants = [
///   { tool = "send-email", until_unix_secs = 1780000000 },
/// ]
/// ```
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct McpConfig {
    /// When `true` (default), deny decisions return HTTP `403`.
    /// When `false`, a `DryRunBroker` is used: denials are logged but every
    /// request passes. Use this to calibrate allowlists before enforcing.
    #[serde(default = "default_true")]
    pub enforce: bool,
    /// Per-tenant allowlists, keyed by numeric tenant ID as a string
    /// (e.g. `"1"` maps to `TenantId(1)`). An empty map disables the broker.
    #[serde(default)]
    pub tenants: std::collections::HashMap<String, McpTenantConfig>,
    /// Hex-encoded 32-byte HMAC signing key for attestation headers.
    ///
    /// If absent, a random ephemeral key is generated at startup (not
    /// persistent across restarts). Pin this value in production.
    #[serde(default)]
    pub signing_key_hex: Option<String>,
    /// Directory path for the MCP audit WAL.
    ///
    /// When set, every `authorize()` decision is appended to a durable
    /// `FileWal` under this path. When absent, audit records are computed
    /// but not persisted (development/test mode).
    #[serde(default)]
    pub wal_path: Option<String>,
}

/// Per-tenant MCP allowlist configuration.
///
/// Mirrors `riftgate_mcp::TenantAllowlist` exactly; duplicated here so
/// `riftgate-config` stays independent of `riftgate-mcp`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct McpTenantConfig {
    /// Tools this tenant may invoke.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied (takes precedence over `allowed_tools`).
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// Resource URI prefixes this tenant may read. Trailing `*` is a prefix
    /// glob (e.g. `"s3://acme-datasets/*"`).
    #[serde(default)]
    pub allowed_resource_prefixes: Vec<String>,
    /// Temporary tool grants that expire at a given Unix timestamp.
    #[serde(default)]
    pub time_bounded_grants: Vec<McpTimeBoundedGrant>,
}

/// A temporary tool grant active until `until_unix_secs`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpTimeBoundedGrant {
    /// Tool name this grant covers.
    pub tool: String,
    /// Grant expires when `now >= UNIX_EPOCH + until_unix_secs`.
    pub until_unix_secs: u64,
}

// ---------------------------------------------------------------------------
// Multitenancy config (v1.0, ADR 0029)
// ---------------------------------------------------------------------------

/// Multitenancy and per-request tenant identity resolution.
///
/// ```toml
/// [multitenancy]
/// mode = "api-key"        # "api-key" (default) or "trusted-header"
///
/// # API keys stored as SHA-256 hex of the raw Bearer token.
/// # Value is the tenant name (resolved to a TenantId via FNV-1a or numeric parse).
/// [multitenancy.api_keys]
/// "sha256:<64 hex chars>" = "acme"
/// "sha256:<64 hex chars>" = "bigcorp"
/// ```
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct MultitenancyConfig {
    /// Resolution mode: `"api-key"` (default) or `"trusted-header"`.
    ///
    /// - `"api-key"`: reads `Authorization: Bearer <key>`, computes SHA-256,
    ///   looks up in `api_keys`. Recommended for internet-facing deployments.
    /// - `"trusted-header"`: reads `x-riftgate-tenant` directly. Only safe
    ///   on trusted internal networks (service mesh, local dev).
    #[serde(default = "default_multitenancy_mode")]
    pub mode: String,
    /// API key registry: maps `"sha256:<hex>"` to tenant name.
    /// Only used when `mode = "api-key"`.
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
}

fn default_multitenancy_mode() -> String {
    "api-key".to_owned()
}

/// API key resolver: maps `Authorization: Bearer <key>` to a `TenantIdentity`.
///
/// Keys are stored as `"sha256:<64 hex chars>"` in config; the gateway
/// computes SHA-256 of the incoming bearer token and performs an O(1) lookup.
pub struct ApiKeyTenantResolver {
    /// Pre-built map: SHA-256 hex of raw key -> (TenantId, principal string).
    registry: HashMap<String, (riftgate_core::types::TenantId, String)>,
}

impl ApiKeyTenantResolver {
    /// Construct from the `[multitenancy.api_keys]` config table.
    ///
    /// Keys in config are expected to be `"sha256:<64 hex chars>"`.
    /// Values are tenant names (resolved to `TenantId` via FNV-1a or numeric parse).
    pub fn from_config(cfg: &MultitenancyConfig) -> Self {
        use riftgate_core::tenant::tenant_id_from_str;
        use riftgate_core::types::TenantId;
        let registry = cfg
            .api_keys
            .iter()
            .map(|(k, name)| {
                let id = TenantId(tenant_id_from_str(name));
                (k.clone(), (id, name.clone()))
            })
            .collect();
        Self { registry }
    }

    /// SHA-256 hex of `raw_key` in the form `"sha256:<hex>"`.
    fn hash_key(raw_key: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(raw_key.as_bytes());
        let hex: String = digest.iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        });
        format!("sha256:{hex}")
    }
}

impl riftgate_core::tenant::TenantResolver for ApiKeyTenantResolver {
    fn resolve(
        &self,
        headers: &http::HeaderMap,
    ) -> Option<riftgate_core::capability::TenantIdentity> {
        let auth = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
        let raw_key = auth.strip_prefix("Bearer ")?;
        let hashed = Self::hash_key(raw_key);
        let (id, principal) = self.registry.get(&hashed)?;
        Some(riftgate_core::capability::TenantIdentity {
            tenant: *id,
            principal: principal.clone(),
        })
    }
}

#[cfg(test)]
mod tenant_resolver_tests {
    use super::*;
    use http::{HeaderMap, HeaderValue};
    use riftgate_core::tenant::TenantResolver;

    fn make_cfg(key_hex: &str, tenant_name: &str) -> MultitenancyConfig {
        let mut cfg = MultitenancyConfig::default();
        cfg.api_keys
            .insert(format!("sha256:{key_hex}"), tenant_name.to_owned());
        cfg
    }

    #[test]
    fn api_key_resolver_accepts_valid_key() {
        // Build a resolver with a known key.
        let raw_key = "test-api-key-acme";
        let expected_hex = {
            use sha2::{Digest, Sha256};
            let d = Sha256::digest(raw_key.as_bytes());
            d.iter().fold(String::new(), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            })
        };
        let cfg = make_cfg(&expected_hex, "acme");
        let resolver = ApiKeyTenantResolver::from_config(&cfg);

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {raw_key}")).unwrap(),
        );
        let identity = resolver.resolve(&headers).expect("should resolve");
        assert_eq!(identity.principal, "acme");
    }

    #[test]
    fn api_key_resolver_rejects_unknown_key() {
        let cfg = MultitenancyConfig::default();
        let resolver = ApiKeyTenantResolver::from_config(&cfg);
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer unknown-key"),
        );
        assert!(resolver.resolve(&headers).is_none());
    }

    #[test]
    fn api_key_resolver_rejects_missing_auth_header() {
        let cfg = MultitenancyConfig::default();
        let resolver = ApiKeyTenantResolver::from_config(&cfg);
        assert!(resolver.resolve(&HeaderMap::new()).is_none());
    }
}
