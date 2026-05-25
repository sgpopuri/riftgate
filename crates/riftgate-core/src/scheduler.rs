//! Scheduler trait and the `Task` shape.
//!
//! Concrete scheduler impls (`PerShardScheduler` in `v0.1`,
//! `WorkStealingScheduler` opt-in in `v0.2`) live in the `riftgate` binary
//! crate where the runtime is wired up. This module declares the trait so
//! filter / router / observability code can be written against it without
//! depending on the binary.
//!
//! See [`docs/04-design/lld-scheduling.md`](../../../docs/04-design/lld-scheduling.md)
//! for the per-shard ownership rationale and [ADR
//! 0004](../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md) for
//! the v0.1 sharding decision.

/// Unit of work submitted to the scheduler.
///
/// In the v0.1 shape, a `Task` is the closure that drives one request from
/// "parsed" to "responded." Tasks are non-`Sync` (per-shard execution; one
/// shard runs the closure to completion) and `Send` (the closure may move
/// across Tokio worker threads when work-stealing lands in `v0.2`).
pub type Task = Box<dyn FnOnce() + Send + 'static>;

/// Scheduler trait.
///
/// One scheduler instance is shared by the accept loop and the worker
/// shards. The accept loop calls `submit`; each shard's worker loop calls
/// `run`.
///
/// **Trait object safety.** The trait is dyn-safe.
pub trait Scheduler: Send + Sync {
    /// Submit a task to the scheduler.
    ///
    /// In the per-shard impl, the task is enqueued on the destination
    /// shard's MPMC queue. The selection of which shard receives the task
    /// is the impl's concern (round-robin, hash-based, work-stealing).
    fn submit(&self, task: Task);

    /// Worker loop entry point.
    ///
    /// Called once per worker shard. The function returns when the
    /// scheduler is shut down. Implementations are expected to drain the
    /// shard's queue, run timers, and yield to the IO subsystem in a tight
    /// loop.
    fn run(&self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Tiny in-memory scheduler for unit tests.
    ///
    /// Counts submissions; never runs them. Sufficient for verifying that
    /// a caller routes work to the trait.
    struct CountingScheduler(Arc<AtomicUsize>);

    impl Scheduler for CountingScheduler {
        fn submit(&self, _task: Task) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn run(&self) { /* no-op for tests */
        }
    }

    #[test]
    fn scheduler_is_dyn_safe() {
        let counter = Arc::new(AtomicUsize::new(0));
        let s: Box<dyn Scheduler> = Box::new(CountingScheduler(Arc::clone(&counter)));
        s.submit(Box::new(|| {}));
        s.submit(Box::new(|| {}));
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }
}
