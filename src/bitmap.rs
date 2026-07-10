//! Top-level `RoaringBitmap` plus `split`/`join` value-model helpers and datasets.

use crate::container::Container;
use std::cmp::Ordering;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

/// Split a `u32` into its container key (high 16 bits) and low part (low 16 bits).
pub fn split(x: u32) -> (u16, u16) {
    ((x >> 16) as u16, x as u16)
}

/// Inverse of [`split`]: recombine a key and low part into a `u32`.
pub fn join(key: u16, low: u16) -> u32 {
    ((key as u32) << 16) | low as u32
}

/// Deterministic benchmark datasets, shared by the criterion bench, the scaling binary, and tests.
/// `#[doc(hidden) pub]` so benches (which cannot import from `tests/`) can reach them while they
/// stay out of the public API surface. Seeds are pinned so every run measures the same data.
#[doc(hidden)]
pub mod datasets {
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};

    /// `0..1_000_000` — contiguous, so long runs / full bitmap containers.
    pub fn dense() -> Vec<u32> {
        (0..1_000_000).collect()
    }

    /// 1,000,000 uniform random draws (duplicates permitted). ~15 values per key → array containers.
    pub fn sparse() -> Vec<u32> {
        // Pinned seed: identical input to every impl is what makes the comparison fair.
        let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF);
        (0..1_000_000).map(|_| rng.random::<u32>()).collect()
    }

    /// 1,000 random bases, each contributing a length-1000 contiguous span → run/array mixture.
    pub fn clustered() -> Vec<u32> {
        // 0x00C0_FFEE == 0xC0FFEE, grouped in 4-digit blocks for clippy::unusual_byte_groupings.
        let mut rng = StdRng::seed_from_u64(0x00C0_FFEE);
        let mut out = Vec::with_capacity(1_000_000);
        for _ in 0..1_000 {
            // Cap the base so `base + 1_000` cannot overflow `u32`.
            let base = rng.random_range(0..=u32::MAX - 1_000);
            out.extend(base..base + 1_000);
        }
        out
    }

    /// 500,000 hits sampled (with replacement) from `data` + 500,000 uniform random `u32`,
    /// concatenated then shuffled — a ~50% hit rate probe stream.
    pub fn probes(data: &[u32]) -> Vec<u32> {
        let mut rng = StdRng::seed_from_u64(0xFEED_BEEF);
        let mut out = Vec::with_capacity(1_000_000);
        for _ in 0..500_000 {
            let idx = rng.random_range(0..data.len());
            out.push(data[idx]);
        }
        for _ in 0..500_000 {
            out.push(rng.random::<u32>());
        }
        out.shuffle(&mut rng);
        out
    }
}

#[derive(Clone, Default)]
pub struct RoaringBitmap {
    // Parallel vecs (keys[i] owns containers[i]), sorted by key, keys unique. Key binary searches
    // walk a dense 2-byte-stride vec (≤128 KiB even with every key present, cache-resident)
    // instead of striding 48-byte tuples, and a new-key insert shifts 42 B per entry instead of
    // 48. Sharding partitions this same shape by key.
    keys: Vec<u16>,
    containers: Vec<Container>,
}

impl RoaringBitmap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.keys.binary_search(&key) {
            Ok(i) => self.containers[i].insert(low),
            Err(i) => {
                self.keys.insert(i, key);
                self.containers.insert(i, Container::single(low));
                true
            }
        }
    }

    pub fn remove(&mut self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.keys.binary_search(&key) {
            Err(_) => false,
            Ok(i) => {
                let removed = self.containers[i].remove(low);
                // "Never empty" invariant: drop the entry once its last value is gone.
                if removed && self.containers[i].is_empty() {
                    self.keys.remove(i);
                    self.containers.remove(i);
                }
                removed
            }
        }
    }

    pub fn contains(&self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.keys.binary_search(&key) {
            Ok(i) => self.containers[i].contains(low),
            Err(_) => false,
        }
    }

    pub fn len(&self) -> u64 {
        // No cached global count: a single counter would become the cross-shard contention point
        // in every concurrent variant. Summing per-container O(1) cardinalities is cheap.
        self.containers.iter().map(|c| c.cardinality() as u64).sum()
    }

    pub fn is_empty(&self) -> bool {
        // Valid because containers are never stored empty (the remove path drops them).
        self.keys.is_empty()
    }

    pub fn optimize(&mut self) {
        for c in &mut self.containers {
            c.optimize();
        }
    }

    /// Set intersection. Two-pointer merge-join over the sorted key vecs: only keys present in
    /// *both* operands can contribute, and disjoint containers yield empty results we must drop to
    /// preserve the never-empty invariant.
    pub fn and(&self, other: &Self) -> Self {
        // Result keys ⊆ the smaller operand's keys: presizing makes every push realloc-free.
        let cap = self.keys.len().min(other.keys.len());
        let mut keys = Vec::with_capacity(cap);
        let mut containers = Vec::with_capacity(cap);
        let (mut i, mut j) = (0, 0);
        while i < self.keys.len() && j < other.keys.len() {
            match self.keys[i].cmp(&other.keys[j]) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    let c = self.containers[i].and(&other.containers[j]);
                    if !c.is_empty() {
                        keys.push(self.keys[i]);
                        containers.push(c);
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        RoaringBitmap { keys, containers }
    }

    /// Set union. Two-pointer merge-join: keys in exactly one operand carry their container over
    /// (cloned); keys in both merge via the container kernel. Merged containers are non-empty (union
    /// of two non-empty sets), so no drop is needed.
    pub fn or(&self, other: &Self) -> Self {
        // Worst case (disjoint key sets) the union carries every container from both sides.
        let cap = self.keys.len() + other.keys.len();
        let mut keys = Vec::with_capacity(cap);
        let mut containers = Vec::with_capacity(cap);
        let (mut i, mut j) = (0, 0);
        while i < self.keys.len() && j < other.keys.len() {
            match self.keys[i].cmp(&other.keys[j]) {
                Ordering::Less => {
                    keys.push(self.keys[i]);
                    containers.push(self.containers[i].clone());
                    i += 1;
                }
                Ordering::Greater => {
                    keys.push(other.keys[j]);
                    containers.push(other.containers[j].clone());
                    j += 1;
                }
                Ordering::Equal => {
                    keys.push(self.keys[i]);
                    containers.push(self.containers[i].or(&other.containers[j]));
                    i += 1;
                    j += 1;
                }
            }
        }
        keys.extend_from_slice(&self.keys[i..]);
        containers.extend_from_slice(&self.containers[i..]);
        keys.extend_from_slice(&other.keys[j..]);
        containers.extend_from_slice(&other.containers[j..]);
        RoaringBitmap { keys, containers }
    }

    /// Reassemble a whole map from per-shard clones, used by each concurrent type's `snapshot`.
    /// Shards partition the key space disjointly by `key & mask`, so each shard's containers are
    /// key-disjoint from every other's — concatenating them and re-sorting by key reconstructs the
    /// map with no kernel merge. `pub(crate)` so the private fields stay encapsulated.
    pub(crate) fn from_shards(shards: impl IntoIterator<Item = RoaringBitmap>) -> Self {
        let mut pairs: Vec<(u16, Container)> = Vec::new();
        for shard in shards {
            pairs.extend(shard.keys.into_iter().zip(shard.containers));
        }
        pairs.sort_by_key(|(k, _)| *k);
        let (keys, containers) = pairs.into_iter().unzip();
        RoaringBitmap { keys, containers }
    }

    /// Full structural invariant check (sorted/unique keys, no empty containers, per-container
    /// invariants) for use from integration tests.
    #[doc(hidden)]
    pub fn assert_invariants(&self) {
        assert_eq!(
            self.keys.len(),
            self.containers.len(),
            "keys/containers length mismatch"
        );
        for w in self.keys.windows(2) {
            assert!(w[0] < w[1], "container keys not sorted and unique");
        }
        for c in &self.containers {
            assert!(!c.is_empty(), "stored container is empty");
            c.assert_invariants();
        }
    }
}

impl BitAnd<&RoaringBitmap> for &RoaringBitmap {
    type Output = RoaringBitmap;
    fn bitand(self, rhs: &RoaringBitmap) -> RoaringBitmap {
        self.and(rhs)
    }
}

impl BitOr<&RoaringBitmap> for &RoaringBitmap {
    type Output = RoaringBitmap;
    fn bitor(self, rhs: &RoaringBitmap) -> RoaringBitmap {
        self.or(rhs)
    }
}

impl BitAndAssign<&RoaringBitmap> for RoaringBitmap {
    fn bitand_assign(&mut self, rhs: &RoaringBitmap) {
        *self = self.and(rhs);
    }
}

impl BitOrAssign<&RoaringBitmap> for RoaringBitmap {
    fn bitor_assign(&mut self, rhs: &RoaringBitmap) {
        *self = self.or(rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_join_roundtrip() {
        for x in [0u32, 1, 0xFFFF, 0x1_0000, 0x1_2345, u32::MAX] {
            let (k, l) = split(x);
            assert_eq!(join(k, l), x);
        }
        assert_eq!(split(0), (0, 0));
        assert_eq!(split(0xFFFF), (0, 0xFFFF));
        assert_eq!(split(0x1_0000), (1, 0));
        assert_eq!(split(u32::MAX), (0xFFFF, 0xFFFF));
    }

    #[test]
    fn boundary_values() {
        let mut b = RoaringBitmap::new();
        for x in [0u32, u32::MAX, 0xFFFF, 0x1_0000] {
            assert!(b.insert(x));
        }
        assert_eq!(b.len(), 4);
        for x in [0u32, u32::MAX, 0xFFFF, 0x1_0000] {
            assert!(b.contains(x));
        }
        // 0xFFFF and 0x1_0000 straddle the key boundary → two distinct containers.
        assert!(!b.contains(1));
        b.assert_invariants();

        assert!(b.remove(0x1_0000));
        assert!(!b.contains(0x1_0000));
        assert!(!b.remove(0x1_0000));
        assert_eq!(b.len(), 3);
        b.assert_invariants();
    }

    #[test]
    fn empty_container_dropped_and_reused() {
        let mut b = RoaringBitmap::new();
        assert!(b.insert(0x5_0001));
        assert!(b.remove(0x5_0001));
        assert!(b.is_empty());
        // Re-inserting into a just-emptied key must succeed (entry was dropped, then recreated).
        assert!(b.insert(0x5_0001));
        assert_eq!(b.len(), 1);
        b.assert_invariants();
    }
}
