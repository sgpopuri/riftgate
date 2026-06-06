//! # riftgate-filter
//!
//! v0.3 filter chain executor + scaffold for the WASM filter ABI.
//!
//! Per [ADR 0019](../../../docs/06-adrs/0019-wasm-extension-mechanism.md)
//! and [Options 016](../../../docs/05-options/016-extension-mechanism.md):
//!
//! - The [`FilterChain`] executor is a thin sequence of
//!   [`riftgate_core::Filter`] implementations. It runs filters in *order*
//!   on the request side and in *reverse order* on the response side, per
//!   the canonical filter-chain shape (Envoy, Linkerd, Spin).
//! - The [`WasmFilter`] type is a scaffold for the production WASM-component
//!   impl that binds the frozen `riftgate:filter/v1` Component Model ABI.
//!   The wasmtime runtime, the WIT bindings, AOT precompile, instance
//!   pooling, fuel / memory / wallclock limits, and the host-function table
//!   land in a follow-on implementation PR. The scaffold here lets every consumer
//!   compile against the public type surface today.
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
