//! `UringIO` — `AsyncIO` impl backed by `io_uring`.
//!
//! v0.2 scaffold. The ring is created at `new()` and exposes the
//! register/poll surface. Edge-triggered semantics live in the underlying
//! `IORING_OP_POLL_ADD` submission with a multishot flag, mirroring the
//! `EPOLLET` contract the epoll backend already documents.
//!
//! The conformance suite under
//! `crates/riftgate-io-epoll/tests/conformance.rs` becomes a shared harness
//! in v0.3; this crate ships construction + register + poll plumbing only.
//!
//! ## io_uring ring layout (kernel + userspace shared memory)
//!
//! ```text
//!                  userspace                          kernel
//!                 +--------------------+           +------------+
//!  register(fd) ->| Submission Queue   |---------->|            |
//!                 | (mmap'd, lock-free)|  io_uring | sqe worker |
//!                 |   PollAdd(fd, mask)|  enter()  |            |
//!                 |   user_data=token  |---------->|            |
//!                 +--------------------+           +------------+
//!                                                        |
//!                                                        v
//!                                                  ready event
//!                                                        |
//!                 +--------------------+                  |
//!  poll()      <--| Completion Queue   |<-----------------+
//!                 | (mmap'd, lock-free)|  io_uring         cqe.user_data
//!                 |   cqe.result mask  |  enter()          = our token
//!                 |   cqe.user_data    |
//!                 +--------------------+
//! ```
//!
//! ## Per-call flow
//!
//! ```text
//!  register(fd, token, interest):
//!     registrations[fd] = (token, interest)
//!     mask = POLLIN if readable else 0 | POLLOUT if writable else 0
//!     PollAdd(fd, mask, user_data=token).push(SQ); submit()
//!
//!  poll(timeout):
//!     submit_and_wait( 0 if timeout==ZERO else 1 )
//!     for cqe in CQ:
//!         if cqe.result < 0: skip   # -errno
//!         events.push(Event { token=cqe.user_data,
//!                              readable=mask & POLLIN,
//!                              writable=mask & POLLOUT })
//!     # PollAdd is one-shot; re-arm every fd that fired.
//!     for (fd, tok, intr) in fds_that_fired: submit_poll(...)
//!
//!  deregister(fd): drop from registrations; lingering CQEs ignored.
//! ```
//!
//! Per-shard ownership applies: each shard owns one `UringIO`; the ring
//! is not `Send`.

use io_uring::{IoUring, opcode, types};
use riftgate_core::io::{AsyncIO, Event, Interest};
use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

/// `AsyncIO` impl backed by an `io_uring` instance.
///
/// **Per-shard ownership** per [ADR `0004`](../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md):
/// each shard owns one `UringIO`. Not `Send`.
pub struct UringIO {
    ring: IoUring,
    registrations: HashMap<RawFd, (u64, Interest)>,
}

impl UringIO {
    /// Create a new `UringIO` with the given submission/completion queue
    /// depth (rounded up to a power of two by the kernel).
    ///
    /// # Errors
    /// Returns the underlying `io::Error` if the `io_uring_setup` syscall
    /// fails (typically because the kernel is older than 5.10 or the
    /// per-process ring quota is exhausted).
    pub fn new(entries: u32) -> io::Result<Self> {
        let ring = IoUring::new(entries)?;
        Ok(Self {
            ring,
            registrations: HashMap::new(),
        })
    }

    fn submit_poll(&mut self, fd: RawFd, token: u64, interest: Interest) -> io::Result<()> {
        let mut mask: u32 = 0;
        if interest.is_readable() {
            mask |= libc_pollin();
        }
        if interest.is_writable() {
            mask |= libc_pollout();
        }
        let entry = opcode::PollAdd::new(types::Fd(fd), mask)
            .build()
            .user_data(token);
        // SAFETY: the entry borrows no caller memory; `Fd` is a plain
        // `RawFd` value. The submission queue copies the entry by value.
        unsafe {
            self.ring
                .submission()
                .push(&entry)
                .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "io_uring SQ full"))?;
        }
        self.ring.submit()?;
        Ok(())
    }
}

fn libc_pollin() -> u32 {
    // Avoid pulling in the full `libc` dep for two constants. These are
    // ABI-stable on Linux.
    0x0001
}

fn libc_pollout() -> u32 {
    0x0004
}

impl AsyncIO for UringIO {
    fn register(&mut self, fd: RawFd, token: u64, interest: Interest) -> io::Result<()> {
        self.registrations.insert(fd, (token, interest));
        self.submit_poll(fd, token, interest)
    }

    fn deregister(&mut self, fd: RawFd) -> io::Result<()> {
        // Best-effort: drop the registration. Any pending completion that
        // arrives after this point will reference a token whose fd is gone
        // from `registrations`; the caller is expected to ignore tokens it
        // does not own.
        self.registrations.remove(&fd);
        Ok(())
    }

    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        // Submit any pending entries and wait for at least one completion
        // unless the timeout is `Some(Duration::ZERO)`.
        let wait_for = if timeout == Some(Duration::ZERO) {
            0
        } else {
            1
        };
        match self.ring.submit_and_wait(wait_for) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(4) => {} // EINTR
            Err(e) => return Err(e),
        }
        let mut events = Vec::new();
        let mut cq = self.ring.completion();
        cq.sync();
        for cqe in &mut cq {
            let token = cqe.user_data();
            let result = cqe.result();
            if result < 0 {
                // Negative result is `-errno`; skip for v0.2.
                continue;
            }
            let mask = result as u32;
            let readable = mask & libc_pollin() != 0;
            let writable = mask & libc_pollout() != 0;
            events.push(Event {
                token,
                readable,
                writable,
            });
        }
        // Resubmit poll entries for fds that fired — `PollAdd` without a
        // multishot flag is one-shot. Re-arming here matches the epoll
        // edge-triggered contract: the consumer drains, then we re-arm.
        let to_rearm: Vec<(RawFd, u64, Interest)> = self
            .registrations
            .iter()
            .filter(|(_, (tok, _))| events.iter().any(|e| e.token == *tok))
            .map(|(fd, (tok, interest))| (*fd, *tok, *interest))
            .collect();
        for (fd, tok, interest) in to_rearm {
            self.submit_poll(fd, tok, interest)?;
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_construct_ring() {
        let io = UringIO::new(32);
        assert!(io.is_ok(), "expected io_uring construction to succeed");
    }
}
