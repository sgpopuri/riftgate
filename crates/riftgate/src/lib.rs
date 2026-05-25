//! # riftgate (library surface)
//!
//! This is the *library* entry point of the v0.1 Riftgate binary. The
//! actual binary, `src/main.rs`, is a thin wrapper around
//! [`run_with_args`] / [`run`] so that integration tests can construct
//! the same handler stack that the binary builds and exercise it
//! against a mock upstream.
//!
//! See the [crate README](https://github.com/sgpopuri/riftgate/tree/main/crates/riftgate)
//! for the architecture diagram and the FR-coverage table.

#![doc(html_root_url = "https://docs.rs/riftgate/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod bootstrap;
pub mod error;
pub mod health;
pub mod proxy;
pub mod server;
pub mod shutdown;
pub mod upstream;

pub use error::RiftgateError;
