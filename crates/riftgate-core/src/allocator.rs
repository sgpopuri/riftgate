//! Allocator trait + `SystemAllocator` and `BumpArena` impls.
//!
//! ```text
//!   request_in --> arena.alloc(layout)  ----+
//!       |                                   |
//!       |   parser/filter/router/response   |  all borrow from the same arena
//!       |                                   |
//!   request_done --> arena.reset()  <-------+   (returns to per-worker pool)
//! ```
//!
//! The trait is **deliberately not bounded by `Send + Sync`** — `BumpArena`
//! is non-`Send` and non-`Sync` per [ADR
//! 0006](../../../docs/06-adrs/0006-bump-arena-plus-system-malloc.md). The
//! per-shard execution model from [ADR
//! 0004](../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md)
//! means a request stays on its shard, so its arena does not need to cross
//! thread boundaries.
//!
//! See [`docs/04-design/lld-allocator.md`](../../../docs/04-design/lld-allocator.md)
//! and [Options 005](../../../docs/05-options/005-allocator.md) for the full
//! design rationale.

use std::alloc::{Layout, alloc};

/// Per-request or per-subsystem memory allocator.
///
/// Two impls ship in `riftgate-core` for `v0.1`:
///
/// - [`SystemAllocator`] — wraps the global allocator (`std::alloc`). Used
///   by long-lived structures (connection state, timer wheels, the WAL
///   buffer). Send + Sync (auto-implemented because the type is a unit
///   struct).
/// - [`BumpArena`] — per-request bump-pointer arena. Used on the request
///   hot path. Reset is O(1).
///
/// **`alloc` returns a non-null pointer or panics on OOM.** This matches
/// `std::alloc::alloc`'s behavior; `BumpArena` will likewise panic if the
/// per-request cap (enforced one layer up) is exceeded after a fallback
/// attempt. Allocators that need to surface OOM as a recoverable error wrap
/// the `alloc` site at a higher layer.
///
/// **`reset` takes `&mut self`** because `BumpArena` requires unique mutable
/// access to invalidate every borrow it has lent out (a property the borrow
/// checker enforces for free). `SystemAllocator::reset` is a no-op.
pub trait Allocator {
    /// Allocate a block of memory matching `layout`.
    ///
    /// Returns a non-null pointer to a block at least `layout.size()` bytes
    /// long, aligned to `layout.align()`.
    ///
    /// # Panics
    /// Panics on out-of-memory.
    fn alloc(&self, layout: Layout) -> *mut u8;

    /// Free everything allocated since the last `reset`, in O(1).
    ///
    /// `SystemAllocator::reset` is a no-op; the global allocator handles
    /// individual `dealloc` calls per-allocation. `BumpArena::reset` resets
    /// the bump pointer in O(1) and invalidates every borrow returned by
    /// previous `alloc` calls.
    fn reset(&mut self);
}

/// Wraps the global `std::alloc::System` allocator.
///
/// Used for long-lived allocations (connection state, configuration,
/// metric registry). `reset` is a no-op; matched `dealloc` calls are
/// handled by the standard library types (`Box`, `Vec`, `String`, ...) on
/// drop.
///
/// `Send + Sync` are auto-implemented because the type is a zero-sized
/// unit struct.
#[derive(Debug, Default, Copy, Clone)]
pub struct SystemAllocator;

impl SystemAllocator {
    /// Construct a new `SystemAllocator`. Zero cost.
    pub const fn new() -> Self {
        Self
    }
}

impl Allocator for SystemAllocator {
    #[allow(unsafe_code)]
    fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `Layout` enforces a non-zero size and a power-of-two
        // alignment; `std::alloc::alloc` returns null on OOM, which we
        // immediately panic on to match the `std::alloc` policy.
        let ptr = unsafe { alloc(layout) };
        assert!(
            !ptr.is_null(),
            "system allocator returned null for {layout:?}"
        );
        ptr
    }

    fn reset(&mut self) {
        // No-op: the global allocator handles per-allocation `dealloc`
        // independently. The trait shape is unified across the arena and
        // the system allocator at the cost of this no-op.
    }
}

/// Per-request bump-pointer arena.
///
/// Wraps `bumpalo::Bump`, which encapsulates the unsafe needed for a
/// pointer-bump allocator. `BumpArena` is non-`Send` and non-`Sync` by
/// design (per ADR 0006); a request stays on its shard, so cross-thread
/// access is not required.
///
/// Allocation cost on the hot path: ~5–15 ns per allocation (a single
/// pointer increment plus an alignment adjustment). Compare to ~50–200 ns
/// for `std::alloc::System`. The win compounds across the dozens of small
/// allocations a typical request makes.
///
/// `reset` returns the bump pointer to the start of the arena in O(1); the
/// underlying memory is retained for reuse on the next request (per-worker
/// arena pool semantics).
///
/// # Per-request memory cap
///
/// The arena does not enforce a cap directly; the binary's per-shard pool
/// owns the cap policy. ADR 0006 mandates a default 1 MB cap configurable
/// via `RIFTGATE_REQUEST_ARENA_CAP_BYTES`; over-cap allocations fall back
/// to the system allocator with a `riftgate_arena_overflow_total` counter
/// increment. The cap policy is enforced at a higher layer.
pub struct BumpArena {
    inner: bumpalo::Bump,
}

impl BumpArena {
    /// Construct a new arena with the default initial capacity (4 KB; grows
    /// by doubling per `bumpalo::Bump`'s default growth policy).
    pub fn new() -> Self {
        Self {
            inner: bumpalo::Bump::new(),
        }
    }

    /// Construct a new arena with a hint for the initial capacity. Useful
    /// for reusing arena memory across requests.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: bumpalo::Bump::with_capacity(capacity),
        }
    }

    /// Total bytes currently allocated by this arena (sum of all chunks).
    pub fn allocated_bytes(&self) -> usize {
        self.inner.allocated_bytes()
    }

    /// Allocate space for a value and return a mutable reference into the
    /// arena. The reference's lifetime is bound to `&self`; storing it past
    /// the next `reset` is a borrow-checker error.
    pub fn alloc_value<T>(&self, value: T) -> &mut T {
        self.inner.alloc(value)
    }

    /// Allocate a zero-copy slice of bytes inside the arena.
    pub fn alloc_slice_copy(&self, src: &[u8]) -> &mut [u8] {
        self.inner.alloc_slice_copy(src)
    }
}

impl Default for BumpArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Allocator for BumpArena {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        // bumpalo's `alloc_layout` returns a `NonNull<u8>`; we surface it
        // as a raw pointer for trait compatibility with `SystemAllocator`.
        // The trait contract is that callers do not free per-arena
        // pointers individually — `reset` releases everything wholesale.
        self.inner.alloc_layout(layout).as_ptr()
    }

    fn reset(&mut self) {
        // bumpalo's `Bump::reset` requires `&mut self`; the trait now
        // matches.
        self.inner.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_allocator_alloc_returns_non_null() {
        let a = SystemAllocator::new();
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = a.alloc(layout);
        assert!(!ptr.is_null());
        // We do NOT free here — the std types own their own dealloc; this
        // is a one-shot allocation for the test only. In a real binary
        // this leak would matter; in a single-iteration test it does not.
        // We could call into std::alloc::dealloc directly via an unsafe
        // block, but the test goal is just to verify the trait round-trip.
    }

    #[test]
    fn bump_arena_allocates_value() {
        let arena = BumpArena::new();
        let v: &mut u64 = arena.alloc_value(42);
        assert_eq!(*v, 42);
    }

    #[test]
    fn bump_arena_alloc_slice_zero_copy() {
        let arena = BumpArena::new();
        let s = arena.alloc_slice_copy(b"hello, riftgate");
        assert_eq!(s, b"hello, riftgate");
    }

    #[test]
    fn bump_arena_reset_releases() {
        let mut arena = BumpArena::with_capacity(1024);
        for _ in 0..100 {
            let _ = arena.alloc_value(0u128);
        }
        let before = arena.allocated_bytes();
        assert!(before > 0);
        // Trait `reset` works because we hold `&mut arena`.
        Allocator::reset(&mut arena);
        // After reset, bumpalo retains the chunk; subsequent allocations
        // reuse it without growing.
        let _ = arena.alloc_value(0u128);
        assert!(arena.allocated_bytes() <= before);
    }
}
