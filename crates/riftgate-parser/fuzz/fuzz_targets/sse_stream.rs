// cargo-fuzz harness: drive `SseFramer` against arbitrary input.

#![no_main]

use libfuzzer_sys::fuzz_target;
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::SseFramer;

fuzz_target!(|data: &[u8]| {
    let mut f = SseFramer::new();
    for event in f.feed(data) {
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
