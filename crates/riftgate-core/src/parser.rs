//! Stream parser trait + the `ParseEvent` and `ParseError` shared types.
//!
//! Concrete parser impls (`Http1Parser`, `SseFramer`) live in
//! `crates/riftgate-parser`. This module declares the trait and the event
//! / error types that every impl shares so other crates can write
//! parser-agnostic code.
//!
//! See [`docs/04-design/lld-parsing.md`](../../../docs/04-design/lld-parsing.md)
//! for the FSM design rationale.

use crate::request::Headers;
use thiserror::Error;

/// Typed parse event.
///
/// The parser yields events as soon as they are unambiguous. Borrowed slices
/// are returned for body chunks and SSE tokens to avoid copying on the hot
/// path; callers that need to hold the data past the next `feed` call must
/// copy.
#[derive(Debug)]
pub enum ParseEvent<'a> {
    /// Headers (and the request line, for HTTP/1.1) have been fully parsed.
    HeadersComplete(Headers),
    /// One chunk of the request or response body. Borrowed from the input
    /// buffer; valid only until the next `feed` call.
    BodyChunk(&'a [u8]),
    /// The body is complete (signalled by `Content-Length` exhaustion or
    /// `Transfer-Encoding: chunked` zero-length terminator).
    BodyComplete,
    /// One Server-Sent Events token (the bytes between `data:` and `\n\n`).
    /// Borrowed from the input buffer; valid only until the next `feed`
    /// call.
    SseToken(&'a [u8]),
    /// The SSE stream has signalled completion (typically `data: [DONE]\n\n`
    /// for OpenAI streams).
    SseDone,
    /// A structural parse error. The parser is left in a terminal state;
    /// callers must call `reset` before feeding a new request.
    Error(ParseError),
}

/// Parse error.
///
/// Errors are typed (not strings) so downstream behavior can pattern-match.
#[derive(Debug, Clone, Eq, PartialEq, Error)]
pub enum ParseError {
    /// Headers exceeded the parser's scratch-buffer cap.
    #[error("header section exceeded scratch buffer cap ({0} bytes)")]
    HeaderTooLarge(usize),
    /// A chunk-encoding chunk-size line could not be parsed.
    #[error("invalid chunked encoding: {reason}")]
    InvalidChunkedEncoding {
        /// Human-readable reason; for diagnostics, not for `match`-ing.
        reason: &'static str,
    },
    /// The HTTP request line was malformed (bad method, version, or path).
    #[error("malformed request line: {reason}")]
    MalformedRequestLine {
        /// Human-readable reason; for diagnostics, not for `match`-ing.
        reason: &'static str,
    },
    /// A header line was malformed.
    #[error("malformed header line at byte offset {offset}")]
    MalformedHeader {
        /// Byte offset within the current scratch buffer.
        offset: usize,
    },
    /// An SSE event was structurally invalid (e.g. a `data:` line with no
    /// terminating `\n\n` and the stream closed).
    #[error("malformed SSE event: {reason}")]
    MalformedSse {
        /// Human-readable reason; for diagnostics.
        reason: &'static str,
    },
    /// The HTTP version is not supported by this parser.
    #[error("unsupported HTTP version: {0}")]
    UnsupportedHttpVersion(String),
}

/// Stream parser trait.
///
/// `feed` drives the FSM forward by appending bytes; `reset` returns the
/// parser to its initial state so it can be reused for a fresh request
/// (avoids allocation on the hot path).
///
/// **Trait object safety.** The trait uses a generic-free signature so
/// `Box<dyn StreamParser>` works.
pub trait StreamParser: Send {
    /// Feed bytes into the parser. Returns the events produced by this batch
    /// of input; the returned slice is borrowed from the input buffer where
    /// possible.
    ///
    /// Behavior:
    /// - The events are returned in the order they were unambiguously
    ///   parsed.
    /// - If the input is incomplete, no events for the in-progress message
    ///   are emitted; the parser remembers its state across calls.
    /// - On a structural error, a single `ParseEvent::Error` is emitted and
    ///   the parser is in a terminal state until `reset` is called.
    fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Vec<ParseEvent<'a>>;

    /// Return the parser to its initial state.
    ///
    /// Does not free the scratch buffer; the next `feed` call reuses it.
    fn reset(&mut self);
}
