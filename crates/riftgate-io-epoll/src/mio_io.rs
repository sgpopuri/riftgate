//! [`MioIO`]: an `AsyncIO` backed by [`mio::Poll`].
//!
//! See the crate-level docs for the architecture overview.

use mio::unix::SourceFd;
use mio::{Events, Interest as MioInterest, Poll, Token};
use riftgate_core::io::{AsyncIO, Event, Interest};
use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

/// Capacity of the mio event-set buffer used by `poll`. Sized so a single
/// `poll` call can return up to this many ready events without an
/// allocation.
const EVENTS_BUF_CAPACITY: usize = 1024;

/// `AsyncIO` impl backed by `mio::Poll`.
///
/// Per-shard ownership: each shard's worker holds its own `MioIO`. Cross-
/// shard registration goes through the per-shard work queue, not through a
/// shared IO handle.
///
/// **Edge-triggered semantics on Linux.** mio uses edge-triggered epoll
/// internally; callers MUST drain to `EAGAIN` after every readable wakeup
/// or risk missing subsequent events. The conformance suite covers this.
pub struct MioIO {
    poll: Poll,
    events_buf: Events,
    registered: HashMap<RawFd, ()>,
}

impl MioIO {
    /// Construct a new `MioIO`.
    ///
    /// # Errors
    /// Returns the underlying OS error if `epoll_create1`/`kqueue` fails.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            poll: Poll::new()?,
            events_buf: Events::with_capacity(EVENTS_BUF_CAPACITY),
            registered: HashMap::new(),
        })
    }

    /// Number of file descriptors currently registered.
    ///
    /// Useful for tests and metrics; not on the hot path.
    pub fn registered_count(&self) -> usize {
        self.registered.len()
    }
}

#[allow(clippy::missing_panics_doc)]
fn mio_interest_from(i: Interest) -> MioInterest {
    match (i.is_readable(), i.is_writable()) {
        (true, true) => MioInterest::READABLE.add(MioInterest::WRITABLE),
        (true, false) => MioInterest::READABLE,
        (false, true) => MioInterest::WRITABLE,
        // The Interest type only exposes constructors that set at least
        // one bit, so this case is unreachable from outside the crate.
        // We surface it as a panic with a clear message rather than as
        // silent UB.
        (false, false) => unreachable!("Interest has no readable or writable bit set"),
    }
}

impl AsyncIO for MioIO {
    fn register(&mut self, fd: RawFd, token: u64, interest: Interest) -> io::Result<()> {
        let mut source = SourceFd(&fd);
        let mio_interest = mio_interest_from(interest);
        // mio's Token is `usize`; we map our `u64` token into it,
        // accepting the truncation on 32-bit targets. The `riftgate`
        // binary keeps tokens within `u32::MAX` per shard for this reason.
        #[allow(clippy::cast_possible_truncation)]
        let mio_token = Token(token as usize);
        if self.registered.contains_key(&fd) {
            self.poll
                .registry()
                .reregister(&mut source, mio_token, mio_interest)?;
        } else {
            self.poll
                .registry()
                .register(&mut source, mio_token, mio_interest)?;
            self.registered.insert(fd, ());
        }
        Ok(())
    }

    fn deregister(&mut self, fd: RawFd) -> io::Result<()> {
        if self.registered.remove(&fd).is_some() {
            let mut source = SourceFd(&fd);
            self.poll.registry().deregister(&mut source)?;
        }
        Ok(())
    }

    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        // Retry on EINTR (a common interruption from signals on Linux).
        loop {
            match self.poll.poll(&mut self.events_buf, timeout) {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let events: Vec<Event> = self
            .events_buf
            .iter()
            .map(|ev| Event {
                token: ev.token().0 as u64,
                readable: ev.is_readable(),
                writable: ev.is_writable(),
            })
            .collect();
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_succeeds() {
        let io = MioIO::new().expect("MioIO::new should not fail on a healthy system");
        assert_eq!(io.registered_count(), 0);
    }

    #[test]
    fn deregister_unknown_fd_is_no_op() {
        let mut io = MioIO::new().unwrap();
        // fd 999999 is almost certainly not open in the test process; the
        // trait contract is that deregister is a no-op for unknown fds,
        // not an error.
        let result = io.deregister(999_999);
        assert!(
            result.is_ok(),
            "deregister of unknown fd should be no-op, got {result:?}"
        );
    }
}
