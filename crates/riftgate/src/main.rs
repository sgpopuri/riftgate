//! Riftgate v0.1 walking-skeleton binary.
//!
//! The binary is a thin orchestrator over the library entry points
//! defined in [`riftgate`](crate). All logic lives there so the
//! integration tests in `tests/e2e.rs` can stand the same stack up
//! against a mock upstream.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use arc_swap::ArcSwap;
use clap::Parser;
use riftgate::error::RiftgateError;
use riftgate::mcp;
use riftgate::signals;
use riftgate::{bootstrap, proxy, server, shutdown, upstream};
use riftgate_config::{Env, load};
use riftgate_core::GpuPressureSource;
use riftgate_core::router::{BackendId, BackendPool, BackendSignals};
use riftgate_obs::DcgmScrapeSource;
use riftgate_router::{
    CircuitBreakerArbiter, CircuitBreakerConfig, HedgedConfig, HedgedRouter, KvAwareConfig,
    KvAwareRouter, WeightedRandomRouter,
};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

/// CLI flags.
///
/// Per [ADR 0012](../../docs/06-adrs/0012-static-toml-env-override-v01.md)
/// the gateway accepts a TOML config file, overlays `RIFTGATE_*` env
/// vars on top, validates the result, and exits non-zero on any
/// validation failure.
#[derive(Debug, Parser)]
#[command(
    name = "riftgate",
    version,
    about = "Riftgate: a programmable Rust gateway for OpenAI-compatible LLM traffic.",
    long_about = None,
)]
struct Cli {
    /// Path to a TOML config file. Optional: when absent, the loader
    /// uses defaults plus `RIFTGATE_*` env overrides.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Validate the configuration and exit. Non-zero on any error.
    #[arg(long)]
    check: bool,

    /// Drain grace window in milliseconds. After SIGTERM the binary
    /// waits this long for in-flight requests before forcibly closing
    /// remaining connections.
    #[arg(long, default_value_t = 30_000, value_name = "MS")]
    drain_grace_ms: u64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("riftgate: failed to construct tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        match run(cli).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(RiftgateError::Config(msg)) => {
                eprintln!("riftgate: {msg}");
                ExitCode::from(78)
            }
            Err(e) => {
                eprintln!("riftgate: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

/// Build a multi-thread Tokio runtime per [ADR 0003](../../docs/06-adrs/0003-tokio-multithread-default.md).
fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    #[cfg(feature = "per-core-scheduler")]
    builder.thread_name("riftgate-per-core");
    #[cfg(not(feature = "per-core-scheduler"))]
    builder.thread_name("riftgate-worker");
    builder.build()
}

async fn run(cli: Cli) -> Result<(), RiftgateError> {
    let env = Env::from_process();
    let config = load(cli.config.as_deref(), &env).map_err(RiftgateError::from)?;

    bootstrap::init_tracing(&config);
    #[cfg(feature = "per-core-scheduler")]
    tracing::info!("scheduler mode: per-core-scheduler feature enabled");
    #[cfg(not(feature = "per-core-scheduler"))]
    tracing::info!("scheduler mode: tokio default");
    tracing::info!(
        listen = %config.server.listen_addr,
        backend = %config.backend.url,
        "riftgate starting"
    );

    if cli.check {
        tracing::info!("--check: configuration is valid; exiting");
        return Ok(());
    }

    let (bus, otel_provider) = bootstrap::build_observability(&config);
    let publisher = bus.publisher();

    let upstream_client = upstream::build_client();
    let pool = Arc::new(BackendPool::from_ids(vec![BackendId(0)]));
    let signals = Arc::new(ArcSwap::from_pointee(BackendSignals::new()));
    let weighted = WeightedRandomRouter::new(&[(BackendId(0), 1)]);
    let kv_aware = KvAwareRouter::new(weighted, KvAwareConfig::default());
    let hedged = HedgedRouter::new(kv_aware, HedgedConfig::default());
    let routed = CircuitBreakerArbiter::new(hedged, CircuitBreakerConfig::default());
    let router: Arc<dyn riftgate_core::router::Router> = Arc::new(routed);

    let listener = server::bind(config.server.listen_addr).await.map_err(|e| {
        RiftgateError::Io(std::io::Error::new(
            e.kind(),
            format!(
                "could not bind {addr}: {e}",
                addr = config.server.listen_addr
            ),
        ))
    })?;
    let bound_addr = listener.local_addr()?;
    tracing::info!(addr = %bound_addr, "listening");

    let (drain_tx, drain_rx) = shutdown::channel();
    tokio::spawn(async move {
        let signal = shutdown::wait_for_signal().await;
        tracing::info!(signal, "received shutdown signal; beginning drain");
        shutdown::begin_drain(&drain_tx);
    });

    if let Ok(endpoint) = env::var("RIFTGATE_GPU_DCGM_ENDPOINT") {
        let signals_store = Arc::clone(&signals);
        tokio::spawn(async move {
            let source = DcgmScrapeSource::new(BackendId(0), endpoint, 0)
                .with_timeout(Duration::from_secs(2));
            tracing::info!(source = source.name(), "GPU pressure poller enabled");

            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                match source.poll_once() {
                    Ok(samples) => {
                        signals::apply_gpu_pressure_updates(&signals_store, &samples);
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "GPU pressure poll failed");
                    }
                }
            }
        });
    } else {
        tracing::info!("GPU pressure poller disabled (set RIFTGATE_GPU_DCGM_ENDPOINT to enable)");
    }

    let mcp_broker = mcp::build_mcp_broker(&config);
    let state = proxy::HandlerState {
        config: Arc::new(config),
        router,
        pool,
        signals,
        upstream: upstream_client,
        publisher,
        drain: drain_rx.clone(),
        mcp_broker,
    };
    let drain_grace = Duration::from_millis(cli.drain_grace_ms);
    let accepted = server::accept_loop(listener, state, drain_rx, drain_grace).await?;

    tracing::info!(accepted, "accept loop exited; flushing observability");

    if let Some(provider) = otel_provider {
        if let Err(e) = provider.shutdown() {
            tracing::warn!(error = %e, "otel provider shutdown returned an error");
        }
    }

    drop(bus);

    Ok(())
}
