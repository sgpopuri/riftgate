//! `ConfigError` — typed errors surfaced by the loader and validator.
//!
//! Every variant carries enough context for the operator to jump
//! directly to the offending key. The error is `Display`-friendly for
//! the binary's startup error message and `Debug`-friendly for tests.

use std::path::PathBuf;
use thiserror::Error;

/// Source layer that produced a value (or attempted to).
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SourceLayer {
    /// Built-in defaults baked into the schema.
    Defaults,
    /// File on disk.
    File,
    /// Environment variable.
    Env,
}

impl core::fmt::Display for SourceLayer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Defaults => write!(f, "defaults"),
            Self::File => write!(f, "file"),
            Self::Env => write!(f, "env"),
        }
    }
}

/// Configuration loading or validation error.
///
/// Returned by [`crate::load`] and [`crate::validate()`]. Errors are
/// accumulated where possible (the binary surfaces every violation, not
/// just the first) so operators can fix multiple issues per
/// edit-validate cycle.
// thiserror treats a field literally named `source` as a cause-chain
// source that must implement `std::error::Error`. The fields below are
// already-rendered error strings (from `std::io::Error::to_string()`,
// `toml::de::Error::to_string()`, etc.), not nested errors, so we name
// them `message` and `layer` instead.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file could not be read from disk.
    #[error("could not read config file `{path}`: {message}")]
    FileRead {
        /// Path that was attempted.
        path: PathBuf,
        /// IO error message.
        message: String,
    },

    /// The config file did not parse as TOML.
    #[error("invalid TOML in `{path}`: {message}")]
    TomlParse {
        /// Path of the file that failed to parse.
        path: PathBuf,
        /// `toml::de::Error` message.
        message: String,
    },

    /// An environment variable contained a value the schema could not
    /// accept (e.g. `RIFTGATE_SERVER_LISTEN_ADDR=garbage`).
    #[error("env var `{key}` is not a valid {expected}: got `{got}`")]
    EnvParse {
        /// The full env var name.
        key: String,
        /// What the schema expected (e.g. `"socket address"`).
        expected: &'static str,
        /// The literal value that was rejected (redacted if the field
        /// is a secret).
        got: String,
    },

    /// A typed value did not pass schema validation.
    #[error("config validation: `{path}` must be {expected}, got `{got}` (from {layer})")]
    Validation {
        /// Dot-path to the field (e.g. `"backend.timeout_ms"`).
        path: String,
        /// Human description of the constraint (e.g. `"a positive integer"`).
        expected: &'static str,
        /// Observed value (redacted if the field is a secret).
        got: String,
        /// Source layer that produced the value.
        layer: SourceLayer,
    },
}
