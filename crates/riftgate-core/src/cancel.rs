//! Stream-cancellation primitive — v0.3 trait surface.
//!
//! Per [ADR `0020`](../../../docs/06-adrs/0020-stream-cancellation-cancellation-token.md)
//! and [Options `024`](../../../docs/05-options/024-stream-cancellation.md):
//!
//! - Riftgate uses `tokio_util::sync::CancellationToken` as the substrate.
//! - The kernel exposes two typed newtypes over it:
//!   - [`Cancellation`] — the *cancel-side* handle (one per request), trips
//!     the token with a typed [`CancelCause`].
//!   - [`CancellationDriver`] — the *await-side* handle (cloned freely
//!     across the IO future, the parser future, the upstream future, and
//!     the SSE framer), reports the trip cause to consumers.
//! - The SSE framer keys its terminal `Cancelled { bytes_seen, cause }`
//!   state off the driver. See `crates/riftgate-parser/src/sse.rs`.
//!
//! ## State machine
//!
//! ```text
//!   +-----------+    .cancel(cause)        +-----------+
//!   |   Live    | -----------------------> | Cancelled |
//!   |  (open)   |                          | (sealed)  |
//!   +-----------+                          +-----------+
//!         |                                      ^
//!         |  Drop (last Cancellation handle)     |
//!         +--------------------------------------+
//!         cancel(DroppedByClient) on the way down
//! ```
//!
//! The drop path is the load-bearing reason we own a thin newtype rather
//! than passing `CancellationToken` directly: dropping a request future
//! mid-stream *must* trip the cancellation with `DroppedByClient` so the
//! upstream future tears down on the next poll. Doing that by hand at every
//! call site is error-prone; doing it in `impl Drop for Cancellation` is
//! free.

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Why a request was cancelled.
///
/// Typed (not a string) so downstream observers and filters can branch on
/// the cause without parsing free-form messages. The discriminant is also
/// packed into the shared atomic that the [`CancellationDriver`] reads, so
/// the cause is observable lock-free from any future awaiting the
/// cancellation.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum CancelCause {
    /// Sentinel value indicating the cancellation has not tripped yet.
    /// Never returned to user code; only used internally by the atomic
    /// state.
    NotCancelled = 0,
    /// The client disconnected before the response completed.
    DroppedByClient = 1,
    /// The request's wall-clock deadline expired.
    DeadlineExceeded = 2,
    /// A backend declared the request unrecoverable (5xx, 408, 504, or a
    /// circuit-breaker open). The hedger uses this cause when it cancels
    /// the loser of a hedge race.
    UpstreamFailed = 3,
    /// The hedge winner has returned; cancel the loser mid-stream.
    HedgeLost = 4,
    /// A filter chain returned [`crate::filter::FilterAction::Terminate`].
    FilterTerminated = 5,
    /// The gateway is draining (SIGTERM); finish active requests up to a
    /// grace deadline, then cancel.
    Draining = 6,
    /// Caller-supplied cause not covered by the canonical list. Used by
    /// tests and by future extension points.
    Other = 7,
}

impl CancelCause {
    /// Wire-format string for this cancel cause. Stable for observability.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotCancelled => "not_cancelled",
            Self::DroppedByClient => "dropped_by_client",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::UpstreamFailed => "upstream_failed",
            Self::HedgeLost => "hedge_lost",
            Self::FilterTerminated => "filter_terminated",
            Self::Draining => "draining",
            Self::Other => "other",
        }
    }

    /// Decode a wire-format byte back into a `CancelCause`. Used by the
    /// driver to read the packed atomic.
    fn from_u8(b: u8) -> Self {
        match b {
            1 => Self::DroppedByClient,
            2 => Self::DeadlineExceeded,
            3 => Self::UpstreamFailed,
            4 => Self::HedgeLost,
            5 => Self::FilterTerminated,
            6 => Self::Draining,
            7 => Self::Other,
            _ => Self::NotCancelled,
        }
    }
}

impl fmt::Display for CancelCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Internal state shared between a [`Cancellation`] and any number of
/// [`CancellationDriver`] clones.
///
/// The state is split into two fields:
///
/// - `token` — a `tokio_util::sync::CancellationToken`, the awaitable
///   substrate.
/// - `cause` — an `AtomicU8` holding the typed `CancelCause` packed via
///   `CancelCause::from_u8`. The first `.cancel(cause)` call writes the
///   cause then trips the token; observers read the cause after the token
///   trips.
#[derive(Debug)]
struct CancelState {
    token: CancellationToken,
    cause: AtomicU8,
}

impl CancelState {
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            cause: AtomicU8::new(CancelCause::NotCancelled as u8),
        }
    }

    fn trip(&self, cause: CancelCause) {
        // CAS-loop into the cause slot: only the first writer wins. Race-
        // tolerant by design — if two paths attempt to cancel concurrently
        // (client disconnect + deadline expiry), the first one's cause is
        // recorded; both paths still observe `is_cancelled` after the
        // store.
        let _ = self.cause.compare_exchange(
            CancelCause::NotCancelled as u8,
            cause as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.token.cancel();
    }

    fn observed_cause(&self) -> CancelCause {
        CancelCause::from_u8(self.cause.load(Ordering::Acquire))
    }
}

/// Cancel-side handle for a single request.
///
/// One `Cancellation` per inbound request. Tripping it (via [`Cancellation::cancel`]
/// or by dropping the value) causes every cloned [`CancellationDriver`] to
/// observe the cancellation; the SSE framer transitions to its terminal
/// `Cancelled` state on the next read.
///
/// Cloneable only inside the kernel for hedger / draining bookkeeping; the
/// public API surface keeps `Cancellation` non-`Clone` so the ownership
/// shape stays one-per-request.
pub struct Cancellation {
    state: Arc<CancelState>,
    drop_cause: CancelCause,
}

impl fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cancellation")
            .field("tripped", &self.state.token.is_cancelled())
            .field("drop_cause", &self.drop_cause)
            .finish()
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancellation {
    /// Construct a fresh `Cancellation`. Drop-cause defaults to
    /// [`CancelCause::DroppedByClient`]: dropping the handle without an
    /// explicit cancel is the "client gave up" path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancelState::new()),
            drop_cause: CancelCause::DroppedByClient,
        }
    }

    /// Override the cause that the drop path reports. Tests use this to
    /// model deterministic drop scenarios; the gateway's draining path
    /// sets `Draining` so concurrent in-flight requests report a deliberate
    /// shutdown rather than a client disconnect.
    pub fn with_drop_cause(mut self, cause: CancelCause) -> Self {
        self.drop_cause = cause;
        self
    }

    /// Trip the cancellation with the given cause.
    ///
    /// Idempotent: only the first call records a cause; subsequent calls
    /// are no-ops. Tripping is observed by every cloned
    /// [`CancellationDriver`] on the next poll.
    pub fn cancel(&self, cause: CancelCause) {
        self.state.trip(cause);
    }

    /// Mint a [`CancellationDriver`] that observes the same trip. Cheap;
    /// clones an `Arc`.
    #[must_use]
    pub fn driver(&self) -> CancellationDriver {
        CancellationDriver {
            state: Arc::clone(&self.state),
        }
    }

    /// `true` if the cancellation has tripped.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.token.is_cancelled()
    }

    /// Cause recorded for this cancellation, or
    /// [`CancelCause::NotCancelled`] if untripped.
    #[must_use]
    pub fn cause(&self) -> CancelCause {
        self.state.observed_cause()
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        // If we still own the only strong reference, no driver is waiting;
        // skip the trip to avoid synthesising spurious cancellations in
        // tests that build a Cancellation, never clone a driver, and drop
        // it.
        if Arc::strong_count(&self.state) > 1 && !self.state.token.is_cancelled() {
            self.state.trip(self.drop_cause);
        }
    }
}

/// Await-side handle. Cloned freely across all the futures that need to
/// honor the cancellation (IO, parser, upstream, SSE framer).
#[derive(Debug, Clone)]
pub struct CancellationDriver {
    state: Arc<CancelState>,
}

impl CancellationDriver {
    /// `true` if the cancellation has tripped.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.token.is_cancelled()
    }

    /// Cause recorded for this cancellation. Returns
    /// [`CancelCause::NotCancelled`] before the trip.
    #[must_use]
    pub fn cause(&self) -> CancelCause {
        self.state.observed_cause()
    }

    /// Await the cancellation trip. Returns the cause.
    ///
    /// Cooperative: callers race this future against their own IO future
    /// via `tokio::select!` and act on whichever finishes first.
    pub async fn cancelled(&self) -> CancelCause {
        self.state.token.cancelled().await;
        self.state.observed_cause()
    }

    /// Best-effort synchronous wait with a wall-clock deadline. Returns
    /// `None` if the deadline expires before the cancellation trips. Used
    /// by graceful-drain code that already owns a tokio runtime.
    pub async fn wait_with_deadline(&self, deadline: Duration) -> Option<CancelCause> {
        match tokio::time::timeout(deadline, self.state.token.cancelled()).await {
            Ok(()) => Some(self.state.observed_cause()),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_cancellation_is_live() {
        let c = Cancellation::new();
        assert!(!c.is_cancelled());
        assert_eq!(c.cause(), CancelCause::NotCancelled);
    }

    #[test]
    fn cancel_records_typed_cause() {
        let c = Cancellation::new();
        c.cancel(CancelCause::DeadlineExceeded);
        assert!(c.is_cancelled());
        assert_eq!(c.cause(), CancelCause::DeadlineExceeded);
    }

    #[test]
    fn drivers_observe_trip() {
        let c = Cancellation::new();
        let d = c.driver();
        assert!(!d.is_cancelled());
        c.cancel(CancelCause::UpstreamFailed);
        assert!(d.is_cancelled());
        assert_eq!(d.cause(), CancelCause::UpstreamFailed);
    }

    #[test]
    fn second_cancel_does_not_overwrite_cause() {
        let c = Cancellation::new();
        c.cancel(CancelCause::HedgeLost);
        c.cancel(CancelCause::DroppedByClient);
        assert_eq!(c.cause(), CancelCause::HedgeLost);
    }

    #[test]
    fn drop_with_outstanding_driver_trips_default_cause() {
        let c = Cancellation::new();
        let d = c.driver();
        drop(c);
        assert!(d.is_cancelled());
        assert_eq!(d.cause(), CancelCause::DroppedByClient);
    }

    #[test]
    fn drop_cause_override() {
        let c = Cancellation::new().with_drop_cause(CancelCause::Draining);
        let d = c.driver();
        drop(c);
        assert_eq!(d.cause(), CancelCause::Draining);
    }

    #[test]
    fn cause_strings_are_stable() {
        assert_eq!(CancelCause::DroppedByClient.as_str(), "dropped_by_client");
        assert_eq!(CancelCause::DeadlineExceeded.as_str(), "deadline_exceeded");
        assert_eq!(CancelCause::HedgeLost.as_str(), "hedge_lost");
    }
}
