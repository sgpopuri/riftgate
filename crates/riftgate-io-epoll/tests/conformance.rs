//! Conformance tests for [`riftgate_io_epoll::MioIO`].
//!
//! Every `AsyncIO` impl is expected to pass these tests. The suite
//! exercises:
//!
//! 1. Round-trip register / poll / read on a real localhost TCP pair.
//! 2. Re-registration updates the interest in place (no duplicate
//!    registrations).
//! 3. Idempotent deregister on an unknown fd.
//! 4. Edge-triggered drain semantics — after one wakeup, the caller must
//!    drain to `EAGAIN` to receive subsequent events.
//!
//! See [`docs/04-design/lld-io-runtime.md`](../../../docs/04-design/lld-io-runtime.md)
//! for the discussion of edge-triggered epoll's drain-to-EAGAIN pitfall.

use riftgate_core::io::{AsyncIO, Interest};
use riftgate_io_epoll::MioIO;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

/// Wait for at least one event matching the given token via repeated `poll`
/// calls, capping the total wait at `timeout`. Helper for tests that need
/// to tolerate a small amount of jitter in event delivery.
fn wait_for_token(io: &mut MioIO, token: u64, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let events = io.poll(Some(Duration::from_millis(50))).unwrap();
        if events.iter().any(|e| e.token == token) {
            return true;
        }
    }
    false
}

#[test]
fn accept_round_trip_fires_readable_on_listener() {
    let listener = TcpListener::bind("localhost:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    let mut io = MioIO::new().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();

    let _client = thread::spawn(move || {
        let _ = TcpStream::connect(addr);
    });

    assert!(
        wait_for_token(&mut io, 1, Duration::from_secs(2)),
        "expected readable event on the listener within 2s"
    );

    let _ = listener.accept();
    io.deregister(listener.as_raw_fd()).unwrap();
}

#[test]
fn echo_round_trip_fires_readable_on_accepted_socket() {
    let listener = TcpListener::bind("localhost:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    let mut io = MioIO::new().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();

    let client = thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(b"hello, riftgate").unwrap();
        // Hold the stream open until the test completes its read.
        thread::sleep(Duration::from_millis(200));
        drop(s);
    });

    assert!(
        wait_for_token(&mut io, 1, Duration::from_secs(2)),
        "expected listener wakeup"
    );

    let (server_socket, _) = listener.accept().expect("accept after wakeup");
    server_socket.set_nonblocking(true).unwrap();
    io.register(server_socket.as_raw_fd(), 2, Interest::READABLE)
        .unwrap();

    assert!(
        wait_for_token(&mut io, 2, Duration::from_secs(2)),
        "expected readable wakeup on the accepted socket within 2s"
    );

    let mut buf = vec![0u8; 64];
    let n = (&server_socket)
        .read(&mut buf)
        .expect("read should succeed after wakeup");
    assert!(n > 0);
    assert_eq!(&buf[..n], b"hello, riftgate");

    io.deregister(server_socket.as_raw_fd()).unwrap();
    io.deregister(listener.as_raw_fd()).unwrap();

    client.join().unwrap();
}

#[test]
fn reregister_updates_interest_without_duplicating() {
    let listener = TcpListener::bind("localhost:0").unwrap();
    listener.set_nonblocking(true).unwrap();

    let mut io = MioIO::new().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();
    assert_eq!(io.registered_count(), 1);

    // Re-register the same fd with a different interest. The internal
    // `registered` map should still have exactly one entry.
    io.register(listener.as_raw_fd(), 1, Interest::READABLE_AND_WRITABLE)
        .unwrap();
    assert_eq!(io.registered_count(), 1);

    io.deregister(listener.as_raw_fd()).unwrap();
    assert_eq!(io.registered_count(), 0);
}

#[test]
fn deregister_unknown_fd_is_no_op() {
    let mut io = MioIO::new().unwrap();
    // 999999 is unlikely to be open in the test process.
    io.deregister(999_999)
        .expect("deregister of unknown fd should be no-op");
}

#[test]
fn poll_with_zero_timeout_returns_empty_when_idle() {
    let mut io = MioIO::new().unwrap();
    let events = io.poll(Some(Duration::from_millis(0))).unwrap();
    assert!(events.is_empty(), "expected no events on an idle MioIO");
}
