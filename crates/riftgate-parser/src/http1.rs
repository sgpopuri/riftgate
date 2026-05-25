//! HTTP/1.1 request parser.
//!
//! Headers are delegated to `httparse` (well-tested, ~10 ns per header,
//! used by hyper itself). Body framing is a small explicit FSM that
//! supports `Content-Length`-bounded bodies in v0.1; chunked encoding
//! lands in v0.2 per [ADR 0007](../../../../docs/06-adrs/0007-handrolled-fsm-parser.md).
//!
//! ```text
//!   AwaitingHeaders                        (httparse loop on scratch[consumed..])
//!         |  (Complete)
//!         v
//!   StreamingBodyContentLength { remaining }
//!         |  (remaining == 0)
//!         v
//!   Done
//!
//!   On `Transfer-Encoding: chunked`, v0.1 emits ParseError::InvalidChunkedEncoding
//!   and transitions to Error. Chunked support lands in v0.2.
//! ```

use riftgate_core::parser::{ParseError, ParseEvent, StreamParser};
use riftgate_core::request::Headers;

/// Maximum number of headers `httparse` will accept in a single request.
///
/// Most real-world HTTP/1.1 requests carry well under 64 headers; this
/// cap is a defense against pathological clients sending tens of
/// thousands of headers (which would cost CPU on every parse attempt).
const MAX_HEADERS: usize = 64;

/// Maximum byte size of the header section. Defends against an unbounded
/// `scratch` growth on a misbehaving client. Per [`docs/04-design/lld-parsing.md`](../../../../docs/04-design/lld-parsing.md)
/// the v0.1 default is 8 KiB; the v0.2 work makes it configurable.
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// HTTP/1.1 request parser.
///
/// See the module-level docs for the FSM diagram. State transitions are:
///
/// 1. `AwaitingHeaders` → re-runs `httparse::Request::parse` on every
///    `feed` until headers are complete.
/// 2. `StreamingBodyContentLength { remaining }` → emits `BodyChunk`
///    events as bytes arrive, decrementing `remaining` until zero.
/// 3. `Done` → no more events; `reset` returns to `AwaitingHeaders`.
pub struct Http1Parser {
    scratch: Vec<u8>,
    consumed: usize,
    phase: Phase,
}

#[derive(Debug)]
enum Phase {
    AwaitingHeaders,
    StreamingBodyContentLength { remaining: usize },
    Done,
    Error,
}

impl Http1Parser {
    /// Construct a new `Http1Parser` in the initial `AwaitingHeaders`
    /// state.
    pub fn new() -> Self {
        Self {
            scratch: Vec::with_capacity(2048),
            consumed: 0,
            phase: Phase::AwaitingHeaders,
        }
    }
}

impl Default for Http1Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal "what to emit" instruction. Built up as the FSM runs (during
/// the `&mut self` mutation phase) and converted to events at the end of
/// `feed` (during the read-only borrow phase).
enum Emit {
    Headers(Headers),
    BodyRange(std::ops::Range<usize>),
    BodyComplete,
    Error(ParseError),
}

fn parse_content_length(headers: &Headers) -> Option<usize> {
    let raw = headers.get("content-length")?;
    let s = std::str::from_utf8(raw).ok()?;
    s.trim().parse::<usize>().ok()
}

fn is_chunked(headers: &Headers) -> bool {
    headers
        .get("transfer-encoding")
        .map(|v| v.eq_ignore_ascii_case(b"chunked"))
        .unwrap_or(false)
}

impl StreamParser for Http1Parser {
    #[allow(clippy::too_many_lines)] // FSM is naturally one routine
    fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Vec<ParseEvent<'a>> {
        let mut emit: Vec<Emit> = Vec::new();
        if matches!(self.phase, Phase::Error) {
            return emit
                .into_iter()
                .map(|i| convert(i, &self.scratch))
                .collect();
        }

        self.scratch.extend_from_slice(bytes);

        loop {
            match &self.phase {
                Phase::AwaitingHeaders => {
                    if self.scratch.len() > MAX_HEADER_BYTES {
                        emit.push(Emit::Error(ParseError::HeaderTooLarge(self.scratch.len())));
                        self.phase = Phase::Error;
                        break;
                    }

                    let mut header_buf = [httparse::EMPTY_HEADER; MAX_HEADERS];
                    let mut req = httparse::Request::new(&mut header_buf);

                    match req.parse(&self.scratch[self.consumed..]) {
                        Ok(httparse::Status::Complete(n)) => {
                            // Build owned Headers from the parsed slice.
                            let mut headers = Headers::new();
                            for h in req.headers.iter() {
                                if h.name.is_empty() {
                                    break;
                                }
                                headers.insert(h.name.to_owned(), h.value.to_vec());
                            }

                            // Determine body framing.
                            let next_phase = if is_chunked(&headers) {
                                emit.push(Emit::Error(ParseError::InvalidChunkedEncoding {
                                    reason: "chunked encoding lands in v0.2; \
                                             v0.1 supports Content-Length only",
                                }));
                                Phase::Error
                            } else {
                                let cl = parse_content_length(&headers).unwrap_or(0);
                                if cl == 0 {
                                    emit.push(Emit::Headers(headers));
                                    emit.push(Emit::BodyComplete);
                                    Phase::Done
                                } else {
                                    emit.push(Emit::Headers(headers));
                                    Phase::StreamingBodyContentLength { remaining: cl }
                                }
                            };

                            self.consumed += n;
                            self.phase = next_phase;
                            // Continue the loop to drain any body bytes
                            // that arrived in the same feed as the
                            // headers.
                            continue;
                        }
                        Ok(httparse::Status::Partial) => {
                            // Headers not complete yet; wait for more
                            // bytes.
                            break;
                        }
                        Err(e) => {
                            // `httparse::Error` is `#[non_exhaustive]`, so a
                            // catch-all arm exists for forward compat even
                            // though the explicit variants below cover every
                            // value the current crate version emits.
                            let pe = match e {
                                httparse::Error::HeaderName
                                | httparse::Error::HeaderValue
                                | httparse::Error::NewLine => ParseError::MalformedHeader {
                                    offset: self.consumed,
                                },
                                httparse::Error::Token => ParseError::MalformedRequestLine {
                                    reason: "invalid token in request line",
                                },
                                httparse::Error::Status => ParseError::MalformedRequestLine {
                                    reason: "invalid status (this parser handles requests)",
                                },
                                httparse::Error::Version => {
                                    ParseError::UnsupportedHttpVersion("unknown".into())
                                }
                                httparse::Error::TooManyHeaders => {
                                    ParseError::HeaderTooLarge(self.scratch.len())
                                }
                                #[allow(unreachable_patterns)]
                                _ => ParseError::MalformedRequestLine {
                                    reason: "unknown httparse error (forward-compat)",
                                },
                            };
                            emit.push(Emit::Error(pe));
                            self.phase = Phase::Error;
                            break;
                        }
                    }
                }
                Phase::StreamingBodyContentLength { remaining } => {
                    let avail = self.scratch.len() - self.consumed;
                    if avail == 0 {
                        break;
                    }
                    let take = avail.min(*remaining);
                    emit.push(Emit::BodyRange(self.consumed..self.consumed + take));
                    self.consumed += take;
                    let new_remaining = *remaining - take;
                    if new_remaining == 0 {
                        emit.push(Emit::BodyComplete);
                        self.phase = Phase::Done;
                    } else {
                        self.phase = Phase::StreamingBodyContentLength {
                            remaining: new_remaining,
                        };
                    }
                }
                Phase::Done | Phase::Error => break,
            }
        }

        let scratch_ref: &[u8] = &self.scratch;
        emit.into_iter().map(|i| convert(i, scratch_ref)).collect()
    }

    fn reset(&mut self) {
        self.scratch.clear();
        self.consumed = 0;
        self.phase = Phase::AwaitingHeaders;
    }
}

fn convert(i: Emit, scratch: &[u8]) -> ParseEvent<'_> {
    match i {
        Emit::Headers(h) => ParseEvent::HeadersComplete(h),
        Emit::BodyRange(r) => ParseEvent::BodyChunk(&scratch[r]),
        Emit::BodyComplete => ParseEvent::BodyComplete,
        Emit::Error(e) => ParseEvent::Error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive_parse(bytes: &[u8]) -> (Option<Headers>, Vec<u8>, bool, Option<ParseError>) {
        let mut p = Http1Parser::new();
        let mut headers = None;
        let mut body = Vec::new();
        let mut complete = false;
        let mut err = None;
        for event in p.feed(bytes) {
            match event {
                ParseEvent::HeadersComplete(h) => headers = Some(h),
                ParseEvent::BodyChunk(b) => body.extend_from_slice(b),
                ParseEvent::BodyComplete => complete = true,
                ParseEvent::Error(e) => err = Some(e),
                ParseEvent::SseToken(_) | ParseEvent::SseDone => panic!("unexpected SSE event"),
            }
        }
        (headers, body, complete, err)
    }

    #[test]
    fn parses_get_no_body() {
        let req = b"GET /v1/models HTTP/1.1\r\nHost: api.openai.com\r\n\r\n";
        let (headers, body, complete, err) = drive_parse(req);
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(complete);
        assert!(body.is_empty());
        let h = headers.expect("headers should parse");
        assert_eq!(h.get("Host"), Some(&b"api.openai.com"[..]));
    }

    #[test]
    fn parses_post_with_content_length() {
        // `\r\n\<newline>` continuation in Rust raw multi-line strings
        // discards the trailing whitespace on the next line, so the body
        // here is exactly 12 bytes: `{"hello":1}\n`. The header advertises
        // the same length so the parser emits `BodyComplete`.
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\
                    Host: api.openai.com\r\n\
                    Content-Type: application/json\r\n\
                    Content-Length: 12\r\n\
                    \r\n\
                    {\"hello\":1}\n";
        let (headers, body, complete, err) = drive_parse(req);
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(complete, "body should be complete");
        assert_eq!(body, b"{\"hello\":1}\n");
        let h = headers.expect("headers should parse");
        assert_eq!(h.get("Content-Length"), Some(&b"12"[..]));
    }

    #[test]
    fn handles_split_feeds() {
        // Feed the request one byte at a time. The parser must accumulate.
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\
                    Host: api.openai.com\r\n\
                    Content-Length: 5\r\n\
                    \r\n\
                    abcde";
        let mut p = Http1Parser::new();
        let mut body = Vec::new();
        let mut got_complete = false;
        let mut got_headers = false;
        for byte in req.iter() {
            for event in p.feed(std::slice::from_ref(byte)) {
                match event {
                    ParseEvent::HeadersComplete(_) => got_headers = true,
                    ParseEvent::BodyChunk(b) => body.extend_from_slice(b),
                    ParseEvent::BodyComplete => got_complete = true,
                    ParseEvent::Error(e) => panic!("unexpected error: {e:?}"),
                    _ => {}
                }
            }
        }
        assert!(got_headers, "headers should eventually complete");
        assert!(got_complete, "body should eventually complete");
        assert_eq!(body, b"abcde");
    }

    #[test]
    fn chunked_returns_error_in_v01() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\
                    Host: api.openai.com\r\n\
                    Transfer-Encoding: chunked\r\n\
                    \r\n";
        let (_h, _b, _c, err) = drive_parse(req);
        match err {
            Some(ParseError::InvalidChunkedEncoding { .. }) => {}
            other => panic!("expected InvalidChunkedEncoding, got {other:?}"),
        }
    }

    #[test]
    fn rejects_oversized_headers() {
        let mut req: Vec<u8> = b"GET / HTTP/1.1\r\n".to_vec();
        // Fill to over MAX_HEADER_BYTES with bogus header lines that
        // never close — the parser should bail with HeaderTooLarge.
        while req.len() < MAX_HEADER_BYTES + 100 {
            req.extend_from_slice(b"X-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        let (_h, _b, _c, err) = drive_parse(&req);
        match err {
            Some(ParseError::HeaderTooLarge(_)) => {}
            other => panic!("expected HeaderTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn reset_returns_to_initial_state() {
        let mut p = Http1Parser::new();
        let req = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
        let _ = p.feed(req);
        p.reset();
        // Feed again; should parse cleanly without leftover state.
        let req2 = b"GET /b HTTP/1.1\r\nHost: y\r\n\r\n";
        let mut got_headers = false;
        for event in p.feed(req2) {
            if let ParseEvent::HeadersComplete(_) = event {
                got_headers = true;
            }
        }
        assert!(got_headers);
    }
}
