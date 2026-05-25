//! # riftgate-router
//!
//! Routing implementations behind the [`Router`](riftgate_core::router::Router)
//! trait.
//!
//! - [`RoundRobinRouter`] — `v0.1` default. Atomic-cursor over the
//!   [`BackendPool`](riftgate_core::router::BackendPool); fair distribution
//!   without a lock.
//! - [`ConstantRouter`] — `v0.1` test impl (FR-X02 second impl).
//! - [`WeightedRandomRouter`] — `v0.2` (ADR 0014). Walker alias method.
//! - [`CircuitBreakerArbiter`] — `v0.2` (ADR 0016). 3-state breaker
//!   decorator.
//! - [`KvAwareRouter`] — `v0.3` (ADR 0022). Prefix-trie KV-cache-aware
//!   decorator over an inner router.
//! - [`HedgedRouter`] — `v0.3` (ADR 0023). Threshold-triggered (Dean &
//!   Barroso) hedged-request decorator, degree=2.
//!
//! See [`docs/04-design/lld-routing.md`](../../../docs/04-design/lld-routing.md)
//! and [Options 010 / 025](../../../docs/05-options/README.md).

#![doc(html_root_url = "https://docs.rs/riftgate-router/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod circuit;
mod constant;
mod hedged;
mod kv_aware;
mod round_robin;
mod weighted;

pub use circuit::{CircuitBreakerArbiter, CircuitBreakerConfig};
pub use constant::ConstantRouter;
pub use hedged::{HedgeStats, HedgedConfig, HedgedRouter};
pub use kv_aware::{KvAwareConfig, KvAwareRouter, KvAwareStats};
pub use round_robin::RoundRobinRouter;
pub use weighted::{MAX_WEIGHTED_BACKENDS, WeightedRandomRouter};
