//! The shared error type for the kernel.
//!
//! Subsystem-specific errors (parser, timers, ...) live in their own module
//! and convert into this type via `From` where the wrapping is informative.

use thiserror::Error;

/// Top-level error type for kernel operations.
///
/// Subsystem modules expose their own error types ([`crate::parser::ParseError`],
/// for example) and convert into this enum at the boundary where a caller does
/// not care which subsystem failed.
#[derive(Debug, Error)]
pub enum RiftgateCoreError {
    /// An IO operation failed at the kernel boundary.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A bounded buffer rejected an item because it was full. Carries the
    /// human-readable name of the resource for logging.
    #[error("{resource} is full")]
    Full {
        /// Human-readable name of the resource that was full
        /// (e.g. `"observability bus"`, `"per-shard request queue"`).
        resource: &'static str,
    },

    /// A timer handle referenced a timer that no longer exists (already
    /// fired, already cancelled, or never scheduled). Idempotent cancellation
    /// is the trait contract for [`crate::timers::TimerSubsystem::cancel`].
    #[error("timer not found")]
    TimerNotFound,

    /// The parser surfaced a structural error. The variant is preserved
    /// inside via [`crate::parser::ParseError`].
    #[error("parse: {0}")]
    Parse(#[from] crate::parser::ParseError),

    /// A subsystem reported an unexpected condition that did not fit any of
    /// the typed variants. The wrapped string is for human consumption only;
    /// callers should not pattern-match on it.
    #[error("internal: {0}")]
    Internal(String),
}

impl RiftgateCoreError {
    /// Construct an internal error from a string-like value.
    ///
    /// Internal errors should be rare; their primary purpose is to give a
    /// human-readable signal when a subsystem hits an unexpected condition
    /// that isn't worth a typed variant.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}
