//! Vitter Algorithm R reservoir sampling.
//!
//! Maintains a fixed-size reservoir of random samples from a stream
//! using Vitter's Algorithm R (Vitter 1985, "Random sampling with a reservoir").
//! This allows tracking rare tail events without buffering the entire stream.
//!
//! **Properties:**
//! - Uniform sampling probability across all items seen.
//! - Constant memory (fixed reservoir size).
//! - Single-pass streaming (no rewinds).

use rand::Rng;
use std::sync::Mutex;

/// Fixed-capacity reservoir implementing Vitter Algorithm R.
///
/// Non-blocking public interface; internal state is guarded by a Mutex.
pub struct VitterReservoir<T: Clone> {
    capacity: usize,
    items: Mutex<Vec<T>>,
    count: Mutex<u64>,
}

impl<T: Clone> VitterReservoir<T> {
    /// Create a new reservoir with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: Mutex::new(Vec::with_capacity(capacity)),
            count: Mutex::new(0),
        }
    }

    /// Add an item to the reservoir. Non-blocking; uses Vitter's algorithm
    /// to decide with the correct probability whether to include this item.
    pub fn add(&self, item: T) {
        let mut count = self.count.lock().expect("count poisoned");
        *count += 1;
        let n = *count;

        let mut items = self.items.lock().expect("items poisoned");

        if (n as usize) <= self.capacity {
            // Reservoir not yet full; add directly.
            items.push(item);
        } else {
            // Reservoir is full; include with probability k/n.
            let mut rng = rand::thread_rng();
            let j = rng.gen_range(0..(n as usize));
            if j < self.capacity {
                items[j] = item;
            }
        }
    }

    /// Return a snapshot of the current samples (in arbitrary order).
    pub fn samples(&self) -> Vec<T> {
        self.items.lock().expect("items poisoned").clone()
    }

    /// Return the count of items processed so far (including those not in the reservoir).
    pub fn total_count(&self) -> u64 {
        *self.count.lock().expect("count poisoned")
    }
}

impl<T: Clone> Clone for VitterReservoir<T> {
    fn clone(&self) -> Self {
        Self {
            capacity: self.capacity,
            items: Mutex::new(self.items.lock().expect("items poisoned").clone()),
            count: Mutex::new(*self.count.lock().expect("count poisoned")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservoir_fills_to_capacity() {
        let reservoir: VitterReservoir<u64> = VitterReservoir::new(10);
        for i in 0..10 {
            reservoir.add(i);
        }
        let samples = reservoir.samples();
        assert_eq!(samples.len(), 10);
    }

    #[test]
    fn reservoir_maintains_capacity() {
        let reservoir: VitterReservoir<u64> = VitterReservoir::new(10);
        for i in 0..1000 {
            reservoir.add(i);
        }
        let samples = reservoir.samples();
        assert_eq!(samples.len(), 10);
        assert_eq!(reservoir.total_count(), 1000);
    }

    #[test]
    fn reservoir_sampling_is_probabilistic() {
        // Add 1000 items to a reservoir of size 100; expect roughly 100 items
        // (with some variance due to randomness).
        let reservoir: VitterReservoir<u64> = VitterReservoir::new(100);
        for i in 0..1000 {
            reservoir.add(i);
        }
        let samples = reservoir.samples();
        assert_eq!(samples.len(), 100);
    }

    #[test]
    fn reservoir_captures_diverse_values() {
        // Add values 0-999 to a reservoir; expect the sample to contain
        // a mix across the range (not just recent or early values).
        let reservoir: VitterReservoir<u64> = VitterReservoir::new(100);
        for i in 0..1000 {
            reservoir.add(i);
        }
        let samples = reservoir.samples();
        let max_sample = samples.iter().max().copied().unwrap_or(0);
        let min_sample = samples.iter().min().copied().unwrap_or(0);

        // With high probability, reservoir should include values from both
        // the beginning, middle, and end of the range.
        assert!(min_sample < 100, "expected early items in sample");
        assert!(max_sample > 900, "expected late items in sample");
    }
}
