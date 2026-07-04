//! Riftgate Kubernetes operator binary.
//!
//! Without `--features operator`: prints a usage hint and exits.
//! With    `--features operator`: starts the `kube-runtime` controller loop
//!   watching `Riftgate`, `RiftgateBackend`, and `RiftgateRoute` CRDs.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use clap::Parser;
use std::process::ExitCode;

/// CLI arguments.
#[derive(Debug, Parser)]
#[command(
    name = "riftgate-operator",
    version,
    about = "Riftgate Kubernetes operator (riftgate.io/v1alpha1 CRDs)"
)]
struct Cli {
    /// Kubernetes namespace to watch. If absent, watches all namespaces.
    #[arg(long, env = "RIFTGATE_OPERATOR_NAMESPACE")]
    namespace: Option<String>,

    /// Leader-election lock name. Required in multi-replica deployments.
    #[arg(
        long,
        default_value = "riftgate-operator-leader",
        env = "RIFTGATE_OPERATOR_LEADER_LOCK"
    )]
    leader_lock: String,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    #[cfg(not(feature = "operator"))]
    {
        eprintln!(
            "riftgate-operator: compiled without the `operator` feature.\n\
             Rebuild with `--features operator` to enable the live controller.\n\
             \n\
             This stub binary is present so `cargo check --workspace` succeeds\n\
             without Kubernetes headers installed."
        );
        return ExitCode::from(2);
    }

    #[cfg(feature = "operator")]
    {
        let cli = Cli::parse();
        run_operator(cli)
    }
}

#[cfg(feature = "operator")]
fn run_operator(cli: Cli) -> ExitCode {
    use tokio::runtime::Builder;

    let rt = match Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("riftgate-operator: failed to build tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    rt.block_on(async move {
        tracing::info!(
            namespace = cli.namespace.as_deref().unwrap_or("<all>"),
            leader_lock = cli.leader_lock,
            "riftgate-operator starting"
        );

        // TODO(v1.0): wire kube::Client + kube_runtime::Controller for
        // Riftgate, RiftgateBackend, RiftgateRoute. See ADR 0030.
        tracing::warn!(
            "controller loop not yet implemented; \
             operator will exit after logging this message"
        );
        ExitCode::SUCCESS
    })
}
