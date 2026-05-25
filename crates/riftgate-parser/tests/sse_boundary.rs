//! `proptest`-driven SSE boundary tests.
//!
//! For any byte sequence `s` and any split point `k`, feeding `s` as
//! `s[..k]` then `s[k..]` MUST produce the same sequence of `SseToken`
//! events as feeding `s` in one go. This is the defining property of a
//! correct streaming parser; the boundary tests catch off-by-one errors
//! in the line-buffer / event-buffer FSM.

use proptest::prelude::*;
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::SseFramer;

fn drain_tokens(parser: &mut SseFramer, bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for event in parser.feed(bytes) {
        if let ParseEvent::SseToken(t) = event {
            out.push(t.to_vec());
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]

    /// Feeding the same byte sequence in two halves must produce the same
    /// token sequence as feeding it whole.
    #[test]
    fn feed_split_invariance(
        // Build inputs from a small alphabet of valid SSE bytes, biased
        // toward producing some `data:` lines and event boundaries.
        s in prop::collection::vec(prop::sample::select(b"data: hi\n\r:eventid".as_slice()), 0..256),
        split in 0..256usize,
    ) {
        let split = if s.is_empty() { 0 } else { split % s.len() };

        let mut whole_parser = SseFramer::new();
        let whole = drain_tokens(&mut whole_parser, &s);

        let mut split_parser = SseFramer::new();
        let mut split_tokens = drain_tokens(&mut split_parser, &s[..split]);
        split_tokens.extend(drain_tokens(&mut split_parser, &s[split..]));

        prop_assert_eq!(whole, split_tokens);
    }
}
