//! `validate(&Config) -> Result<(), Vec<ConfigError>>`.
//!
//! Validation runs against the *effective* (merged) config, after
//! defaults → file → env have been applied. Errors are accumulated;
//! every violation is reported.
//!
//! See [Options 015 §6](../../../docs/05-options/015-config-model.md)
//! for the rationale.

use crate::error::{ConfigError, SourceLayer};
use crate::schema::Config;

/// Validate a fully-merged `Config`.
///
/// Returns `Ok(())` if every field passes its constraints, or
/// `Err(Vec<ConfigError>)` listing every violation. The binary
/// surfaces every violation before exiting with status 78.
///
/// # Errors
/// Returns one `ConfigError::Validation` variant per violated
/// constraint.
pub fn validate(cfg: &Config) -> Result<(), Vec<ConfigError>> {
    let mut errors: Vec<ConfigError> = Vec::new();

    // Backend URL is required (default is empty string, which fails).
    if cfg.backend.url.is_empty() {
        errors.push(ConfigError::Validation {
            path: "backend.url".to_string(),
            expected: "a non-empty URL",
            got: "(empty)".to_string(),
            layer: SourceLayer::Defaults,
        });
    } else if !(cfg.backend.url.starts_with("http://") || cfg.backend.url.starts_with("https://")) {
        errors.push(ConfigError::Validation {
            path: "backend.url".to_string(),
            expected: "a URL starting with http:// or https://",
            got: cfg.backend.url.clone(),
            layer: SourceLayer::File,
        });
    }

    if cfg.backend.timeout_ms == 0 {
        errors.push(ConfigError::Validation {
            path: "backend.timeout_ms".to_string(),
            expected: "a positive integer (ms)",
            got: "0".to_string(),
            layer: SourceLayer::File,
        });
    }

    if let Some(0) = cfg.server.worker_threads {
        errors.push(ConfigError::Validation {
            path: "server.worker_threads".to_string(),
            expected: "a positive integer or omitted (autodetect)",
            got: "0".to_string(),
            layer: SourceLayer::File,
        });
    }

    if cfg.timer.tick_resolution_ms == 0 {
        errors.push(ConfigError::Validation {
            path: "timer.tick_resolution_ms".to_string(),
            expected: "a positive integer (ms)",
            got: "0".to_string(),
            layer: SourceLayer::File,
        });
    }

    if !(0.0..=1.0).contains(&cfg.obs.sample_rate) {
        errors.push(ConfigError::Validation {
            path: "obs.sample_rate".to_string(),
            expected: "a real number in [0.0, 1.0]",
            got: cfg.obs.sample_rate.to_string(),
            layer: SourceLayer::File,
        });
    }

    if cfg.obs.bus_capacity == 0 {
        errors.push(ConfigError::Validation {
            path: "obs.bus_capacity".to_string(),
            expected: "a positive integer",
            got: "0".to_string(),
            layer: SourceLayer::File,
        });
    }

    let level_lc = cfg.log.level.to_lowercase();
    if !matches!(
        level_lc.as_str(),
        "trace" | "debug" | "info" | "warn" | "error" | "off"
    ) {
        errors.push(ConfigError::Validation {
            path: "log.level".to_string(),
            expected: "one of: trace, debug, info, warn, error, off",
            got: cfg.log.level.clone(),
            layer: SourceLayer::File,
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
