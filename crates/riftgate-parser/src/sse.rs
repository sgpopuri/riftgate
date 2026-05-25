//! Server-Sent Events stream framer.
//!
//! Parses an `EventSource`-formatted stream:
//!
//! ```text
//!   data: {"choices":[{"delta":{"content":"Hello"}}]}\n
//!   \n
//!   data: {"choices":[{"delta":{"content":", world"}}]}\n
//!   \n
//!   data: [DONE]\n
//!   \n
//! ```
//!
//! Each `data:` line contributes its payload (after the colon and an
//! optional space) to the in-progress event; a blank line terminates the
//! event and produces an [`SseToken`](riftgate_core::parser::ParseEvent::SseToken)
//! (or [`SseDone`](riftgate_core::parser::ParseEvent::SseDone) for the
//! sentinel `[DONE]` payload).
//!
//! Other SSE line types (`event: ...`, `id: ...`, `retry: ...`,
//! `:comment`) are recognised and skipped; v0.1 only consumes the data
//! channel.
//!
//! ```text
//!   Reading
//!     |  (\n on data line)  --> append payload to current_event
//!     |  (\n on blank line + current_event non-empty) --> emit SseToken / SseDone
//!     |  (\n on event:/id:/comment line) --> ignore
//!     |  (truly malformed, e.g. bare \r without \n) --> Error
//!     v
//!   Done (after SseDone)
//! ```

use riftgate_core::parser::{ParseError, ParseEvent, StreamParser};

/// Server-Sent Events stream framer. See module docs for the FSM.
pub struct SseFramer {
    /// Bytes accumulated since the last newline; the in-progress *line*.
    line_buf: Vec<u8>,
    /// Bytes accumulated since the last event boundary; the in-progress
    /// *event payload*. Multiple `data:` lines concatenate (with `\n`
    /// separators per the SSE spec).
    current_event: Vec<u8>,
    /// Per-feed payload buffer. Cleared at the start of every `feed`;
    /// emitted events borrow from this buffer.
    output_buffer: Vec<u8>,
    phase: Phase,
}

#[derive(Debug)]
enum Phase {
    Reading,
    Done,
    Error,
}

impl SseFramer {
    /// Construct a new `SseFramer` in the initial state.
    pub fn new() -> Self {
        Self {
            line_buf: Vec::with_capacity(256),
            current_event: Vec::with_capacity(512),
            output_buffer: Vec::with_capacity(2048),
            phase: Phase::Reading,
        }
    }
}

impl Default for SseFramer {
    fn default() -> Self {
        Self::new()
    }
}

enum Emit {
    Token(std::ops::Range<usize>),
    Done,
    Error(ParseError),
}

impl StreamParser for SseFramer {
    fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Vec<ParseEvent<'a>> {
        // Reset per-feed output buffer; previous feed's events have been
        // consumed (the borrow checker enforces this).
        self.output_buffer.clear();

        let mut emit: Vec<Emit> = Vec::new();
        if matches!(self.phase, Phase::Done | Phase::Error) {
            // No further events.
            return Vec::new();
        }

        for &b in bytes.iter() {
            if b == b'\n' {
                // End of line; process line_buf.
                process_line(&self.line_buf, &mut self.current_event);
                let line_was_blank = self.line_buf.is_empty()
                    || (self.line_buf.len() == 1 && self.line_buf[0] == b'\r');
                self.line_buf.clear();

                if line_was_blank && !self.current_event.is_empty() {
                    if self.current_event == b"[DONE]" {
                        emit.push(Emit::Done);
                        self.current_event.clear();
                        self.phase = Phase::Done;
                    } else {
                        let start = self.output_buffer.len();
                        self.output_buffer.extend_from_slice(&self.current_event);
                        let end = self.output_buffer.len();
                        emit.push(Emit::Token(start..end));
                        self.current_event.clear();
                    }
                }
                if matches!(self.phase, Phase::Done) {
                    break;
                }
            } else {
                self.line_buf.push(b);
                if self.line_buf.len() > 1024 * 1024 {
                    emit.push(Emit::Error(ParseError::MalformedSse {
                        reason: "single SSE line exceeded 1 MiB",
                    }));
                    self.phase = Phase::Error;
                    break;
                }
            }
        }

        let buf: &[u8] = &self.output_buffer;
        emit.into_iter()
            .map(|i| match i {
                Emit::Token(r) => ParseEvent::SseToken(&buf[r]),
                Emit::Done => ParseEvent::SseDone,
                Emit::Error(e) => ParseEvent::Error(e),
            })
            .collect()
    }

    fn reset(&mut self) {
        self.line_buf.clear();
        self.current_event.clear();
        self.output_buffer.clear();
        self.phase = Phase::Reading;
    }
}

/// Process one SSE line into the current-event payload.
///
/// SSE spec: lines starting with `data:` (with an optional single space
/// after the colon) contribute to the data field. Multiple data lines
/// concatenate with `\n` separators. Other line prefixes are skipped in
/// v0.1.
fn process_line(line: &[u8], current_event: &mut Vec<u8>) {
    // Strip trailing \r (CRLF tolerance).
    let line = if let Some(b) = line.last() {
        if *b == b'\r' {
            &line[..line.len() - 1]
        } else {
            line
        }
    } else {
        line
    };

    // Comment lines start with `:` per the SSE spec; skip.
    if line.first() == Some(&b':') {
        return;
    }
    // Skip event:, id:, retry: in v0.1 (we only consume data:).
    if line.starts_with(b"data:") {
        let rest = &line[5..];
        // Optional single leading space.
        let payload = if rest.first() == Some(&b' ') {
            &rest[1..]
        } else {
            rest
        };
        if !current_event.is_empty() {
            current_event.push(b'\n');
        }
        current_event.extend_from_slice(payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(bytes: &[u8]) -> (Vec<Vec<u8>>, bool, Option<ParseError>) {
        let mut f = SseFramer::new();
        let mut tokens: Vec<Vec<u8>> = Vec::new();
        let mut done = false;
        let mut err = None;
        for event in f.feed(bytes) {
            match event {
                ParseEvent::SseToken(t) => tokens.push(t.to_vec()),
                ParseEvent::SseDone => done = true,
                ParseEvent::Error(e) => err = Some(e),
                _ => {}
            }
        }
        (tokens, done, err)
    }

    #[test]
    fn parses_single_event() {
        let s = b"data: {\"x\":1}\n\n";
        let (tokens, done, err) = drive(s);
        assert!(err.is_none());
        assert!(!done);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], b"{\"x\":1}");
    }

    #[test]
    fn parses_multiple_events_with_done() {
        let s = b"data: a\n\ndata: b\n\ndata: [DONE]\n\n";
        let (tokens, done, err) = drive(s);
        assert!(err.is_none());
        assert!(done);
        assert_eq!(tokens, vec![b"a".to_vec(), b"b".to_vec()]);
    }

    #[test]
    fn skips_comment_and_metadata_lines() {
        let s = b": comment\nevent: foo\nid: 17\ndata: hi\n\n";
        let (tokens, _done, err) = drive(s);
        assert!(err.is_none());
        assert_eq!(tokens, vec![b"hi".to_vec()]);
    }

    #[test]
    fn handles_crlf_endings() {
        let s = b"data: hi\r\n\r\n";
        let (tokens, _done, err) = drive(s);
        assert!(err.is_none());
        assert_eq!(tokens, vec![b"hi".to_vec()]);
    }

    #[test]
    fn handles_split_feeds_byte_by_byte() {
        let s = b"data: alpha\n\ndata: beta\n\ndata: [DONE]\n\n";
        let mut f = SseFramer::new();
        let mut tokens: Vec<Vec<u8>> = Vec::new();
        let mut done = false;
        for byte in s.iter() {
            for event in f.feed(std::slice::from_ref(byte)) {
                match event {
                    ParseEvent::SseToken(t) => tokens.push(t.to_vec()),
                    ParseEvent::SseDone => done = true,
                    ParseEvent::Error(e) => panic!("unexpected error: {e:?}"),
                    _ => {}
                }
            }
        }
        assert!(done);
        assert_eq!(tokens, vec![b"alpha".to_vec(), b"beta".to_vec()]);
    }

    #[test]
    fn reset_clears_state() {
        let mut f = SseFramer::new();
        let _ = f.feed(b"data: hello\n\ndata: [DONE]\n\n");
        f.reset();
        let mut tokens: Vec<Vec<u8>> = Vec::new();
        for event in f.feed(b"data: world\n\n") {
            if let ParseEvent::SseToken(t) = event {
                tokens.push(t.to_vec());
            }
        }
        assert_eq!(tokens, vec![b"world".to_vec()]);
    }
}
