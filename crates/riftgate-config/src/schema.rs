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
