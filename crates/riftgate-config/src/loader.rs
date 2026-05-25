//! Pure-function loader: `load(path, env) -> Result<Config>`.
//!
//! Layered merge: defaults → file → env. Each layer is applied in turn;
//! validation runs against the *effective* (merged) config (in
//! [`crate::validate()`]).
//!
//! The loader is **re-runnable**: it is a pure function of `(path,
//! env)`, with no global state. The v0.2/v0.3 hot-reload path consumes
//! it unchanged.

use crate::error::ConfigError;
use crate::schema::Config;
use crate::secret::Secret;
use crate::validate::validate;
use std::collections::HashMap;
use std::path::Path;

/// Source of environment variables for the loader.
///
/// Tests can construct an in-memory `Env` to exercise the loader
/// without mutating the process's `std::env`. The binary builds an
/// `Env` from `std::env::vars()` at startup.
#[derive(Debug, Default, Clone)]
pub struct Env {
    inner: HashMap<String, String>,
}

impl Env {
    /// Construct an empty `Env`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current process's environment variables.
    pub fn from_process() -> Self {
        Self {
            inner: std::env::vars().collect(),
        }
    }

    /// Insert a single env var. Useful for tests.
    pub fn with(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.inner.insert(k.into(), v.into());
        self
    }

    /// Borrow the inner map.
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.inner
    }
}

/// Load and validate a configuration.
///
/// - `path`: path to the TOML config file. May be `None` for
///   defaults-plus-env-only loads.
/// - `env`: the environment-variable source.
///
/// Returns the validated [`Config`] on success or an aggregated
/// `Vec<ConfigError>` on failure (every violation is reported, not just
/// the first).
///
/// # Errors
/// Returns one or more `ConfigError`s for any of: file missing /
/// unreadable, invalid TOML, unparseable env var value, or schema
/// validation failure.
pub fn load(path: Option<&Path>, env: &Env) -> Result<Config, Vec<ConfigError>> {
    let mut errors: Vec<ConfigError> = Vec::new();

    // Layer 1+2: defaults + file (or just defaults).
    let mut config: Config = if let Some(path) = path {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(parsed) => parsed,
                Err(e) => {
                    errors.push(ConfigError::TomlParse {
                        path: path.to_path_buf(),
                        message: e.to_string(),
                    });
                    return Err(errors);
                }
            },
            Err(e) => {
                errors.push(ConfigError::FileRead {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                });
                return Err(errors);
            }
        }
    } else {
        Config::default()
    };

    // Layer 3: env overrides. Apply each known key. Unrecognised
    // RIFTGATE_* env vars are warned about but not fatal.
    apply_env_overrides(&mut config, env, &mut errors);

    // Validate effective config.
    if let Err(mut validation_errors) = validate(&config) {
        errors.append(&mut validation_errors);
    }

    if errors.is_empty() {
        Ok(config)
    } else {
        Err(errors)
    }
}

/// Apply `RIFTGATE_<SECTION>_<KEY>` env-var overrides to `config`.
///
/// Unrecognised keys (`RIFTGATE_*` that does not map to any known
/// field) emit a `tracing::warn!` line — most likely a typo, per [ADR
/// 0012](../../../docs/06-adrs/0012-static-toml-env-override-v01.md).
fn apply_env_overrides(config: &mut Config, env: &Env, errors: &mut Vec<ConfigError>) {
    let recognised_prefixes = [
        "RIFTGATE_SERVER_",
        "RIFTGATE_BACKEND_",
        "RIFTGATE_TIMER_",
        "RIFTGATE_OBS_",
        "RIFTGATE_LOG_",
    ];

    for (k, v) in env.as_map() {
        if !k.starts_with("RIFTGATE_") {
            continue;
        }
        match k.as_str() {
            "RIFTGATE_SERVER_LISTEN_ADDR" => match v.parse() {
                Ok(addr) => config.server.listen_addr = addr,
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "socket address (e.g. localhost:8080)",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_SERVER_WORKER_THREADS" => match v.parse() {
                Ok(n) => config.server.worker_threads = Some(n),
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "positive integer",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_BACKEND_URL" => config.backend.url = v.clone(),
            "RIFTGATE_BACKEND_AUTH_HEADER" => {
                config.backend.auth_header = Secret::new(v.clone());
            }
            "RIFTGATE_BACKEND_TIMEOUT_MS" => match v.parse() {
                Ok(n) => config.backend.timeout_ms = n,
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "positive integer (ms)",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_BACKEND_TLS_VERIFY" => match parse_bool(v) {
                Some(b) => config.backend.tls_verify = b,
                None => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "boolean (true/false/1/0)",
                    got: v.clone(),
                }),
            },
            "RIFTGATE_TIMER_TICK_RESOLUTION_MS" => match v.parse() {
                Ok(n) => config.timer.tick_resolution_ms = n,
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "positive integer (ms)",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_OBS_OTEL_ENDPOINT" => config.obs.otel_endpoint = v.clone(),
            "RIFTGATE_OBS_SAMPLE_RATE" => match v.parse() {
                Ok(n) => config.obs.sample_rate = n,
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "f32 in [0.0, 1.0]",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_OBS_BUS_CAPACITY" => match v.parse() {
                Ok(n) => config.obs.bus_capacity = n,
                Err(e) => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "positive integer",
                    got: format!("{v} ({e})"),
                }),
            },
            "RIFTGATE_LOG_LEVEL" => config.log.level = v.clone(),
            "RIFTGATE_LOG_FORMAT" => match v.to_lowercase().as_str() {
                "json" => config.log.format = crate::schema::LogFormat::Json,
                "pretty" => config.log.format = crate::schema::LogFormat::Pretty,
                _ => errors.push(ConfigError::EnvParse {
                    key: k.clone(),
                    expected: "log format: \"json\" or \"pretty\"",
                    got: v.clone(),
                }),
            },
            other => {
                if recognised_prefixes
                    .iter()
                    .any(|prefix| other.starts_with(prefix))
                {
                    tracing::warn!(env = %other, "unrecognised RIFTGATE_* env var (probable typo)");
                }
            }
        }
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
