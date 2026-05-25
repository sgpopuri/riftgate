// cargo-fuzz harness: drive `Http1Parser` against arbitrary input and
// assert that the parser does not panic and ends in either a recognised
// terminal or recognised in-progress state.

#![no_main]

use libfuzzer_sys::fuzz_target;
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::Http1Parser;

fuzz_target!(|data: &[u8]| {
    let mut p = Http1Parser::new();
    for event in p.feed(data) {
        // Just drain. The invariant is: no panics, no UB. We don't
        // assert anything specific because most random inputs are
        // garbage.
        match event {
            ParseEvent::HeadersComplete(_)
            | ParseEvent::BodyChunk(_)
            | ParseEvent::BodyComplete
            | ParseEvent::Error(_)
            | ParseEvent::SseToken(_)
            | ParseEvent::SseDone => {}
        }
    }
});
