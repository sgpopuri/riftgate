//! Lock-free MPMC queue trait.
//!
//! Per [ADR 0005](../../../docs/06-adrs/0005-sharded-mpmc-queue.md) the v0.1
//! production impl wraps `crossbeam-channel`; that wrapper lives in the
//! `riftgate` binary crate alongside the scheduler. This module declares the
//! trait and a `#[cfg(test)] MutexQueue<T>` second impl for FR-X02
//! compliance and for unit-testing scheduler-adjacent code without spinning
//! up a real MPMC.

/// MPMC queue trait.
///
/// Items can be pushed and popped from any thread. Implementations are
/// expected to be lock-free on the fast path; the trait does not require it
/// because the test impl below uses a mutex for simplicity.
///
/// **Generic in `T`.** Trait objects are constructible per concrete `T`
/// (`Box<dyn Queue<MyTask>>`); the trait does not have associated types so
/// it remains dyn-safe per concrete `T`.
pub trait Queue<T>: Send + Sync {
    /// Push an item onto the queue.
    ///
    /// On success returns `Ok(())`. On full returns `Err(item)` so the
    /// caller can retry, drop, or surface backpressure without losing the
    /// item.
    ///
    /// # Errors
    /// Returns the rejected item back to the caller when the queue cannot
    /// accept more entries (bounded queue at capacity, or shut-down sender).
    fn push(&self, item: T) -> Result<(), T>;

    /// Pop an item from the queue, or `None` if empty.
    fn pop(&self) -> Option<T>;

    /// Approximate number of items in the queue.
    ///
    /// Lock-free implementations return a snapshot that may already be
    /// stale by the time the caller reads it; this is fine for backpressure
    /// signalling but unsuitable for correctness logic.
    fn len(&self) -> usize;

    /// `true` if the queue is empty (snapshot; see `len` caveat).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
pub(crate) mod test_impl {
    //! Mutex-backed VecDeque for tests.
    //!
    //! Not lock-free; not the v0.1 default. Provided to satisfy FR-X02 and
    //! to give scheduler unit tests a `Queue<T>` they can construct without
    //! pulling in `crossbeam-channel`.

    use super::Queue;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    pub(crate) struct MutexQueue<T> {
        inner: Mutex<VecDeque<T>>,
        capacity: Option<usize>,
    }

    impl<T> MutexQueue<T> {
        pub(crate) fn unbounded() -> Self {
            Self {
                inner: Mutex::new(VecDeque::new()),
                capacity: None,
            }
        }
        pub(crate) fn bounded(capacity: usize) -> Self {
            Self {
                inner: Mutex::new(VecDeque::with_capacity(capacity)),
                capacity: Some(capacity),
            }
        }
    }

    impl<T: Send> Queue<T> for MutexQueue<T> {
        fn push(&self, item: T) -> Result<(), T> {
            let mut g = self.inner.lock().expect("MutexQueue poisoned");
            if let Some(cap) = self.capacity {
                if g.len() >= cap {
                    return Err(item);
                }
            }
            g.push_back(item);
            Ok(())
        }
        fn pop(&self) -> Option<T> {
            self.inner.lock().expect("MutexQueue poisoned").pop_front()
        }
        fn len(&self) -> usize {
            self.inner.lock().expect("MutexQueue poisoned").len()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bounded_rejects_when_full() {
            let q = MutexQueue::<u32>::bounded(2);
            assert!(q.push(1).is_ok());
            assert!(q.push(2).is_ok());
            assert_eq!(q.push(3), Err(3));
            assert_eq!(q.pop(), Some(1));
            assert!(q.push(4).is_ok());
        }

        #[test]
        fn unbounded_grows() {
            let q = MutexQueue::<u32>::unbounded();
            for i in 0..1000 {
                q.push(i).unwrap();
            }
            assert_eq!(q.len(), 1000);
        }
    }
}
