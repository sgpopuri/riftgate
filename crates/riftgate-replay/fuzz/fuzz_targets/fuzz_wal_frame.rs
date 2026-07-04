//! Fuzz target: feed arbitrary bytes to the WAL frame parser (FR-404).
//!
//! Run with:
//!   cd crates/riftgate-replay/fuzz
//!   cargo fuzz run fuzz_wal_frame -- -max_total_time=60
//!
//! The invariant under test: `try_parse_frames` must NEVER panic on
//! arbitrary byte input, regardless of length or content.

#![no_main]

use libfuzzer_sys::fuzz_target;
use riftgate_replay::try_parse_frames;

fuzz_target!(|data: &[u8]| {
    // Feed the raw bytes to the frame parser. We consume the result to ensure
    // the compiler does not elide the call, but we do not assert anything
    // beyond "no panic" — the correctness properties are in the unit tests.
    let frames = try_parse_frames(data);
    // Access the results to prevent dead-code elimination.
    let _ = frames.len();
});
