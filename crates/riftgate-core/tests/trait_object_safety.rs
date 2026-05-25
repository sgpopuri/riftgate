//! Compile-time check that every kernel trait that is intended to be used
//! as a trait object is in fact dyn-safe.
//!
//! These tests do not assert behavior; they only verify that the
//! `Box<dyn Trait>` construction compiles. A regression here means the
//! trait shape changed in a way that breaks pluggability — which requires
//! a new ADR per [`AGENTS.md` §5](../../../AGENTS.md).

use riftgate_core::allocator::{Allocator, BumpArena, SystemAllocator};
use riftgate_core::capability::CapabilityBroker;
use riftgate_core::filter::{Filter, IdentityFilter, LoggingFilter};
use riftgate_core::io::AsyncIO;
use riftgate_core::obs::{InMemorySink, ObservabilitySink};
use riftgate_core::queue::Queue;
use riftgate_core::rate_limit::RateLimiter;
use riftgate_core::router::Router;
use riftgate_core::scheduler::Scheduler;
use riftgate_core::timers::{BinaryHeapTimers, DeterministicTimers, TimerSubsystem};
use riftgate_core::wal::WAL;

#[test]
fn allocator_is_dyn_safe() {
    let _a: Box<dyn Allocator> = Box::new(SystemAllocator::new());
    let _b: Box<dyn Allocator> = Box::new(BumpArena::new());
}

#[test]
fn timer_subsystem_is_dyn_safe() {
    let _t: Box<dyn TimerSubsystem> = Box::new(BinaryHeapTimers::new());
    let _u: Box<dyn TimerSubsystem> = Box::new(DeterministicTimers::new());
}

#[test]
fn filter_is_dyn_safe() {
    let _f: Box<dyn Filter> = Box::new(IdentityFilter::new());
    let _g: Box<dyn Filter> = Box::new(LoggingFilter::new());
}

#[test]
fn obs_sink_is_dyn_safe() {
    let _s: Box<dyn ObservabilitySink> = Box::new(InMemorySink::new());
}

#[test]
fn async_io_router_scheduler_queue_dyn_safety_compiles() {
    // These are dyn-safe but we don't have an in-core impl for each. The
    // `compile-time only` checks below assert the trait *shape* permits
    // dyn dispatch; impls live in their own crates.
    fn _async_io_box(_b: Box<dyn AsyncIO>) {}
    fn _router_box(_b: Box<dyn Router>) {}
    fn _scheduler_box(_b: Box<dyn Scheduler>) {}
    fn _queue_box(_b: Box<dyn Queue<u32>>) {}
}

#[test]
fn rate_limiter_wal_capability_dyn_safety_compiles() {
    fn _rl_box(_b: Box<dyn RateLimiter>) {}
    fn _wal_box(_b: Box<dyn WAL>) {}
    fn _cb_box(_b: Box<dyn CapabilityBroker>) {}
}
