//! Drain coordination for graceful shutdown.
//!
//! On `SIGTERM` / `SIGINT` the binary stops accepting new connections,
//! flips `/ready` to 503, and waits up to a configurable deadline for
//! in-flight requests to complete. After the deadline the process
//! exits regardless.
//!
//! Implementation:
//!
//! - One [`tokio::sync::watch`] channel carries the boolean
//!   "is_draining" flag. The accept loop selects on it; per-request
//!   handlers borrow it for the `/ready` decision.
//! - One [`tokio::sync::Notify`] is used to ask the accept loop to
//!   exit promptly (after a final accept iteration).

use tokio::sync::watch;

/// Sender side of the drain signal.
pub type DrainSender = watch::Sender<bool>;

/// Receiver side of the drain signal. Cheap to clone via
/// [`watch::Receiver::clone`].
pub type DrainReceiver = watch::Receiver<bool>;

/// Construct the drain signal pair.
///
/// Initial state is `false` (not draining). Call [`begin_drain`] when
/// a shutdown signal arrives.
pub fn channel() -> (DrainSender, DrainReceiver) {
    watch::channel(false)
}

/// Mark the gateway as draining. Idempotent.
pub fn begin_drain(tx: &DrainSender) {
    let _ = tx.send(true);
}

/// `true` if the gateway is currently draining.
pub fn is_draining(rx: &DrainReceiver) -> bool {
    *rx.borrow()
}

/// Block the current task until SIGTERM or SIGINT.
///
/// Returns the name of the signal that was received, for logging.
///
/// On non-Unix platforms only Ctrl-C is observed.
pub async fn wait_for_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "could not install SIGTERM handler; falling back to SIGINT only");
                tokio::signal::ctrl_c()
                    .await
                    .expect("could not install SIGINT handler");
                return "SIGINT";
            }
        };
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "could not install SIGINT handler; using SIGTERM only");
                sigterm.recv().await;
                return "SIGTERM";
            }
        };
        tokio::select! {
            _ = sigterm.recv() => "SIGTERM",
            _ = sigint.recv() => "SIGINT",
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("could not install Ctrl-C handler");
        "Ctrl-C"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_starts_false_and_can_flip() {
        let (tx, rx) = channel();
        assert!(!is_draining(&rx));
        begin_drain(&tx);
        assert!(is_draining(&rx));
        // Idempotent.
        begin_drain(&tx);
        assert!(is_draining(&rx));
    }
}
