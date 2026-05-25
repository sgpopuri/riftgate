//! Request, response, and supporting HTTP types.
//!
//! Deliberately minimal: the v0.1 walking skeleton needs just enough to
//! describe an OpenAI-format chat-completions call and its SSE response.
//! Richer types (multipart, trailers, HTTP/2 streams) land as they are needed
//! by the relevant Options docs and ADRs.
//!
//! ```text
//!   wire bytes  --[StreamParser::feed]-->  ParseEvent::HeadersComplete(Headers)
//!                                          ParseEvent::BodyChunk(&[u8])
//!                                          ParseEvent::BodyComplete
//!                                                |
//!                                                v
//!                                  Request { method, path, headers, body }
//!                                                |
//!                                                v
//!                                  [Filter::on_request] --> [Router::route]
//!                                                                |
//!                                                                v
//!                                                  upstream Response
//! ```

use crate::types::RequestId;

/// HTTP method.
///
/// Only the methods Riftgate actually parses today. `Other` carries the raw
/// bytes for forward compatibility; the parser does not normalize case.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `DELETE`
    Delete,
    /// `OPTIONS`
    Options,
    /// `HEAD`
    Head,
    /// Any other method, preserved verbatim.
    Other(String),
}

/// HTTP status code (numeric).
///
/// Wraps a `u16` so a `StatusCode` cannot be confused with any other 16-bit
/// value at a call site.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct StatusCode(pub u16);

impl StatusCode {
    /// `200 OK`.
    pub const OK: Self = Self(200);
    /// `400 Bad Request`.
    pub const BAD_REQUEST: Self = Self(400);
    /// `401 Unauthorized`.
    pub const UNAUTHORIZED: Self = Self(401);
    /// `403 Forbidden`.
    pub const FORBIDDEN: Self = Self(403);
    /// `404 Not Found`.
    pub const NOT_FOUND: Self = Self(404);
    /// `429 Too Many Requests`. Used by the v0.2 rate limiter.
    pub const TOO_MANY_REQUESTS: Self = Self(429);
    /// `500 Internal Server Error`.
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    /// `502 Bad Gateway`. Returned when an upstream backend is unreachable
    /// or returns a malformed response.
    pub const BAD_GATEWAY: Self = Self(502);
    /// `503 Service Unavailable`. Returned by the v0.2 backpressure policy
    /// when the local queue depth exceeds the configured high-water mark.
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    /// `504 Gateway Timeout`. Returned when a deadline fires before the
    /// upstream responds.
    pub const GATEWAY_TIMEOUT: Self = Self(504);

    /// Returns `true` if the status code is in the 2xx success class.
    #[inline]
    pub fn is_success(self) -> bool {
        (200..300).contains(&self.0)
    }

    /// Returns `true` if the status code is in the 4xx client-error class.
    #[inline]
    pub fn is_client_error(self) -> bool {
        (400..500).contains(&self.0)
    }

    /// Returns `true` if the status code is in the 5xx server-error class.
    #[inline]
    pub fn is_server_error(self) -> bool {
        (500..600).contains(&self.0)
    }
}

/// HTTP headers as a small ordered list of `(name, value)` pairs.
///
/// Names are stored in their original case but compared case-insensitively
/// via the `get` method. The list is small (typical real-world request
/// carries 10–20 headers); a `HashMap` would over-pay on the hot path.
///
/// Header values are stored as `Vec<u8>` rather than `String` because the
/// HTTP spec permits non-UTF-8 bytes in some headers (`Set-Cookie` in
/// pathological cases); upgrading to `String` only at the call site that
/// needs it keeps the parser path zero-copy where possible.
#[derive(Debug, Default, Clone)]
pub struct Headers {
    entries: Vec<(String, Vec<u8>)>,
}

impl Headers {
    /// Construct an empty `Headers`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a header. Duplicates are preserved; HTTP allows them
    /// (most-significant for `Set-Cookie`).
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<Vec<u8>>) {
        self.entries.push((name.into(), value.into()));
    }

    /// Return the first value of the named header, case-insensitively.
    ///
    /// O(n) over the header list; n is small in practice.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
    }

    /// Number of header entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if there are no header entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries in their original insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

/// Request or response body.
///
/// `Bytes` carries an in-memory body suitable for non-streaming requests
/// (e.g. an OpenAI chat-completions JSON body that is well under 1 MB in
/// practice). Streaming bodies are represented at a higher level — a
/// `Body::Bytes` is what the v0.1 walking skeleton consumes and emits.
#[derive(Debug, Clone)]
pub enum Body {
    /// Empty body (no `Content-Length` or `Content-Length: 0`).
    Empty,
    /// Bounded byte sequence loaded into memory.
    Bytes(Vec<u8>),
}

impl Body {
    /// Number of bytes in the body, or 0 for `Empty`.
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Bytes(b) => b.len(),
        }
    }

    /// `true` if the body has no content.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parsed inbound request.
///
/// Carries the request id (assigned by the accept loop), method, path,
/// headers, and body. The body is owned; in `v0.2`+ a per-request `BumpArena`
/// owns the underlying allocation for the lifetime of the request.
#[derive(Debug, Clone)]
pub struct Request {
    /// Per-request identifier; matches the id used in OTel spans and logs.
    pub id: RequestId,
    /// HTTP method.
    pub method: Method,
    /// Request path, including any query string. Not URL-decoded.
    pub path: String,
    /// HTTP headers in original wire order.
    pub headers: Headers,
    /// Request body.
    pub body: Body,
}

/// Outbound response.
#[derive(Debug, Clone)]
pub struct Response {
    /// Per-request identifier of the request this response answers.
    pub id: RequestId,
    /// HTTP status code.
    pub status: StatusCode,
    /// Response headers.
    pub headers: Headers,
    /// Response body. For streaming responses this is empty at construction
    /// time; chunks are emitted via the SSE framer.
    pub body: Body,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_classes() {
        assert!(StatusCode::OK.is_success());
        assert!(StatusCode::BAD_REQUEST.is_client_error());
        assert!(StatusCode::SERVICE_UNAVAILABLE.is_server_error());
    }

    #[test]
    fn headers_get_is_case_insensitive() {
        let mut h = Headers::new();
        h.insert("Content-Type", b"application/json".to_vec());
        assert_eq!(h.get("content-type"), Some(&b"application/json"[..]));
        assert_eq!(h.get("CONTENT-TYPE"), Some(&b"application/json"[..]));
        assert_eq!(h.get("missing"), None);
    }

    #[test]
    fn headers_preserves_duplicates() {
        let mut h = Headers::new();
        h.insert("Set-Cookie", b"a=1".to_vec());
        h.insert("Set-Cookie", b"b=2".to_vec());
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn body_len_and_is_empty() {
        assert!(Body::Empty.is_empty());
        assert_eq!(Body::Empty.len(), 0);
        let b = Body::Bytes(vec![1, 2, 3]);
        assert_eq!(b.len(), 3);
        assert!(!b.is_empty());
    }
}
