//! v0.2 scheduler and queue production impls.
//!
//! The trait surface lives in
//! [`riftgate_core::scheduler`](../../../riftgate-core/src/scheduler.rs) and
//! [`riftgate_core::queue`](../../../riftgate-core/src/queue.rs); both modules
//! declare that the production impls live alongside the binary's runtime
//! wiring, so they land here.
//!
//! This module ships three pieces, all governed by existing ADRs:
//!
//! - [`MpmcQueue<T>`] — a thin [`crossbeam_channel::bounded`] wrapper that
//!   satisfies the [`Queue<T>`] trait. Per
//!   [ADR 0005](../../../../docs/06-adrs/0005-sharded-mpmc-queue.md) the v0.1
//!   queue substrate is a vetted MPMC channel rather than a hand-rolled
//!   lock-free queue.
//! - [`ShardedMpmcQueue<T>`] — N `MpmcQueue<T>` shards plus an
//!   atomic-cursor producer fan-out. Each consumer is pinned to one shard,
//!   matching the per-shard scheduler ownership model from
//!   [ADR 0004](../../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md).
//!   A `pop_or_steal` helper provides an optional non-blocking probe across
//!   sibling shards; the binary's v0.2 default does not enable it, but the
//!   API is the seam a future `--features work-stealing` flips on.
//! - [`PerShardScheduler`] — implements
//!   [`riftgate_core::scheduler::Scheduler`]. One bounded shard queue per
//!   worker; `submit` fans tasks across shards via round-robin atomic
//!   cursor; `run` is the per-shard worker loop draining its queue.
//!
//! The v0.2 binary keeps the tokio multi-thread runtime as the default per
//! [ADR 0003](../../../../docs/06-adrs/0003-tokio-multithread-default.md);
//! `PerShardScheduler` is constructed and exercised by tests and the
//! `--features per-core-scheduler` migration path.
//!
//! ## Data layout
//!
//! ```text
//!                  ShardedMpmcQueue<Task>
//!                  +--------------------------------------------------+
//!  producers  --> | cursor: AtomicUsize (round-robin fan-out)         |
//!  (many)         +--------------------------------------------------+
//!                 | shards: Vec<MpmcQueue<Task>>                      |
//!                 |  +--------+  +--------+  +--------+   +--------+ |
//!                 |  | shard0 |  | shard1 |  | shard2 |...| shardN | |
//!                 |  |  cap C |  |  cap C |  |  cap C |   |  cap C | |
//!                 |  +---^----+  +---^----+  +---^----+   +---^----+ |
//!                 +------|----------|-----------|------------|-------+
//!                        |          |           |            |
//!                   worker_0    worker_1   worker_2   ... worker_N
//!                  (pinned to    (pinned)     (pinned)      (pinned)
//!                   shard0)
//! ```
//!
//! Each worker thread owns exactly one shard for the lifetime of the
//! scheduler. `pop_or_steal` exists as the seam for opting into work
//! stealing later; v0.2 default `run` uses `pop_from(idx)` only.
//!
//! ## Lifecycle
//!
//! ```text
//!  PerShardScheduler::new(N, C, poll_idle)
//!         |
//!         v
//!  spawn N worker threads ---->  loop {
//!                                   match queue.pop_from(idx) {
//!                                       Some(task) => task();
//!                                       None       => park(poll_idle);
//!                                   }
//!                                   if shutdown.load() && empty -> break
//!                                }
//!
//!  Scheduler::submit(task)
//!         |
//!         v
//!  cursor++ % N --> push_inner on chosen shard
//!                       on full -> walk shards once, else return Err(task)
//!                       (caller then runs the backpressure policy)
//!
//!  PerShardScheduler::shutdown()
//!         |
//!         v
//!  shutdown.store(true) --> workers drain remaining tasks, then join
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use riftgate_core::queue::Queue;
use riftgate_core::scheduler::{Scheduler, Task};

/// Bounded MPMC queue backed by a vetted `crossbeam-channel` pair.
///
/// `push` is non-blocking: returns `Err(item)` when the queue is full so the
/// caller (typically a backpressure policy) can choose to drop, reject, or
/// retry. `pop` is non-blocking and returns `None` when the queue is empty.
pub struct MpmcQueue<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
}

impl<T> MpmcQueue<T> {
    /// Construct a new bounded MPMC queue with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity == 0` — a zero-capacity rendezvous channel has
    /// different semantics than the bounded MPMC the scheduler expects and
    /// is almost always a configuration bug.
    #[must_use]
    pub fn bounded(capacity: usize) -> Self {
        assert!(capacity > 0, "MpmcQueue capacity must be > 0");
        let (tx, rx) = crossbeam_channel::bounded(capacity);
        Self { tx, rx }
    }

    /// Push without requiring the caller to import `Queue`.
    ///
    /// # Errors
    /// Returns the item back when the queue is at capacity or all
    /// receivers have disconnected.
    pub fn push_inner(&self, item: T) -> Result<(), T> {
        match self.tx.try_send(item) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(item)) | Err(TrySendError::Disconnected(item)) => Err(item),
        }
    }

    /// Pop without requiring the caller to import `Queue`.
    pub fn pop_inner(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Approximate snapshot of the queue depth.
    #[must_use]
    pub fn len_inner(&self) -> usize {
        self.rx.len()
    }
}

impl<T: Send> Queue<T> for MpmcQueue<T> {
    fn push(&self, item: T) -> Result<(), T> {
        self.push_inner(item)
    }

    fn pop(&self) -> Option<T> {
        self.pop_inner()
    }

    fn len(&self) -> usize {
        self.len_inner()
    }
}

/// N-shard MPMC queue with a round-robin producer cursor.
///
/// Each consumer owns one shard index and only pops from that shard; this is
/// the per-shard ownership invariant from
/// [ADR 0004](../../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md).
/// Producers spread across shards via [`Self::push`], which advances an
/// atomic cursor each call. [`Self::pop_or_steal`] gives a single attempt to
/// steal from sibling shards when the owned shard is empty; the v0.2 default
/// path does not call it.
pub struct ShardedMpmcQueue<T> {
    shards: Vec<MpmcQueue<T>>,
    cursor: AtomicUsize,
}

impl<T> ShardedMpmcQueue<T> {
    /// Build an N-shard queue. Each shard is `per_shard_capacity` deep.
    ///
    /// # Panics
    /// Panics if `shards == 0` or `per_shard_capacity == 0`.
    #[must_use]
    pub fn new(shards: usize, per_shard_capacity: usize) -> Self {
        assert!(shards > 0, "ShardedMpmcQueue requires at least one shard");
        Self {
            shards: (0..shards)
                .map(|_| MpmcQueue::bounded(per_shard_capacity))
                .collect(),
            cursor: AtomicUsize::new(0),
        }
    }

    /// Number of shards.
    #[must_use]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// Depth of shard `idx`. Snapshot; may be stale.
    #[must_use]
    pub fn shard_len(&self, idx: usize) -> usize {
        self.shards[idx].len_inner()
    }

    /// Pop from shard `idx` only. Used by consumers pinned to that shard.
    #[must_use]
    pub fn pop_from(&self, idx: usize) -> Option<T> {
        self.shards[idx].pop_inner()
    }

    /// Pop from shard `idx`; if empty, attempt one non-blocking probe at
    /// every other shard in ascending order.
    ///
    /// Returns `None` only if every shard is empty at the moment of probe.
    /// This is the seam a future `--features work-stealing` flips on; it is
    /// not called by the default v0.2 worker loop.
    #[must_use]
    pub fn pop_or_steal(&self, idx: usize) -> Option<T> {
        if let Some(item) = self.shards[idx].pop_inner() {
            return Some(item);
        }
        let n = self.shards.len();
        for offset in 1..n {
            let victim = (idx + offset) % n;
            if let Some(item) = self.shards[victim].pop_inner() {
                return Some(item);
            }
        }
        None
    }
}

impl<T: Send> Queue<T> for ShardedMpmcQueue<T> {
    fn push(&self, item: T) -> Result<(), T> {
        // Round-robin over shards. Try each shard at most once before
        // surfacing backpressure; this matches the drop-newest discipline
        // from ADR 0017.
        let n = self.shards.len();
        let start = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
        let mut current = item;
        for offset in 0..n {
            let target = (start + offset) % n;
            match self.shards[target].push_inner(current) {
                Ok(()) => return Ok(()),
                Err(returned) => current = returned,
            }
        }
        Err(current)
    }

    fn pop(&self) -> Option<T> {
        // Generic pop: linear scan across shards. Pinned consumers should
        // prefer `pop_from(idx)`; this exists so the trait is total.
        for shard in &self.shards {
            if let Some(item) = shard.pop_inner() {
                return Some(item);
            }
        }
        None
    }

    fn len(&self) -> usize {
        self.shards.iter().map(MpmcQueue::len_inner).sum()
    }
}

/// Per-shard scheduler: N worker threads, one bounded shard queue each.
///
/// Tasks land on a shard via [`Scheduler::submit`] (round-robin atomic
/// cursor). Each worker thread is spawned in [`Self::start`] and drains its
/// shard via [`ShardedMpmcQueue::pop_from`]. Shutdown is signalled by
/// [`Self::shutdown`]: workers observe the atomic flag, drain their shard,
/// and exit.
///
/// This impl deliberately does not pin to a CPU. CPU pinning is a Linux-only
/// concern that belongs in the runtime-wiring layer when the v0.2 binary
/// flips the `--features per-core-scheduler` migration default.
pub struct PerShardScheduler {
    queue: Arc<ShardedMpmcQueue<Task>>,
    handles: std::sync::Mutex<Vec<thread::JoinHandle<()>>>,
    shutdown: Arc<AtomicBool>,
    poll_idle: Duration,
}

impl PerShardScheduler {
    /// Construct a scheduler with `shards` worker threads, each owning a
    /// shard of `per_shard_capacity` tasks. Workers are spawned eagerly.
    ///
    /// # Panics
    /// Panics if `shards == 0` or `per_shard_capacity == 0`.
    #[must_use]
    pub fn start(shards: usize, per_shard_capacity: usize) -> Arc<Self> {
        let queue = Arc::new(ShardedMpmcQueue::<Task>::new(shards, per_shard_capacity));
        let shutdown = Arc::new(AtomicBool::new(false));
        let me = Arc::new(Self {
            queue: Arc::clone(&queue),
            handles: std::sync::Mutex::new(Vec::with_capacity(shards)),
            shutdown: Arc::clone(&shutdown),
            poll_idle: Duration::from_micros(100),
        });
        let mut handles = me
            .handles
            .lock()
            .expect("PerShardScheduler handles poisoned");
        for shard_idx in 0..shards {
            let queue = Arc::clone(&queue);
            let shutdown = Arc::clone(&shutdown);
            let poll_idle = me.poll_idle;
            let handle = thread::Builder::new()
                .name(format!("riftgate-shard-{shard_idx:02}"))
                .spawn(move || worker_loop(shard_idx, &queue, &shutdown, poll_idle))
                .expect("riftgate-shard thread spawn");
            handles.push(handle);
        }
        drop(handles);
        me
    }

    /// Number of worker shards.
    #[must_use]
    pub fn shard_count(&self) -> usize {
        self.queue.shard_count()
    }

    /// Signal workers to drain and exit, then join them. Idempotent.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let mut handles = self
            .handles
            .lock()
            .expect("PerShardScheduler handles poisoned");
        for h in handles.drain(..) {
            let _ = h.join();
        }
    }

    /// Borrow the underlying queue (for tests and metrics).
    #[must_use]
    pub fn queue(&self) -> &Arc<ShardedMpmcQueue<Task>> {
        &self.queue
    }
}

impl Scheduler for PerShardScheduler {
    fn submit(&self, task: Task) {
        // If every shard is full, drop on the floor. Production callers
        // wrap submit in a `BackpressurePolicy::on_enqueue` check that
        // turns this into a `503 + Retry-After` per ADR 0017 before reaching
        // the scheduler. The scheduler itself is not the place to encode
        // that policy.
        let _ = Queue::push(&*self.queue, task);
    }

    fn run(&self) {
        // `Scheduler::run` is the worker-loop hook for impls that expose
        // the per-shard loop to the caller. `PerShardScheduler` owns its
        // worker threads; `run` is a no-op for compatibility.
    }
}

impl Drop for PerShardScheduler {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut handles) = self.handles.lock() {
            for h in handles.drain(..) {
                let _ = h.join();
            }
        }
    }
}

fn worker_loop(
    shard_idx: usize,
    queue: &Arc<ShardedMpmcQueue<Task>>,
    shutdown: &Arc<AtomicBool>,
    poll_idle: Duration,
) {
    loop {
        if let Some(task) = queue.pop_from(shard_idx) {
            task();
            continue;
        }
        if shutdown.load(Ordering::Acquire) {
            // Drain remainder, then exit.
            while let Some(task) = queue.pop_from(shard_idx) {
                task();
            }
            return;
        }
        thread::sleep(poll_idle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    #[test]
    fn mpmc_queue_push_pop_roundtrip() {
        let q: MpmcQueue<u32> = MpmcQueue::bounded(4);
        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn mpmc_queue_returns_item_when_full() {
        let q: MpmcQueue<u32> = MpmcQueue::bounded(2);
        q.push(1).unwrap();
        q.push(2).unwrap();
        assert_eq!(q.push(3), Err(3));
    }

    #[test]
    fn sharded_queue_distributes_across_shards() {
        let q: ShardedMpmcQueue<u32> = ShardedMpmcQueue::new(4, 8);
        for i in 0..16 {
            Queue::push(&q, i).unwrap();
        }
        // Each shard should have received roughly 16/4 = 4 items.
        for shard in 0..4 {
            assert_eq!(q.shard_len(shard), 4, "shard {shard} length");
        }
    }

    #[test]
    fn sharded_queue_pop_or_steal_finds_work_in_sibling() {
        let q: ShardedMpmcQueue<u32> = ShardedMpmcQueue::new(4, 4);
        // Force everything onto shard 1 by pushing to that shard directly.
        q.shards[1].push_inner(42).unwrap();
        // Owner of shard 0 finds nothing locally but should steal from
        // shard 1.
        assert_eq!(q.pop_or_steal(0), Some(42));
        assert_eq!(q.pop_or_steal(0), None);
    }

    #[test]
    fn sharded_queue_push_surfaces_backpressure_when_full() {
        let q: ShardedMpmcQueue<u32> = ShardedMpmcQueue::new(2, 1);
        Queue::push(&q, 1).unwrap();
        Queue::push(&q, 2).unwrap();
        // Both shards are full. Next push must return the item.
        assert_eq!(Queue::push(&q, 3), Err(3));
    }

    #[test]
    fn per_shard_scheduler_runs_submitted_tasks() {
        let sched = PerShardScheduler::start(2, 16);
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..32 {
            let c = Arc::clone(&counter);
            sched.submit(Box::new(move || {
                c.fetch_add(1, Ordering::Relaxed);
            }));
        }
        // Wait for the workers to drain. 32 trivial closures × 2 shards
        // completes in well under a second on any developer machine; if it
        // does not, the test should fail loudly rather than hang.
        let deadline = Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::Relaxed) < 32 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(counter.load(Ordering::Relaxed), 32);
        sched.shutdown();
    }

    #[test]
    fn per_shard_scheduler_drains_on_shutdown() {
        let sched = PerShardScheduler::start(2, 64);
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let c = Arc::clone(&counter);
            sched.submit(Box::new(move || {
                c.fetch_add(1, Ordering::Relaxed);
            }));
        }
        sched.shutdown();
        // After shutdown the worker threads have joined; all 50 closures
        // ran exactly once. Re-shutdown is idempotent.
        assert_eq!(counter.load(Ordering::Relaxed), 50);
        sched.shutdown();
    }
}
