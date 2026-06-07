//! # riftgate-filter
//!
//! v0.3 filter chain executor + WASM filter runtime.
//!
//! Per [ADR 0019](../../../docs/06-adrs/0019-wasm-extension-mechanism.md)
//! and [Options 016](../../../docs/05-options/016-extension-mechanism.md):
//!
//! - The [`FilterChain`] executor is a thin sequence of
//!   [`riftgate_core::Filter`] implementations. It runs filters in *order*
//!   on the request side and in *reverse order* on the response side, per
//!   the canonical filter-chain shape (Envoy, Linkerd, Spin).
//! - The [`WasmFilter`] type binds the frozen `riftgate:filter/v1`
//!   Component Model ABI behind the `wasm` cargo feature.
//!   `WasmFilter::try_load` performs component loading + validation;
//!   `WasmFilter::scaffold` is still available for identity behavior when
//!   operators intentionally do not load a component.
//!
//! ## Filter ordering
//!
//! ```text
//!   inbound:                outbound:
//!   filter[0].on_request    filter[N-1].on_response
//!   filter[1].on_request    ...
//!   ...                     filter[1].on_response
//!   filter[N-1].on_request  filter[0].on_response
//! ```

#![doc(html_root_url = "https://docs.rs/riftgate-filter/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod chain;
mod wasm;

pub use chain::FilterChain;
pub use wasm::{WasmFilter, WasmFilterConfig};
