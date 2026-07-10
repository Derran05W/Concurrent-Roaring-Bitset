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

#[derive(Clone, Default)]
pub struct RoaringBitmap {
    // Sorted by key, keys unique. This exact shape is what P7 shards (partition by key).
    containers: Vec<(u16, Container)>,
}

impl RoaringBitmap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.containers.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(i) => self.containers[i].1.insert(low),
            Err(i) => {
                self.containers.insert(i, (key, Container::single(low)));
                true
            }
        }
    }

    pub fn remove(&mut self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.containers.binary_search_by_key(&key, |(k, _)| *k) {
            Err(_) => false,
            Ok(i) => {
                let removed = self.containers[i].1.remove(low);
                // "Never empty" invariant: drop the entry once its last value is gone.
                if removed && self.containers[i].1.is_empty() {
                    self.containers.remove(i);
                }
                removed
            }
        }
    }

    pub fn contains(&self, x: u32) -> bool {
        let (key, low) = split(x);
        match self.containers.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(i) => self.containers[i].1.contains(low),
            Err(_) => false,
        }
    }

    pub fn len(&self) -> u64 {
        // No cached global count: a single counter would become the cross-shard contention point
        // in every concurrent variant (P7/P8). Summing per-container O(1) cardinalities is cheap.
        self.containers
            .iter()
            .map(|(_, c)| c.cardinality() as u64)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        // Valid because containers are never stored empty (the remove path drops them).
        self.containers.is_empty()
    }

    pub fn optimize(&mut self) {
        for (_, c) in &mut self.containers {
            c.optimize();
        }
    }

    /// Set intersection. Two-pointer merge-join over the sorted key vecs: only keys present in
    /// *both* operands can contribute, and disjoint containers yield empty results we must drop to
    /// preserve the never-empty invariant.
    pub fn and(&self, other: &Self) -> Self {
        let (a, b) = (&self.containers, &other.containers);
        let mut out: Vec<(u16, Container)> = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            match a[i].0.cmp(&b[j].0) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    let c = a[i].1.and(&b[j].1);
                    if !c.is_empty() {
                        out.push((a[i].0, c));
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        RoaringBitmap { containers: out }
    }

    /// Set union. Two-pointer merge-join: keys in exactly one operand carry their container over
    /// (cloned); keys in both merge via the container kernel. Merged containers are non-empty (union
    /// of two non-empty sets), so no drop is needed.
    pub fn or(&self, other: &Self) -> Self {
        let (a, b) = (&self.containers, &other.containers);
        let mut out: Vec<(u16, Container)> = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            match a[i].0.cmp(&b[j].0) {
                Ordering::Less => {
                    out.push(a[i].clone());
                    i += 1;
                }
                Ordering::Greater => {
                    out.push(b[j].clone());
                    j += 1;
                }
                Ordering::Equal => {
                    out.push((a[i].0, a[i].1.or(&b[j].1)));
                    i += 1;
                    j += 1;
                }
            }
        }
        out.extend_from_slice(&a[i..]);
        out.extend_from_slice(&b[j..]);
        RoaringBitmap { containers: out }
    }

    /// Full §2.3/§2.5 invariant check for use from integration tests.
    #[doc(hidden)]
    pub fn assert_invariants(&self) {
        let mut prev_key: Option<u16> = None;
        for (key, c) in &self.containers {
            if let Some(pk) = prev_key {
                assert!(*key > pk, "container keys not sorted and unique");
            }
            assert!(!c.is_empty(), "stored container is empty");
            c.assert_invariants();
            prev_key = Some(*key);
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
