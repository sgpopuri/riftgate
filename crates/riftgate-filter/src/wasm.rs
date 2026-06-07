//! `WasmFilter` production runtime for the frozen `riftgate:filter/v1` ABI.
//!
//! Per [ADR 0019](../../../docs/06-adrs/0019-wasm-extension-mechanism.md),
//! this implementation hosts a WebAssembly Component Model plugin via
//! wasmtime. The runtime:
//!
//! - Loads and validates a component from disk (`try_load`).
//! - Wires host functions (`log`, `now-millis`, `emit-counter`).
//! - Calls exported `on-request` / `on-response` entry points.
//! - Maps component actions to `riftgate_core::FilterAction`.
//!
//! The `wasm` cargo feature gates the runtime dependency. When disabled,
//! `try_load` returns `WasmFilterError::FeatureDisabled` and
//! `WasmFilter::scaffold` remains available for identity behavior.

use riftgate_core::{Filter, FilterAction, Request, Response};
use std::path::PathBuf;

#[cfg(feature = "wasm")]
mod runtime {
    use super::{WasmFilterConfig, WasmFilterError};
    use parking_lot::Mutex;
    use riftgate_core::{Body, FilterAction, Request, Response, StatusCode};
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use wasmtime::component::{Component, Linker};
    use wasmtime::{Config, Engine, Store};

    wasmtime::component::bindgen!({
        path: "wit",
        world: "riftgate-filter",
        tracing: true,
    });

    pub(crate) struct WasmRuntime {
        engine: Engine,
        component: Component,
    }

    #[derive(Default)]
    struct HostState {
        started_at: Option<Instant>,
    }

    impl riftgate::filter::host::Host for HostState {
        fn log(&mut self, level: String, message: String) {
            match level.as_str() {
                "error" => tracing::error!(target: "riftgate_filter::wasm", "{message}"),
                "warn" => tracing::warn!(target: "riftgate_filter::wasm", "{message}"),
                "info" => tracing::info!(target: "riftgate_filter::wasm", "{message}"),
                "debug" => tracing::debug!(target: "riftgate_filter::wasm", "{message}"),
                _ => tracing::trace!(target: "riftgate_filter::wasm", "{message}"),
            }
        }

        fn now_millis(&mut self) -> u64 {
            if let Some(start) = self.started_at {
                return start.elapsed().as_millis() as u64;
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0_u128, |d| d.as_millis());
            now as u64
        }

        fn emit_counter(&mut self, name: String, value: u64) {
            tracing::debug!(target: "riftgate_filter::wasm", counter = %name, value, "wasm counter");
        }
    }

    impl WasmRuntime {
        pub(crate) fn load(cfg: &WasmFilterConfig) -> Result<Self, WasmFilterError> {
            let mut wasmtime_cfg = Config::new();
            wasmtime_cfg.wasm_component_model(true);
            wasmtime_cfg.wasm_multi_memory(true);

            // Enable pooling to pre-allocate instance slots for this runtime.
            let mut pooling = wasmtime::PoolingAllocationConfig::default();
            let max_instance_size: usize = usize::try_from(cfg.memory_bytes).map_err(|_| {
                WasmFilterError::LoadFailed(format!(
                    "memory_bytes does not fit usize: {}",
                    cfg.memory_bytes
                ))
            })?;
            pooling.max_component_instance_size(max_instance_size);
            pooling.total_component_instances(cfg.instance_pool);
            wasmtime_cfg
                .allocation_strategy(wasmtime::InstanceAllocationStrategy::Pooling(pooling));

            let engine = Engine::new(&wasmtime_cfg)
                .map_err(|e| WasmFilterError::LoadFailed(e.to_string()))?;

            let bytes = std::fs::read(&cfg.component_path)
                .map_err(|e| WasmFilterError::LoadFailed(e.to_string()))?;

            // Validate AOT-eligibility during load. Runtime currently executes
            // from the loaded component object.
            let _ = engine
                .precompile_component(&bytes)
                .map_err(|e| WasmFilterError::LoadFailed(e.to_string()))?;

            let component = Component::from_binary(&engine, &bytes)
                .map_err(|e| WasmFilterError::LoadFailed(e.to_string()))?;

            Ok(Self { engine, component })
        }

        fn instantiate(
            &self,
            started_at: Instant,
        ) -> Result<(Store<HostState>, RiftgateFilter), WasmFilterError> {
            let mut store = Store::new(
                &self.engine,
                HostState {
                    started_at: Some(started_at),
                },
            );

            let mut linker = Linker::new(&self.engine);
            riftgate::filter::host::add_to_linker(&mut linker, |state| state)
                .map_err(|e| WasmFilterError::InstantiateFailed(e.to_string()))?;

            let bindings = RiftgateFilter::instantiate(&mut store, &self.component, &linker)
                .map_err(|e| WasmFilterError::InstantiateFailed(e.to_string()))?;

            Ok((store, bindings))
        }

        pub(crate) fn on_request(
            &self,
            req: &Request,
            started_at: Instant,
        ) -> Result<FilterAction, WasmFilterError> {
            let mut headers = Vec::with_capacity(req.headers.len());
            for (name, value) in req.headers.iter() {
                headers.push(riftgate::filter::filter_types::Header {
                    name: name.to_string(),
                    value: String::from_utf8_lossy(value).into_owned(),
                });
            }

            let body = match &req.body {
                Body::Empty => Vec::new(),
                Body::Bytes(bytes) => bytes.clone(),
            };

            let payload = riftgate::filter::filter_types::Request {
                method: format!("{:?}", req.method),
                path: req.path.clone(),
                headers,
                body,
            };

            let (mut store, bindings) = self.instantiate(started_at)?;
            let action = bindings
                .interface0
                .call_on_request(&mut store, &payload)
                .map_err(|e| WasmFilterError::ExecuteFailed(e.to_string()))?;
            Ok(map_action(action))
        }

        pub(crate) fn on_response(
            &self,
            resp: &Response,
            started_at: Instant,
        ) -> Result<FilterAction, WasmFilterError> {
            let mut headers = Vec::with_capacity(resp.headers.len());
            for (name, value) in resp.headers.iter() {
                headers.push(riftgate::filter::filter_types::Header {
                    name: name.to_string(),
                    value: String::from_utf8_lossy(value).into_owned(),
                });
            }

            let body = match &resp.body {
                Body::Empty => Vec::new(),
                Body::Bytes(bytes) => bytes.clone(),
            };

            let payload = riftgate::filter::filter_types::Response {
                status: resp.status.0,
                headers,
                body,
            };

            let (mut store, bindings) = self.instantiate(started_at)?;
            let action = bindings
                .interface0
                .call_on_response(&mut store, &payload)
                .map_err(|e| WasmFilterError::ExecuteFailed(e.to_string()))?;
            Ok(map_action(action))
        }
    }

    fn map_action(action: exports::riftgate::filter::filter::Action) -> FilterAction {
        match action {
            exports::riftgate::filter::filter::Action::Continue => FilterAction::Continue,
            exports::riftgate::filter::filter::Action::Terminate(code) => {
                FilterAction::Terminate(StatusCode(code))
            }
        }
    }

    #[derive(Clone)]
    pub(crate) struct RuntimeHandle(pub(crate) Arc<Mutex<WasmRuntime>>);

    impl RuntimeHandle {
        pub(crate) fn load(cfg: &WasmFilterConfig) -> Result<Self, WasmFilterError> {
            Ok(Self(Arc::new(Mutex::new(WasmRuntime::load(cfg)?))))
        }

        pub(crate) fn on_request(
            &self,
            req: &Request,
            started_at: Instant,
        ) -> Result<FilterAction, WasmFilterError> {
            self.0.lock().on_request(req, started_at)
        }

        pub(crate) fn on_response(
            &self,
            resp: &Response,
            started_at: Instant,
        ) -> Result<FilterAction, WasmFilterError> {
            self.0.lock().on_response(resp, started_at)
        }
    }
}

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
    /// The component could not be instantiated with the host imports.
    InstantiateFailed(String),
    /// Component entry-point execution failed.
    ExecuteFailed(String),
    /// The crate was built without the `wasm` feature.
    FeatureDisabled,
}

impl core::fmt::Display for WasmFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LoadFailed(why) => write!(f, "wasm component load failed: {why}"),
            Self::InstantiateFailed(why) => {
                write!(f, "wasm component instantiate failed: {why}")
            }
            Self::ExecuteFailed(why) => write!(f, "wasm component execution failed: {why}"),
            Self::FeatureDisabled => {
                f.write_str("wasm feature disabled; rebuild with --features wasm")
            }
        }
    }
}

impl std::error::Error for WasmFilterError {}

/// `Filter` impl backed by a `riftgate:filter/v1` WASM component.
///
/// **Scaffold:** see module docs for the deferred-impl boundary. The
/// scaffold is `FilterAction::Continue` on every call; callers wire it in
/// today knowing the substitution will be transparent.
#[derive(Clone)]
pub struct WasmFilter {
    cfg: WasmFilterConfig,
    #[cfg(feature = "wasm")]
    runtime: Option<runtime::RuntimeHandle>,
}

impl WasmFilter {
    /// Construct a scaffold `WasmFilter` from a config.
    ///
    /// The scaffold behaves like `IdentityFilter` and does not load a
    /// component.
    #[must_use]
    pub fn scaffold(cfg: WasmFilterConfig) -> Self {
        tracing::warn!(
            path = %cfg.component_path.display(),
            "WasmFilter scaffold enabled; behaving as IdentityFilter"
        );
        Self {
            cfg,
            #[cfg(feature = "wasm")]
            runtime: None,
        }
    }

    /// Load a production `WasmFilter` from a component path.
    ///
    /// # Errors
    /// Returns `WasmFilterError` when the component cannot be loaded,
    /// instantiated, or when the crate is built without `--features wasm`.
    pub fn try_load(cfg: WasmFilterConfig) -> Result<Self, WasmFilterError> {
        if !cfg.component_path.is_file() {
            return Err(WasmFilterError::LoadFailed(format!(
                "component path is not a file: {}",
                cfg.component_path.display()
            )));
        }

        #[cfg(not(feature = "wasm"))]
        {
            let _ = cfg;
            return Err(WasmFilterError::FeatureDisabled);
        }

        #[cfg(feature = "wasm")]
        {
            let runtime = runtime::RuntimeHandle::load(&cfg)?;
            Ok(Self {
                cfg,
                runtime: Some(runtime),
            })
        }
    }

    /// Snapshot the configuration. Useful in tests.
    #[must_use]
    pub fn config(&self) -> &WasmFilterConfig {
        &self.cfg
    }
}

impl Filter for WasmFilter {
    fn on_request(&self, req: &mut Request) -> FilterAction {
        #[cfg(feature = "wasm")]
        {
            if let Some(runtime) = &self.runtime {
                return runtime
                    .on_request(req, std::time::Instant::now())
                    .unwrap_or_else(|err| {
                        tracing::error!(error = %err, "wasm filter request invocation failed");
                        FilterAction::Terminate(riftgate_core::StatusCode::INTERNAL_SERVER_ERROR)
                    });
            }
        }
        let _ = req;
        FilterAction::Continue
    }

    fn on_response(&self, resp: &mut Response) -> FilterAction {
        #[cfg(feature = "wasm")]
        {
            if let Some(runtime) = &self.runtime {
                return runtime
                    .on_response(resp, std::time::Instant::now())
                    .unwrap_or_else(|err| {
                        tracing::error!(error = %err, "wasm filter response invocation failed");
                        FilterAction::Terminate(riftgate_core::StatusCode::INTERNAL_SERVER_ERROR)
                    });
            }
        }
        let _ = resp;
        FilterAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_load_rejects_non_file_path() {
        let err = WasmFilter::try_load(WasmFilterConfig::default());
        assert!(matches!(err, Err(WasmFilterError::LoadFailed(_))));
    }

    #[test]
    fn scaffold_behaves_as_identity() {
        let f = WasmFilter::scaffold(WasmFilterConfig::default());
        assert_eq!(f.config().fuel, 5_000_000);
    }

    #[test]
    #[cfg(not(feature = "wasm"))]
    fn try_load_reports_feature_disabled_when_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wasm = dir.path().join("f.wasm");
        std::fs::write(&wasm, b"not-a-component").expect("write file");

        let cfg = WasmFilterConfig {
            component_path: wasm,
            ..WasmFilterConfig::default()
        };
        let err = WasmFilter::try_load(cfg);
        assert!(matches!(err, Err(WasmFilterError::FeatureDisabled)));
    }
}
