//! WAL frame parser — decodes raw bytes into typed frame records.
//!
//! Used by both the `riftgate-replay` CLI (`dump` subcommand) and the
//! fuzz harness (`crates/riftgate-replay/fuzz/`). Extracted here so
//! the fuzz target can reference it without depending on the binary.
//!
//! Frame layout (little-endian):
//! ```text
//! offset  0   1   2   3   4   5   6..13   14..21   22..22+len
//!       +---+---+---+---+---+---+-------+--------+-----------+
//!       | u32 payload_len | dur | knd | u64 ts_ns | u64 id | payload |
//!       +---+---+---+---+---+---+-------+--------+-----------+
//! ```
//! See [`crate::ENTRY_HEADER_BYTES`] (22).

use std::io::{self, Read};

use thiserror::Error;

/// A successfully parsed WAL frame.
#[derive(Debug, Clone)]
pub struct ParsedFrame {
    /// Monotonic entry id from the WAL.
    pub entry_id: u64,
    /// Wall-clock nanoseconds when the entry was appended.
    pub timestamp_nanos: u64,
    /// Durability tag (0 = Async, 1 = FdataSync, 2 = Fsync).
    pub durability_tag: u8,
    /// Entry-kind tag (0 = default; reserved for future schema versioning).
    pub entry_kind: u8,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Error produced by [`try_parse_frames`].
#[derive(Debug, Error)]
pub enum ParseFrameError {
    /// The payload length field declared a size that could not be read.
    #[error("payload truncated: declared {declared} bytes, got {got}")]
    PayloadTruncated {
        /// Declared length.
        declared: u32,
        /// Actual bytes available.
        got: usize,
    },
    /// IO error reading the frame.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// The declared payload length would overflow available memory.
    #[error("payload length overflow: {0}")]
    PayloadOverflow(u32),
}

/// Maximum supported payload length (16 MiB).
///
/// Prevents the fuzz target from allocating unbounded memory for large
/// `payload_len` values embedded in crafted input.
const MAX_PAYLOAD_BYTES: u32 = 16 * 1024 * 1024;

/// Parse as many complete frames as possible from `data`.
///
/// Returns one `Result` per frame attempted. A truncated final frame
/// (partial header or payload) is silently ignored rather than returned as
/// an error — this matches the WAL's crash-recovery behavior where the
/// last-written entry may be incomplete.
///
/// Never panics on arbitrary input.
pub fn try_parse_frames(data: &[u8]) -> Vec<Result<ParsedFrame, ParseFrameError>> {
    let mut cursor = io::Cursor::new(data);
    let mut results = Vec::new();

    loop {
        let mut header = [0u8; crate::ENTRY_HEADER_BYTES];
        match read_exact_or_eof(&mut cursor, &mut header) {
            Ok(true) => break, // clean EOF
            Ok(false) => {}
            Err(_) => break, // IO error (won't happen with Cursor, but be safe)
        }

        let payload_len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);

        if payload_len > MAX_PAYLOAD_BYTES {
            results.push(Err(ParseFrameError::PayloadOverflow(payload_len)));
            break;
        }

        let mut payload = vec![0u8; payload_len as usize];
        match cursor.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // Truncated final frame — stop silently.
                break;
            }
            Err(e) => {
                results.push(Err(ParseFrameError::Io(e)));
                break;
            }
        }

        results.push(Ok(ParsedFrame {
            entry_id: u64::from_le_bytes(header[14..22].try_into().unwrap()),
            timestamp_nanos: u64::from_le_bytes(header[6..14].try_into().unwrap()),
            durability_tag: header[4],
            entry_kind: header[5],
            payload,
        }));
    }

    results
}

/// Returns `Ok(true)` on clean EOF before reading anything, `Ok(false)` on
/// successful read, error on partial read.
fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<bool> {
    let mut bytes_read = 0;
    while bytes_read < buf.len() {
        match r.read(&mut buf[bytes_read..]) {
            Ok(0) => {
                return if bytes_read == 0 {
                    Ok(true) // clean EOF
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "partial header",
                    ))
                };
            }
            Ok(n) => bytes_read += n,
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(entry_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let payload_len = payload.len() as u32;
        out.extend_from_slice(&payload_len.to_le_bytes()); // length
        out.push(0u8); // durability = Async
        out.push(0u8); // entry_kind = default
        out.extend_from_slice(&0u64.to_le_bytes()); // timestamp_nanos
        out.extend_from_slice(&entry_id.to_le_bytes()); // entry_id
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn empty_input_returns_no_frames() {
        assert!(try_parse_frames(b"").is_empty());
    }

    #[test]
    fn single_frame_roundtrips() {
        let data = make_frame(42, b"hello world");
        let frames = try_parse_frames(&data);
        assert_eq!(frames.len(), 1);
        let f = frames[0].as_ref().unwrap();
        assert_eq!(f.entry_id, 42);
        assert_eq!(f.payload, b"hello world");
    }

    #[test]
    fn two_frames_parse_correctly() {
        let mut data = make_frame(1, b"first");
        data.extend(make_frame(2, b"second"));
        let frames = try_parse_frames(&data);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_ref().unwrap().entry_id, 1);
        assert_eq!(frames[1].as_ref().unwrap().entry_id, 2);
    }

    #[test]
    fn truncated_payload_is_silently_ignored() {
        let mut data = make_frame(1, b"complete");
        // Add a header that declares 100 bytes but only provide 5.
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 18]); // rest of header
        data.extend_from_slice(&[0u8; 5]); // only 5 of 100 payload bytes
        let frames = try_parse_frames(&data);
        // Only the first (complete) frame should be returned.
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn oversized_payload_returns_error() {
        let oversized_len = MAX_PAYLOAD_BYTES + 1;
        let mut data = Vec::new();
        data.extend_from_slice(&oversized_len.to_le_bytes());
        data.extend_from_slice(&[0u8; 18]); // rest of header
        let frames = try_parse_frames(&data);
        assert_eq!(frames.len(), 1);
        assert!(matches!(
            frames[0],
            Err(ParseFrameError::PayloadOverflow(_))
        ));
    }

    #[test]
    fn arbitrary_junk_never_panics() {
        let _ = try_parse_frames(b"\xff\xff\xff\xff");
        let _ = try_parse_frames(&[0u8; 100]);
        let _ = try_parse_frames(b"\x01\x00\x00\x00"); // declares 1 byte, EOF
    }
}
