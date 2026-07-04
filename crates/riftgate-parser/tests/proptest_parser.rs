//! Property-based tests for `Http1Parser` and `SseFramer` (FR-404).
//!
//! Invariants verified against arbitrary inputs:
//!
//! - **No panic:** the parser never panics on arbitrary byte sequences.
//! - **Incremental == batch:** splitting a valid request at any offset
//!   produces the same final body as a single-shot feed.
//! - **Invalid method never silently completes:** junk input does not
//!   emit `BodyComplete` without a prior `Error`.
//! - **SSE tokens are non-empty:** every `SseToken` event has at least one byte.

use proptest::prelude::*;
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::{Http1Parser, SseFramer};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_body(input: &[u8]) -> Vec<u8> {
    let mut parser = Http1Parser::new();
    let mut out = Vec::new();
    for event in parser.feed(input) {
        if let ParseEvent::BodyChunk(b) = event {
            out.extend_from_slice(b);
        }
    }
    out
}

fn valid_post(body: &[u8]) -> Vec<u8> {
    let mut req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Length: {}\r\n\
         \r\n",
        body.len()
    )
    .into_bytes();
    req.extend_from_slice(body);
    req
}

// ---------------------------------------------------------------------------
// Http1Parser: no-panic property
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn http1_parser_never_panics(input in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = collect_body(&input);
    }
}

// ---------------------------------------------------------------------------
// Http1Parser: incremental == batch
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn http1_incremental_matches_batch(
        body in proptest::collection::vec(any::<u8>(), 0..256),
        split_frac in 0.0f64..1.0,
    ) {
        let request = valid_post(&body);
        let batch_body = collect_body(&request);

        let split = ((request.len() as f64) * split_frac) as usize;
        let mut parser = Http1Parser::new();
        let mut incremental = Vec::new();
        for event in parser.feed(&request[..split]) {
            if let ParseEvent::BodyChunk(b) = event {
                incremental.extend_from_slice(b);
            }
        }
        for event in parser.feed(&request[split..]) {
            if let ParseEvent::BodyChunk(b) = event {
                incremental.extend_from_slice(b);
            }
        }
        prop_assert_eq!(batch_body, incremental);
    }
}

// ---------------------------------------------------------------------------
// Http1Parser: junk start never silently completes
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn http1_junk_never_completes_silently(
        noise in proptest::collection::vec(
            any::<u8>().prop_filter("not uppercase ASCII", |b| !b.is_ascii_uppercase()),
            1..64,
        ),
    ) {
        let mut parser = Http1Parser::new();
        let events: Vec<_> = parser.feed(&noise);
        let has_complete = events.iter().any(|e| matches!(e, ParseEvent::BodyComplete));
        let has_error = events.iter().any(|e| matches!(e, ParseEvent::Error(_)));
        if has_complete {
            prop_assert!(has_error, "BodyComplete emitted without Error on junk input");
        }
    }
}

// ---------------------------------------------------------------------------
// SseFramer: no-panic property
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn sse_framer_never_panics(input in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut framer = SseFramer::new();
        let _ = framer.feed(&input);
    }
}

// ---------------------------------------------------------------------------
// SseFramer: SseToken events are non-empty
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn sse_framer_tokens_are_nonempty(
        payload in "[a-zA-Z0-9 ]{1,64}",
    ) {
        let frame = format!("data:{payload}\n\n");
        let mut framer = SseFramer::new();
        for event in framer.feed(frame.as_bytes()) {
            if let ParseEvent::SseToken(bytes) = event {
                prop_assert!(!bytes.is_empty(), "SseToken was empty");
            }
        }
    }
}
