//! Conformance tests for [`riftgate_io_epoll::MioIO`].
//!
//! This backend executes the shared `AsyncIO` conformance harness in
//! `riftgate-core/tests/io_conformance`.

use riftgate_io_epoll::MioIO;

#[path = "../../riftgate-core/tests/io_conformance/mod.rs"]
mod io_conformance;

#[test]
fn accept_round_trip_fires_readable_on_listener() {
    io_conformance::accept_round_trip_fires_readable_on_listener(MioIO::new);
}

#[test]
fn echo_round_trip_fires_readable_on_accepted_socket() {
    io_conformance::echo_round_trip_fires_readable_on_accepted_socket(MioIO::new);
}

#[test]
fn reregister_updates_interest_without_duplicating() {
    io_conformance::reregister_updates_interest_without_breaking_readable(MioIO::new);
}

#[test]
fn deregister_unknown_fd_is_no_op() {
    io_conformance::deregister_unknown_fd_is_no_op(MioIO::new);
}

#[test]
fn poll_with_zero_timeout_returns_empty_when_idle() {
    io_conformance::poll_with_zero_timeout_returns_empty_when_idle(MioIO::new);
}
