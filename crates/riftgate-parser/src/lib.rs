//! # riftgate-parser
//!
//! Two [`StreamParser`](riftgate_core::parser::StreamParser) impls for the
//! v0.1 walking skeleton:
//!
//! - [`Http1Parser`] — parses HTTP/1.1 requests. Headers via `httparse`,
//!   body via a hand-rolled `Content-Length` FSM (chunked encoding lands
//!   in v0.2 per [ADR 0007](../../../docs/06-adrs/0007-handrolled-fsm-parser.md)).
//! - [`SseFramer`] — parses Server-Sent Events streams.
//!
//! ```text
//!   raw bytes  --feed-->  [Http1Parser FSM]  --emit-->  HeadersComplete
//!                                                       BodyChunk
//!                                                       BodyComplete
//!
//!   raw bytes  --feed-->  [SseFramer FSM]    --emit-->  SseToken
//!                                                       SseDone
//! ```
//!
//! Both impls follow the same shape: they buffer bytes in a `scratch`
//! `Vec<u8>` across feed calls, drive an explicit state machine over the
//! buffered bytes, and emit owned [`Headers`](riftgate_core::request::Headers)
//! plus body-byte slices borrowed back from the scratch buffer (with the
//! returned-events-borrow-from-`&mut self` lifetime that the trait
//! contract permits).

#![doc(html_root_url = "https://docs.rs/riftgate-parser/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod http1;
mod sse;

pub use http1::Http1Parser;
pub use sse::SseFramer;
