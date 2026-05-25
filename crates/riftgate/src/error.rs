//! Binary-local error type.
//!
//! Distinct from `riftgate_core::error::RiftgateCoreError` because the
//! binary aggregates errors from multiple crates (config, hyper, OTel
//! SDK, signal handling) plus its own bootstrap failures. We keep the
//! kernel error free of these dependencies.

use std::io;
use thiserror::Error;

/// Binary-local error type.
#[derive(Debug, Error)]
pub enum RiftgateError {
    /// Configuration loading or validation failed. Each variant carries
    /// the human-readable rendering produced by `riftgate-config`.
    #[error("configuration error: {0}")]
    Config(String),

    /// IO error during bind / accept / signal handling. Almost always
    /// fatal at startup; recoverable per-connection at runtime.
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// OpenTelemetry SDK setup failure. Logged at startup; we do NOT
    /// abort on this so a misconfigured collector does not take down
    /// the gateway.
    #[error("opentelemetry setup failed: {0}")]
    OpenTelemetry(String),

    /// Hyper-side error during a server connection. Surfaced into the
    /// per-connection task; the accept loop logs and continues.
    #[error("hyper error: {0}")]
    Hyper(String),
}

impl From<hyper::Error> for RiftgateError {
    fn from(e: hyper::Error) -> Self {
        Self::Hyper(e.to_string())
    }
}

impl From<Vec<riftgate_config::ConfigError>> for RiftgateError {
    fn from(errors: Vec<riftgate_config::ConfigError>) -> Self {
        let body = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n  - ");
        Self::Config(format!("the configuration was rejected:\n  - {body}"))
    }
}
