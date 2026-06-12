//! Shared `AsyncIO` conformance harness used by backend crates.
//!
//! The v0.2 close-out audit called out that IO conformance lived only in the
//! epoll backend test tree. This module centralizes the contract so multiple
//! backends can run the same behavior tests.

use riftgate_core::io::{AsyncIO, Interest};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

fn wait_for_token<I: AsyncIO>(io: &mut I, token: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let events = io.poll(Some(Duration::from_millis(50))).unwrap();
        if events.iter().any(|e| e.token == token) {
            return true;
        }
    }
    false
}

pub(crate) fn accept_round_trip_fires_readable_on_listener<I, F>(mut new_io: F)
where
    I: AsyncIO,
    F: FnMut() -> std::io::Result<I>,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    let mut io = new_io().unwrap();
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

pub(crate) fn echo_round_trip_fires_readable_on_accepted_socket<I, F>(mut new_io: F)
where
    I: AsyncIO,
    F: FnMut() -> std::io::Result<I>,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    let mut io = new_io().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();

    let client = thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(b"hello, riftgate").unwrap();
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
        "expected readable wakeup on accepted socket within 2s"
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

pub(crate) fn reregister_updates_interest_without_breaking_readable<I, F>(mut new_io: F)
where
    I: AsyncIO,
    F: FnMut() -> std::io::Result<I>,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    let mut io = new_io().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();

    io.register(listener.as_raw_fd(), 1, Interest::READABLE_AND_WRITABLE)
        .unwrap();

    let _client = thread::spawn(move || {
        let _ = TcpStream::connect(addr);
    });

    assert!(
        wait_for_token(&mut io, 1, Duration::from_secs(2)),
        "expected listener wakeup after re-register"
    );

    let _ = listener.accept();
    io.deregister(listener.as_raw_fd()).unwrap();
}

pub(crate) fn deregister_unknown_fd_is_no_op<I, F>(mut new_io: F)
where
    I: AsyncIO,
    F: FnMut() -> std::io::Result<I>,
{
    let mut io = new_io().unwrap();
    io.deregister(999_999)
        .expect("deregister of unknown fd should be no-op");
}

pub(crate) fn poll_with_zero_timeout_returns_empty_when_idle<I, F>(mut new_io: F)
where
    I: AsyncIO,
    F: FnMut() -> std::io::Result<I>,
{
    let mut io = new_io().unwrap();
    let events = io.poll(Some(Duration::from_millis(0))).unwrap();
    assert!(events.is_empty(), "expected no events on an idle backend");
}
