//! # riftgate-router
//!
//! Routing implementations behind the [`Router`](riftgate_core::router::Router)
//! trait.
//!
//! - [`RoundRobinRouter`] — `v0.1` default. Atomic-cursor over the
//!   [`BackendPool`](riftgate_core::router::BackendPool); fair distribution
//!   without a lock.
//! - [`ConstantRouter`] — `v0.1` test impl (FR-X02 second impl).
//!
//! Future impls (weighted, KV-aware, hedged) land in `v0.2`+ behind the
//! same trait.
//!
//! See [`docs/04-design/lld-routing.md`](../../../docs/04-design/lld-routing.md)
//! and [Options 010](../../../docs/05-options/README.md) (to be authored
//! at the open of `v0.2`).

#![doc(html_root_url = "https://docs.rs/riftgate-router/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod circuit;
mod constant;
mod round_robin;
mod weighted;

pub use circuit::{CircuitBreakerArbiter, CircuitBreakerConfig};
pub use constant::ConstantRouter;
pub use round_robin::RoundRobinRouter;
pub use weighted::{MAX_WEIGHTED_BACKENDS, WeightedRandomRouter};
