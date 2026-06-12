//! # riftgate-obs
//!
//! Observability bus + sinks for Riftgate `v0.1`.
//!
//! See the [crate README](https://github.com/sgpopuri/riftgate/tree/main/crates/riftgate-obs)
//! for the architecture diagram and rationale links.
//!
//! ## Public API
//!
//! - [`Bus`] — owns the bounded MPSC and a worker. The data plane
//!   acquires a [`Publisher`] via [`Bus::publisher`] and calls
//!   [`Publisher::publish`].
//! - [`MultiSink`] — fan-out helper that itself implements
//!   `ObservabilitySink`.
//! - [`OtelSink`], [`JsonStdoutSink`] — concrete sinks shipped in `v0.1`.
//! - [`spans`] — canonical span-name constants per
//!   [`FR-006`](../../../docs/01-requirements/functional.md).
//!
//! Drop-on-full discipline lives in [`Publisher::publish`] and increments
//! `riftgate_observability_dropped_total`. Operators surface the counter
//! via the same sink as everything else (no special path).

#![doc(html_root_url = "https://docs.rs/riftgate-obs/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod bpf;
mod bus;
pub mod gpu;
mod json_stdout_sink;
mod multi_sink;
mod otel_sink;
pub mod token_level;

pub mod spans;

pub use bpf::{BpfRuntimeState, BpfSink, RIFTGATE_ENABLE_BPF_ENV};
pub use bus::{Bus, Publisher};
pub use gpu::DcgmScrapeSource;
pub use json_stdout_sink::JsonStdoutSink;
pub use multi_sink::MultiSink;
pub use otel_sink::OtelSink;
