//! `WasmFilter` scaffold for the v0.3 production filter ABI.
//!
//! Per [ADR 0019](../../../docs/06-adrs/0019-wasm-extension-mechanism.md):
//! the production impl binds the frozen `riftgate:filter/v1` Component
//! Model ABI via wasmtime, with AOT precompile via
//! `Engine::precompile_component`, instance pooling via
//! `PoolingAllocationConfig`, host functions `log` / `now-millis` /
//! `emit-counter`, and per-filter defaults of 5M fuel / 16 MiB memory /
//! 50 ms wallclock.
//!
//! Pass 1 (this commit) lands the **public type surface** so
//! callers compile against `WasmFilter` / `WasmFilterConfig` today. The
//! wasmtime engine, the WIT bindings, and the host-function table land in
//! a follow-on implementation PR; this scaffold returns
//! [`FilterAction::Continue`] from every entry point, behaving as the
//! identity filter under any configuration. Tests and callers may
//! substitute it freely; the substitution will be transparent when the
//! production impl lands.

use riftgate_core::{Filter, FilterAction, Request, Response};
use std::path::PathBuf;

/// Configuration for a [`WasmFilter`]. Defaults match
/// [ADR 0019](../../../docs/06-adrs/0019-wasm-extension-mechanism.md).
#[derive(Debug, Clone)]
pub struct WasmFilterConfig {
    /// Path to the WASM component (`.wasm` file) implementing
    /// `riftgate:filter/v1`.
    pub component_path: PathBuf,
    /// Per-call fuel budget. Default 5_000_000.
    pub fuel: u64,
    /// Per-call memory cap in bytes. Default 16 MiB.
    pub memory_bytes: u64,
    /// Per-call wallclock cap in milliseconds. Default 50.
    pub wallclock_ms: u32,
    /// Pre-allocated instance count for the wasmtime pooling allocator.
    /// Default 256.
    pub instance_pool: u32,
}

impl Default for WasmFilterConfig {
    fn default() -> Self {
        Self {
            component_path: PathBuf::from("/dev/null"),
            fuel: 5_000_000,
            memory_bytes: 16 * 1024 * 1024,
            wallclock_ms: 50,
            instance_pool: 256,
        }
    }
}

/// Errors produced while constructing or running a [`WasmFilter`].
#[derive(Debug)]
pub enum WasmFilterError {
    /// Configuration referenced a component file that could not be loaded.
    LoadFailed(String),
    /// The production wasmtime backend is not yet wired up in this build.
    /// Returned by the scaffold's `try_load` to make the "not yet
    /// implemented" path observable rather than silent.
    BackendNotWired,
}

impl core::fmt::Display for WasmFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LoadFailed(why) => write!(f, "wasm component load failed: {why}"),
            Self::BackendNotWired => f.write_str(
                "wasm runtime backend not yet wired; use FilterChain over native filters until the production wasm backend follow-on lands",
            ),
        }
    }
}

impl std::error::Error for WasmFilterError {}

/// `Filter` impl backed by a `riftgate:filter/v1` WASM component.
///
/// **Scaffold:** see module docs for the deferred-impl boundary. The
/// scaffold is `FilterAction::Continue` on every call; callers wire it in
/// today knowing the substitution will be transparent.
#[derive(Debug, Clone)]
pub struct WasmFilter {
    cfg: WasmFilterConfig,
}

impl WasmFilter {
    /// Construct a scaffold `WasmFilter` from a config. The scaffold does
    /// not touch the file system or the wasmtime engine.
    #[must_use]
    pub fn scaffold(cfg: WasmFilterConfig) -> Self {
        tracing::warn!(
            path = %cfg.component_path.display(),
            "WasmFilter scaffold: production wasmtime backend not yet wired; behaving as IdentityFilter"
        );
        Self { cfg }
    }

    /// The production constructor. Errors with
    /// `WasmFilterError::BackendNotWired`
    /// until the wasmtime backend lands in the follow-on implementation PR.
    ///
    /// # Errors
    /// See `WasmFilterError`.
    pub fn try_load(_cfg: WasmFilterConfig) -> Result<Self, WasmFilterError> {
        Err(WasmFilterError::BackendNotWired)
    }

    /// Snapshot the configuration. Useful in tests.
    #[must_use]
    pub fn config(&self) -> &WasmFilterConfig {
        &self.cfg
    }
}

impl Filter for WasmFilter {
    fn on_request(&self, _req: &mut Request) -> FilterAction {
        FilterAction::Continue
    }

    fn on_response(&self, _resp: &mut Response) -> FilterAction {
        FilterAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_load_reports_backend_not_wired() {
        let err = WasmFilter::try_load(WasmFilterConfig::default());
        assert!(matches!(err, Err(WasmFilterError::BackendNotWired)));
    }

    #[test]
    fn scaffold_behaves_as_identity() {
        let f = WasmFilter::scaffold(WasmFilterConfig::default());
        assert_eq!(f.config().fuel, 5_000_000);
    }
}
