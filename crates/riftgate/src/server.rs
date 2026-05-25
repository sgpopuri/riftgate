//! TCP accept loop and per-connection HTTP/1.1 service.
//!
//! ```text
//!   bind(listen_addr)
//!         |
//!         v
//!   accept_loop:
//!       loop:
//!           select:
//!               accept() -> spawn(handle_connection)
//!               drain_signal -> break
//!       wait for in-flight handlers (or until grace deadline)
//! ```

use crate::proxy::{HandlerState, handle};
use crate::shutdown::{DrainReceiver, is_draining};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time;

/// Bind a TCP listener on the given address. Reports the actual bound
/// address (useful when `port=0` for tests).
pub async fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind(addr).await?;
    Ok(listener)
}

/// Run the accept loop until `drain` flips to `true`, then drain
/// in-flight requests up to `drain_grace`. Returns the count of
/// connections accepted over the loop's lifetime.
pub async fn accept_loop(
    listener: TcpListener,
    state: HandlerState,
    mut drain: DrainReceiver,
    drain_grace: Duration,
) -> std::io::Result<usize> {
    let local_addr = listener.local_addr()?;
    tracing::info!(addr = %local_addr, "riftgate accept loop started");

    let in_flight = Arc::new(AtomicUsize::new(0));
    let mut connection_tasks = JoinSet::new();
    let mut accepted = 0usize;

    loop {
        if is_draining(&drain) {
            break;
        }

        tokio::select! {
            biased;
            // Drain notification: leave the accept loop and start
            // the in-flight wait.
            res = drain.changed() => {
                if res.is_err() {
                    // Sender dropped — treat the same as a drain.
                    break;
                }
                if is_draining(&drain) {
                    break;
                }
            }
            res = listener.accept() => {
                match res {
                    Ok((stream, peer)) => {
                        accepted += 1;
                        let state = state.clone();
                        let in_flight = in_flight.clone();
                        in_flight.fetch_add(1, Ordering::SeqCst);
                        connection_tasks.spawn(async move {
                            handle_connection(stream, peer, state).await;
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    Err(e) => {
                        // Per `tokio::net::TcpListener::accept` docs the
                        // error is per-accept and almost always recoverable
                        // (resource limits, peer reset). We log and continue.
                        tracing::warn!(error = %e, "accept failed; continuing");
                        time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        }
    }

    tracing::info!(
        in_flight = in_flight.load(Ordering::SeqCst),
        grace_ms = drain_grace.as_millis() as u64,
        "drain initiated; waiting for in-flight requests"
    );

    // Drain phase: wait for either all in-flight tasks to complete or
    // the grace deadline to elapse.
    let drained = tokio::select! {
        () = wait_until_done(&mut connection_tasks) => true,
        () = time::sleep(drain_grace) => false,
    };

    if drained {
        tracing::info!(accepted, "drain complete");
    } else {
        tracing::warn!(
            accepted,
            still_in_flight = in_flight.load(Ordering::SeqCst),
            "drain grace expired; aborting in-flight connections"
        );
        connection_tasks.shutdown().await;
    }

    Ok(accepted)
}

/// Helper: wait until every task in `set` finishes.
async fn wait_until_done(set: &mut JoinSet<()>) {
    while set.join_next().await.is_some() {}
}

/// Serve one accepted connection.
///
/// We use hyper 1.x's HTTP/1 connection driver. Requests are dispatched
/// to [`crate::proxy::handle`] via `service_fn`.
async fn handle_connection(stream: tokio::net::TcpStream, peer: SocketAddr, state: HandlerState) {
    let _ = stream.set_nodelay(true);
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req| {
        let state = state.clone();
        async move { handle(req, state).await }
    });
    let conn = http1::Builder::new()
        .keep_alive(true)
        .serve_connection(io, svc);
    if let Err(e) = conn.await {
        // Hyper surfaces normal client disconnects as errors; log at
        // debug to keep the access log readable.
        tracing::debug!(peer = %peer, error = %e, "http/1 connection ended with error");
    }
}
