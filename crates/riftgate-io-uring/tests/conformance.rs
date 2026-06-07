#![cfg(all(target_os = "linux", feature = "io-uring"))]

//! Conformance tests for [`riftgate_io_uring::UringIO`].
//!
//! Runs the shared `AsyncIO` conformance harness from
//! `riftgate-core/tests/io_conformance`. Some Linux environments disable
//! `io_uring`; in that case these tests return early.

use riftgate_io_uring::UringIO;

#[path = "../../riftgate-core/tests/io_conformance/mod.rs"]
mod io_conformance;

fn make_uring() -> std::io::Result<UringIO> {
    UringIO::new(256)
}

fn uring_supported() -> bool {
    match UringIO::new(8) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("skipping io_uring conformance tests: {e}");
            false
        }
    }
}

#[test]
fn accept_round_trip_fires_readable_on_listener() {
    if !uring_supported() {
        return;
    }
    io_conformance::accept_round_trip_fires_readable_on_listener(make_uring);
}

#[test]
fn echo_round_trip_fires_readable_on_accepted_socket() {
    if !uring_supported() {
        return;
    }
    io_conformance::echo_round_trip_fires_readable_on_accepted_socket(make_uring);
}

#[test]
fn reregister_updates_interest_without_breaking_readable() {
    if !uring_supported() {
        return;
    }
    io_conformance::reregister_updates_interest_without_breaking_readable(make_uring);
}

#[test]
fn deregister_unknown_fd_is_no_op() {
    if !uring_supported() {
        return;
    }
    io_conformance::deregister_unknown_fd_is_no_op(make_uring);
}

#[test]
fn poll_with_zero_timeout_returns_empty_when_idle() {
    if !uring_supported() {
        return;
    }
    io_conformance::poll_with_zero_timeout_returns_empty_when_idle(make_uring);
}
