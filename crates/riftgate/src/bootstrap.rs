//! Bootstrap helpers: tracing-subscriber, OTel SDK, observability bus.
//!
//! Lives in its own module so `main.rs` reads as a top-level
//! orchestration script and the wiring details are factored out.

use crate::error::RiftgateError;
use riftgate_config::{Config, LogFormat};
use riftgate_obs::{BpfRuntimeState, BpfSink, Bus, JsonStdoutSink, MultiSink, OtelSink};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

/// Initialise the global `tracing` subscriber.
///
/// `RIFTGATE_LOG_LEVEL` (or the value from `config.log.level`) controls
/// the root filter; the `RUST_LOG` environment variable, when present,
/// overrides it.
pub fn init_tracing(config: &Config) {
    let env_filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new(config.log.level.clone()));
    let result = match config.log.format {
        LogFormat::Json => tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .json()
            .try_init(),
        LogFormat::Pretty => tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .pretty()
            .try_init(),
    };
    if let Err(e) = result {
        // try_init fails if a global subscriber was already installed
        // (e.g. by a parent process under test). Log and continue.
        eprintln!("warning: tracing subscriber already installed: {e}");
    }
}

/// Initialise the OpenTelemetry SDK with an OTLP/gRPC exporter.
///
/// Returns the constructed `TracerProvider` so the binary can call
/// `shutdown()` on it during drain.
///
/// On any setup failure this returns an error; callers should log and
/// fall back to a stdout-only sink instead of aborting.
pub fn init_otel(
    endpoint: &str,
) -> Result<opentelemetry_sdk::trace::TracerProvider, RiftgateError> {
    use opentelemetry_otlp::WithExportConfig;
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| RiftgateError::OpenTelemetry(e.to_string()))?;
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}

/// Build the observability bus.
///
/// - If `endpoint` is reachable at startup, the bus fans events out to
///   both `OtelSink` and `JsonStdoutSink`.
/// - If OTel init fails (no collector running, bad endpoint), the bus
///   ships events to `JsonStdoutSink` only and a warning is logged.
///
/// Returns `(bus, otel_provider_or_none)` so the caller can shut the
/// provider down on drain.
pub fn build_observability(
    config: &Config,
) -> (Bus, Option<opentelemetry_sdk::trace::TracerProvider>) {
    let json: Arc<dyn riftgate_core::obs::ObservabilitySink> = Arc::new(JsonStdoutSink::stdout());
    let bpf_sink = BpfSink::from_env();
    let bpf_state = bpf_sink.state().clone();

    let mut multi = MultiSink::new().with(json).with(Arc::new(bpf_sink));
    let otel_provider = match init_otel(&config.obs.otel_endpoint) {
        Ok(provider) => {
            let otel: Arc<dyn riftgate_core::obs::ObservabilitySink> = Arc::new(OtelSink::new());
            multi = multi.with(otel);
            tracing::info!(endpoint = %config.obs.otel_endpoint, "OTel SDK initialised");
            Some(provider)
        }
        Err(e) => {
            tracing::warn!(
                endpoint = %config.obs.otel_endpoint,
                error = %e,
                "OTel SDK init failed; continuing with JsonStdoutSink only"
            );
            None
        }
    };

    match &bpf_state {
        BpfRuntimeState::CompiledOut => {
            tracing::info!("BPF sink compiled out (requires Linux + riftgate-obs `bpf` feature)");
        }
        BpfRuntimeState::DisabledByEnv => {
            tracing::info!("BPF sink available but disabled (`RIFTGATE_ENABLE_BPF=1` to enable)");
        }
        BpfRuntimeState::Loaded { programs } => {
            tracing::info!(programs = ?programs, "BPF sink enabled and program slots loaded");
        }
    }

    let multi_arc: Arc<dyn riftgate_core::obs::ObservabilitySink> = Arc::new(multi);
    let bus = Bus::new(config.obs.bus_capacity, multi_arc);
    (bus, otel_provider)
}
