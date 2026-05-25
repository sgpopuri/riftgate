//! Timer subsystem trait + `BinaryHeapTimers` and `DeterministicTimers` impls.
//!
//! Per [ADR 0010](../../../docs/06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md)
//! the v0.1 default is `BinaryHeapTimers`: a `std::collections::BinaryHeap`
//! of `(deadline, timer_id)` with lazy cancellation via a `HashSet<TimerId>`
//! of cancelled ids. The hierarchical wheel lands in v0.2 behind the same
//! trait.
//!
//! ```text
//!   schedule(deadline, callback) ----> push (Reverse((deadline, id))) onto heap
//!                                      insert callback into HashMap
//!                                      ---> TimerHandle(id)
//!
//!   cancel(handle)              ----> remove callback from HashMap (if present)
//!                                      insert id into cancelled HashSet
//!                                      ---> bool (true iff pre-empted firing)
//!
//!   tick(now)                   ----> while heap.peek().deadline <= now:
//!                                        pop (deadline, id)
//!                                        if cancelled.remove(id) -> skip
//!                                        else if callback present -> fire
//!                                      ---> ()
//!
//!   compaction                  ----> when cancelled.len() > 25% of heap.len():
//!                                        rebuild heap, dropping cancelled ids
//! ```
//!
//! See [`docs/04-design/lld-timers.md`](../../../docs/04-design/lld-timers.md)
//! for the full design rationale and pitfalls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::Instant;

/// Opaque handle for a scheduled timer.
///
/// Returned by [`TimerSubsystem::schedule`]; passed to
/// [`TimerSubsystem::cancel`] to cancel the timer before it fires.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct TimerHandle(pub u64);

/// Per-shard timer subsystem.
///
/// **Not bounded by `Send + Sync`.** Per-shard ownership ([ADR 0004](../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md))
/// means each shard's `TimerSubsystem` instance is private to that shard's
/// worker; cross-thread access is not needed.
///
/// Methods take `&mut self` because the in-core `BinaryHeapTimers`
/// modifies its heap and cancelled set on every call. This deviates from
/// the `&self` signature in the v0.0 outline-stage LLD; the LLD is updated
/// in Phase J of the v0.1 plan to match.
///
/// Trait object safety: yes (no generics, no associated types).
pub trait TimerSubsystem {
    /// Schedule `on_fire` to be called when `tick(now)` is called with
    /// `now >= deadline`.
    ///
    /// Returns a [`TimerHandle`] that can be passed to [`Self::cancel`] to
    /// pre-empt the firing.
    ///
    /// O(log n) for the binary-heap impl; O(1) for the hierarchical-wheel
    /// impl in `v0.2`.
    fn schedule(&mut self, deadline: Instant, on_fire: Box<dyn FnOnce() + Send>) -> TimerHandle;

    /// Cancel a previously-scheduled timer.
    ///
    /// Returns `true` if cancellation pre-empted the firing; `false` if the
    /// timer had already fired or had already been cancelled (idempotent).
    ///
    /// O(1) average for the binary-heap impl (lazy deletion via the
    /// cancelled set); O(1) for the hierarchical-wheel impl.
    fn cancel(&mut self, handle: TimerHandle) -> bool;

    /// Process all timers whose deadlines have passed.
    ///
    /// Called once per resolution unit (10 ms by default per the LLD).
    /// Cost is O(k log n) for the binary-heap impl, where `k` is the
    /// number of expired timers; O(1) amortized for the hierarchical-wheel
    /// impl modulo cascade bursts.
    fn tick(&mut self, now: Instant);

    /// Number of live (not-yet-fired, not-cancelled) timers.
    fn len(&self) -> usize;

    /// `true` if there are no live timers.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Deadline of the next timer to fire, or `None` if no timers are
    /// scheduled.
    ///
    /// Used by the per-shard worker loop to compute the next IO poll
    /// timeout (`AsyncIO::poll(timeout = next_deadline - now)`).
    fn next_deadline(&self) -> Option<Instant>;
}

/// Binary-heap timer subsystem with lazy cancellation.
///
/// The default `v0.1` impl per [ADR 0010](../../../docs/06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md).
///
/// State per timer: one heap entry (`Reverse((Instant, TimerId))`) plus one
/// `HashMap` entry (id -> callback). Cancellation inserts the id into the
/// cancelled `HashSet`; the actual heap entry is dropped at the next pop.
///
/// **Compaction policy:** when the cancelled set's size exceeds
/// `compaction_ratio * heap.len()` (default 0.25), the heap is rebuilt,
/// dropping all cancelled-but-not-yet-popped entries. The compaction cost
/// is O(n + n log n) (rebuild) but is amortized.
pub struct BinaryHeapTimers {
    heap: BinaryHeap<Reverse<(Instant, u64)>>,
    callbacks: HashMap<u64, Box<dyn FnOnce() + Send>>,
    cancelled: HashSet<u64>,
    next_id: u64,
    compaction_ratio: f32,
}

impl BinaryHeapTimers {
    /// Construct a new `BinaryHeapTimers` with the default compaction ratio
    /// (25%).
    pub fn new() -> Self {
        Self::with_compaction_ratio(0.25)
    }

    /// Construct with a custom compaction ratio.
    ///
    /// `compaction_ratio` is the cancelled-set size relative to the heap
    /// size at which a rebuild is triggered. Smaller values mean more
    /// frequent (cheaper) rebuilds; larger values trade memory for fewer
    /// rebuilds. Values outside `(0.0, 1.0]` are clamped.
    pub fn with_compaction_ratio(compaction_ratio: f32) -> Self {
        Self {
            heap: BinaryHeap::new(),
            callbacks: HashMap::new(),
            cancelled: HashSet::new(),
            next_id: 1,
            compaction_ratio: compaction_ratio.clamp(0.001, 1.0),
        }
    }

    /// Number of cancelled-but-not-yet-popped entries currently sitting
    /// inside the heap. The metric `riftgate_timers_cancelled_pending` is
    /// derived from this.
    pub fn cancelled_pending(&self) -> usize {
        self.cancelled.len()
    }

    fn maybe_compact(&mut self) {
        let live = self.heap.len();
        if live == 0 {
            return;
        }
        let threshold = (live as f32 * self.compaction_ratio) as usize;
        if self.cancelled.len() > threshold {
            self.compact();
        }
    }

    fn compact(&mut self) {
        // Rebuild the heap, dropping cancelled entries. Visits every entry
        // once; the cancelled set is cleared at the end.
        let mut kept: Vec<Reverse<(Instant, u64)>> = Vec::with_capacity(self.heap.len());
        while let Some(item) = self.heap.pop() {
            let Reverse((_, id)) = item;
            if !self.cancelled.contains(&id) {
                kept.push(item);
            }
        }
        self.heap = BinaryHeap::from(kept);
        self.cancelled.clear();
    }
}

impl Default for BinaryHeapTimers {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerSubsystem for BinaryHeapTimers {
    fn schedule(&mut self, deadline: Instant, on_fire: Box<dyn FnOnce() + Send>) -> TimerHandle {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.heap.push(Reverse((deadline, id)));
        self.callbacks.insert(id, on_fire);
        TimerHandle(id)
    }

    fn cancel(&mut self, handle: TimerHandle) -> bool {
        let removed = self.callbacks.remove(&handle.0).is_some();
        if removed {
            self.cancelled.insert(handle.0);
            self.maybe_compact();
        }
        removed
    }

    fn tick(&mut self, now: Instant) {
        while let Some(Reverse((deadline, id))) = self.heap.peek().copied() {
            if deadline > now {
                break;
            }
            self.heap.pop();
            if self.cancelled.remove(&id) {
                continue;
            }
            if let Some(cb) = self.callbacks.remove(&id) {
                cb();
            }
        }
    }

    fn len(&self) -> usize {
        // Active timers = total scheduled (callbacks present) minus cancelled.
        // The HashMap holds exactly the active set.
        self.callbacks.len()
    }

    fn next_deadline(&self) -> Option<Instant> {
        // Skip past the cancelled prefix to find the next live entry.
        // O(k) in the worst case where k is the number of cancelled-at-top
        // entries; in steady state k is small because compaction runs.
        for entry in &self.heap {
            let Reverse((deadline, id)) = *entry;
            if !self.cancelled.contains(&id) {
                return Some(deadline);
            }
        }
        None
    }
}

/// Manual-clock timer subsystem for unit tests.
///
/// `tick(now)` advances the simulated clock; `schedule` and `cancel` behave
/// the same as the heap impl. Lets deadline-sensitive code be unit-tested
/// without sleeping. FR-X02 second impl alongside [`BinaryHeapTimers`].
pub struct DeterministicTimers {
    inner: BinaryHeapTimers,
    last_tick: Option<Instant>,
}

impl DeterministicTimers {
    /// Construct a new `DeterministicTimers`.
    pub fn new() -> Self {
        Self {
            inner: BinaryHeapTimers::new(),
            last_tick: None,
        }
    }

    /// The most recent `now` passed to `tick`, or `None` if `tick` has not
    /// been called.
    pub fn last_tick(&self) -> Option<Instant> {
        self.last_tick
    }
}

impl Default for DeterministicTimers {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerSubsystem for DeterministicTimers {
    fn schedule(&mut self, deadline: Instant, on_fire: Box<dyn FnOnce() + Send>) -> TimerHandle {
        self.inner.schedule(deadline, on_fire)
    }
    fn cancel(&mut self, handle: TimerHandle) -> bool {
        self.inner.cancel(handle)
    }
    fn tick(&mut self, now: Instant) {
        self.last_tick = Some(now);
        self.inner.tick(now);
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn next_deadline(&self) -> Option<Instant> {
        self.inner.next_deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn fired_counter() -> (Arc<AtomicUsize>, Box<dyn FnOnce() + Send>) {
        let c = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&c);
        let cb: Box<dyn FnOnce() + Send> = Box::new(move || {
            c2.fetch_add(1, Ordering::SeqCst);
        });
        (c, cb)
    }

    #[test]
    fn fires_in_deadline_order() {
        let mut t = BinaryHeapTimers::new();
        let now = Instant::now();
        let (c1, cb1) = fired_counter();
        let (c2, cb2) = fired_counter();
        t.schedule(now + Duration::from_millis(20), cb2);
        t.schedule(now + Duration::from_millis(10), cb1);
        t.tick(now + Duration::from_millis(15));
        assert_eq!(c1.load(Ordering::SeqCst), 1);
        assert_eq!(c2.load(Ordering::SeqCst), 0);
        t.tick(now + Duration::from_millis(25));
        assert_eq!(c2.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_pre_empts_fire() {
        let mut t = BinaryHeapTimers::new();
        let now = Instant::now();
        let (c, cb) = fired_counter();
        let h = t.schedule(now + Duration::from_millis(10), cb);
        assert!(t.cancel(h));
        t.tick(now + Duration::from_millis(20));
        assert_eq!(c.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancel_is_idempotent() {
        let mut t = BinaryHeapTimers::new();
        let now = Instant::now();
        let (_c, cb) = fired_counter();
        let h = t.schedule(now + Duration::from_millis(10), cb);
        assert!(t.cancel(h));
        assert!(!t.cancel(h));
        assert!(!t.cancel(TimerHandle(99999)));
    }

    #[test]
    fn next_deadline_skips_cancelled() {
        let mut t = BinaryHeapTimers::new();
        let now = Instant::now();
        let (_c1, cb1) = fired_counter();
        let (_c2, cb2) = fired_counter();
        let h1 = t.schedule(now + Duration::from_millis(10), cb1);
        let d2 = now + Duration::from_millis(20);
        t.schedule(d2, cb2);
        assert_eq!(t.next_deadline(), Some(now + Duration::from_millis(10)));
        t.cancel(h1);
        assert_eq!(t.next_deadline(), Some(d2));
    }

    #[test]
    fn compaction_drains_cancelled_set() {
        // Schedule many, cancel most: compaction should fire and drain the
        // cancelled set below the threshold.
        let mut t = BinaryHeapTimers::with_compaction_ratio(0.25);
        let now = Instant::now();
        let mut handles = Vec::new();
        for i in 0..100 {
            let (_c, cb) = fired_counter();
            handles.push(t.schedule(now + Duration::from_millis(i + 1), cb));
        }
        for h in handles.iter().take(50) {
            assert!(t.cancel(*h));
        }
        // After cancelling 50 of 100, the heap holds 100 raw entries but
        // 50 are cancelled; compaction should have drained them. After
        // compaction the cancelled set is empty.
        assert!(
            t.cancelled_pending() < 25,
            "expected compaction to drain cancelled set, got {}",
            t.cancelled_pending()
        );
    }

    #[test]
    fn deterministic_timers_records_last_tick() {
        let mut t = DeterministicTimers::new();
        let now = Instant::now();
        t.tick(now);
        assert_eq!(t.last_tick(), Some(now));
        let later = now + Duration::from_millis(50);
        t.tick(later);
        assert_eq!(t.last_tick(), Some(later));
    }

    #[test]
    fn timer_subsystem_is_dyn_safe() {
        let mut t: Box<dyn TimerSubsystem> = Box::new(BinaryHeapTimers::new());
        let (_c, cb) = fired_counter();
        let _ = t.schedule(Instant::now(), cb);
    }
}
