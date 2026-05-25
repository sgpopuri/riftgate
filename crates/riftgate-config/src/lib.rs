//! # riftgate-config
//!
//! Riftgate's configuration surface for `v0.1`.
//!
//! Per [Options 015](../../../docs/05-options/015-config-model.md) and
//! [ADR 0012](../../../docs/06-adrs/0012-static-toml-env-override-v01.md):
//!
//! ```text
//!   defaults  --[layer 1]--+
//!                          |
//!     file   --[layer 2]---+--> merged Config --> validate(&Config) -+--> ArcSwap<Config>
//!                          |                                          |
//!     env    --[layer 3]---+                                          +--> Vec<ConfigError> (exit 78)
//! ```
//!
//! ## Public API
//!
//! - [`Config`] — the typed root configuration struct.
//! - [`load`] — pure function `load(path, env) -> Result<Config>`. Re-runnable.
//! - [`validate()`] — runs after the merge; returns every violation, not
//!   just the first.
//! - [`Secret`] — newtype that redacts at every leak surface.
//! - [`ConfigError`] — typed error variants surfaced by the loader and
//!   validator.
//!
//! Hot reload (`v0.2`/`v0.3`) lives outside this crate; it consumes
//! [`load`] and the [`#[reload = ...]`](Config) annotations on [`Config`]
//! fields to compute a safe-subset diff.

#![doc(html_root_url = "https://docs.rs/riftgate-config/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod error;
mod loader;
mod schema;
mod secret;
mod validate;

pub use error::ConfigError;
pub use loader::{Env, load};
pub use schema::{
    BackendConfig, Config, LogConfig, LogFormat, ObsConfig, ServerConfig, TimerConfig,
};
pub use secret::Secret;
pub use validate::validate;
