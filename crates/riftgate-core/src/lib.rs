//! # riftgate-core
//!
//! Kernel trait surface and shared types for every Riftgate subsystem.
//!
//! This crate is the **pluggability seam** of the project. Every load-bearing
//! subsystem of the data plane and extension plane is defined here as a
//! trait; concrete implementations live in dedicated crates (or, for tiny
//! universal impls like `BumpArena`, here alongside the trait).
//!
//! ## Trait surface at a glance
//!
//! ```text
//!   client_bytes ---> [AsyncIO::poll] ---> [StreamParser::feed]
//!                                                |
//!                                                v
//!         [Allocator::alloc] (per request)   ParseEvent stream
//!                                                |
//!                                                v
//!                                       [Queue<Task>::push]    \
//!                                                |             |  per-shard
//!                                                v             |
//!                                  [Scheduler::run worker]    /
//!                                                |
//!                                                v
//!                                         [Filter::on_request]
//!                                                |
//!                                                v
//!                                         [Router::route]
//!                                                |
//!                                                v
//!                                  upstream backend (HTTP+SSE)
//!                                                |
//!                                                v
//!                                         [Filter::on_response]
//!                                                |
//!                                                v
//!                                            client SSE
//!
//!   In parallel, on every checkpoint:
//!     [TimerSubsystem::tick] enforces deadlines
//!     [ObservabilitySink::publish] emits typed events
//!     [Allocator::reset] frees per-request memory in O(1)
//!
//!   Deferred (no v0.1 impl, trait shape only):
//!     [RateLimiter::check]      (ships in v0.2 per ADR 0009)
//!     [WAL::append]             (ships in v0.2)
//!     [CapabilityBroker::check] (ships in v0.5 per ADR 0015)
//! ```
//!
//! ## Discipline
//!
//! - Every trait either has at least two implementations, or has a documented
//!   reason for one (`FR-X02`). The reason for the three deferred-impl traits
//!   is the corresponding milestone gate.
//! - Public items are documented (`#![warn(missing_docs)]`).
//! - This crate carries `#![deny(unsafe_code)]`. The `bumpalo` dependency
//!   encapsulates the unsafe a per-request arena needs; we never write raw
//!   `unsafe` in `riftgate-core`.
//! - Trait shapes are the kernel contract. Changing a public trait requires
//!   a new ADR per [`AGENTS.md` §5](../../../AGENTS.md).
//!
//! ## Module layout
//!
//! - [`types`] — basic identifier types (`RequestId`, `BackendId`, ...).
//! - [`request`] — `Request`, `Response`, `Outcome`, `Body`, `Headers`,
//!   `Method`, `StatusCode`.
//! - [`io`] — `AsyncIO`, `Interest`, `Event`.
//! - [`parser`] — `StreamParser`, `ParseEvent`, `ParseError`.
//! - [`scheduler`] — `Scheduler`, `Task`.
//! - [`queue`] — `Queue<T>`, `CrossbeamMpmcQueue` (added in Phase H).
//! - [`allocator`] — `Allocator`, `SystemAllocator`, `BumpArena`.
//! - [`timers`] — `TimerSubsystem`, `BinaryHeapTimers`, `DeterministicTimers`,
//!   `TimerHandle`.
//! - [`filter`] — `Filter`, `FilterAction`, `IdentityFilter`,
//!   `LoggingFilter`.
//! - [`router`] — `Router`, `BackendPool`, `BackendSignal`, `BackendSignals`,
//!   `RoutingDecision`, `CircuitState`.
//! - [`obs`] — `ObservabilitySink`, `ObservabilityEvent`, `Labels`,
//!   `Attributes`, `InMemorySink`.
//! - [`rate_limit`] — trait shape only; impl deferred to `v0.2`.
//! - [`wal`] — trait shape only; impl deferred to `v0.2`.
//! - [`capability`] — trait shape only; impl deferred to `v0.5`.
//! - [`error`] — `RiftgateCoreError`, the shared error type.

#![doc(html_root_url = "https://docs.rs/riftgate-core/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod allocator;
pub mod backpressure;
pub mod cancel;
pub mod capability;
pub mod error;
pub mod filter;
pub mod gpu;
pub mod io;
pub mod obs;
pub mod parser;
pub mod queue;
pub mod rate_limit;
pub mod request;
pub mod router;
pub mod scheduler;
pub mod timers;
pub mod types;
pub mod wal;

// Convenience re-exports: the most-used types from any caller.
pub use allocator::{Allocator, BumpArena, SystemAllocator};
pub use backpressure::{AdmissionDecision, BackpressurePolicy, DenialReason, HighWaterPolicy};
pub use cancel::{CancelCause, Cancellation, CancellationDriver};
pub use error::RiftgateCoreError;
pub use filter::{Filter, FilterAction, IdentityFilter, LoggingFilter};
pub use gpu::{
    GpuPressure, GpuPressureError, GpuPressureSource, GpuThrottleState, NoopGpuSource,
    StaticGpuSource,
};
pub use io::{AsyncIO, Event, Interest};
pub use obs::{InMemorySink, Labels, ObservabilityEvent, ObservabilitySink};
pub use parser::{ParseError, ParseEvent, StreamParser};
pub use queue::Queue;
pub use rate_limit::{LimitDecision, RateLimiter, SubjectKey};
pub use request::{Body, Headers, Method, Request, Response, StatusCode};
pub use router::{
    BackendId, BackendPool, BackendSignal, BackendSignals, CircuitState, Outcome, Router,
    RoutingDecision,
};
pub use scheduler::{Scheduler, Task};
pub use timers::{BinaryHeapTimers, DeterministicTimers, TimerHandle, TimerSubsystem};
pub use types::{RequestId, RouteId, ShardId, TenantId};
