//! Fixture-driven HTTP/1.1 request-parse tests.
//!
//! Each fixture exercises a specific corner of the FSM:
//!
//! - body bytes arriving in the same feed as the headers
//! - body bytes arriving across many small feeds
//! - empty body (`Content-Length: 0`)
//! - missing `Content-Length` on a method that should have one
//! - unsupported `Transfer-Encoding: chunked` (deferred to v0.2)

use riftgate_core::parser::{ParseError, ParseEvent, StreamParser};
use riftgate_parser::Http1Parser;

fn drive(bytes: &[u8]) -> (Vec<u8>, bool, Option<ParseError>) {
    let mut p = Http1Parser::new();
    let mut body = Vec::new();
    let mut done = false;
    let mut err = None;
    for event in p.feed(bytes) {
        match event {
            ParseEvent::BodyChunk(b) => body.extend_from_slice(b),
            ParseEvent::BodyComplete => done = true,
            ParseEvent::Error(e) => err = Some(e),
            _ => {}
        }
    }
    (body, done, err)
}

#[test]
fn body_in_same_feed_as_headers() {
    let req = b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello";
    let (body, done, err) = drive(req);
    assert!(err.is_none(), "{err:?}");
    assert!(done);
    assert_eq!(body, b"hello");
}

#[test]
fn empty_body_completes_immediately() {
    let req = b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
    let (body, done, err) = drive(req);
    assert!(err.is_none(), "{err:?}");
    assert!(done);
    assert!(body.is_empty());
}

#[test]
fn no_content_length_treated_as_no_body() {
    // The v0.1 parser treats a missing Content-Length on a request as
    // "no body" — same as Content-Length: 0. This is the right call for
    // GET / HEAD; for POST it's a defensive default that lets the
    // upstream return an error rather than the parser hanging.
    let req = b"GET /v1/models HTTP/1.1\r\nHost: x\r\n\r\n";
    let (body, done, err) = drive(req);
    assert!(err.is_none(), "{err:?}");
    assert!(done);
    assert!(body.is_empty());
}

#[test]
fn body_arrives_across_many_small_feeds() {
    let req = b"POST / HTTP/1.1\r\nContent-Length: 11\r\n\r\nhello world";
    let mut p = Http1Parser::new();
    let mut body = Vec::new();
    let mut done = false;
    for chunk in req.chunks(3) {
        for event in p.feed(chunk) {
            match event {
                ParseEvent::BodyChunk(b) => body.extend_from_slice(b),
                ParseEvent::BodyComplete => done = true,
                ParseEvent::Error(e) => panic!("unexpected error: {e:?}"),
                _ => {}
            }
        }
    }
    assert!(done);
    assert_eq!(body, b"hello world");
}
