//! Criterion benchmark: accept + echo round-trip on a localhost TCP pair.
//!
//! Establishes a listener on an ephemeral port, registers it with
//! `MioIO`, then in a tight loop:
//!
//! - Connects from a client thread.
//! - Polls the listener until it is readable.
//! - Accepts the new socket.
//! - Reads a small payload from it (the client sends "ping").
//! - Closes both sockets.
//!
//! Measures the cost of one round-trip including the kernel's epoll/kqueue
//! syscalls. Useful as a regression gate when changes touch the IO loop.

use criterion::{Criterion, criterion_main};
use riftgate_core::io::{AsyncIO, Interest};
use riftgate_io_epoll::MioIO;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

fn one_round_trip(io: &mut MioIO, listener: &TcpListener) {
    let addr = listener.local_addr().unwrap();
    let client = thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(b"ping").unwrap();
        thread::sleep(Duration::from_millis(2));
    });

    // Wait for the listener to become readable.
    loop {
        let events = io.poll(Some(Duration::from_millis(100))).unwrap();
        if events.iter().any(|e| e.token == 1) {
            break;
        }
    }
    let (server, _) = listener.accept().unwrap();
    server.set_nonblocking(true).unwrap();
    io.register(server.as_raw_fd(), 2, Interest::READABLE)
        .unwrap();
    loop {
        let events = io.poll(Some(Duration::from_millis(100))).unwrap();
        if events.iter().any(|e| e.token == 2) {
            break;
        }
    }
    let mut buf = [0u8; 16];
    let _ = (&server).read(&mut buf);
    io.deregister(server.as_raw_fd()).unwrap();
    drop(server);
    client.join().unwrap();
}

fn bench_accept_echo(c: &mut Criterion) {
    let listener = TcpListener::bind("localhost:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut io = MioIO::new().unwrap();
    io.register(listener.as_raw_fd(), 1, Interest::READABLE)
        .unwrap();

    c.bench_function("io_epoll/accept_echo_round_trip", |b| {
        b.iter(|| {
            one_round_trip(&mut io, &listener);
        });
    });

    io.deregister(listener.as_raw_fd()).unwrap();
}

// `criterion_group!` expands to a `pub fn benches()` we cannot rustdoc;
// silence the workspace `missing_docs` lint on the bench harness.
#[allow(missing_docs)]
mod harness {
    use super::bench_accept_echo;
    use criterion::criterion_group;
    criterion_group!(benches, bench_accept_echo);
}
criterion_main!(harness::benches);
