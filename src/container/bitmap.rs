//! `BitmapContainer`: `Box<[u64; 1024]>` bitset for high-cardinality containers.

use super::array::ArrayContainer;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapContainer {
    words: Box<[u64; 1024]>, // boxed so a `Container` stays pointer-sized rather than inline 8 KiB (§2.3)
    cardinality: u32,        // u32: a full container holds 65536 values > u16::MAX (§2.3)
}

impl BitmapContainer {
    pub fn new() -> Self {
        Self {
            words: Box::new([0u64; 1024]),
            cardinality: 0,
        }
    }

    pub fn from_array(a: &ArrayContainer) -> Self {
        let mut b = Self::new();
        for &v in a.as_slice() {
            let i = (v >> 6) as usize;
            b.words[i] |= 1u64 << (v & 63);
        }
        b.cardinality = a.cardinality();
        b
    }

    pub fn to_array(&self) -> ArrayContainer {
        // Only legal below the array threshold; caller (Container::remove) checks the count.
        debug_assert!(self.cardinality <= 4096);
        let mut a = ArrayContainer::new();
        for (word_idx, &word) in self.words.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros();
                a.insert((word_idx as u16) * 64 + bit as u16);
                w &= w - 1; // clear the lowest set bit
            }
        }
        a
    }

    pub fn contains(&self, v: u16) -> bool {
        let i = (v >> 6) as usize;
        let mask = 1u64 << (v & 63);
        self.words[i] & mask != 0
    }

    pub fn insert(&mut self, v: u16) -> bool {
        let i = (v >> 6) as usize;
        let mask = 1u64 << (v & 63);
        let old = self.words[i];
        self.words[i] = old | mask;
        let added = old & mask == 0;
        self.cardinality += added as u32;
        added
    }

    pub fn remove(&mut self, v: u16) -> bool {
        let i = (v >> 6) as usize;
        let mask = 1u64 << (v & 63);
        let old = self.words[i];
        self.words[i] = old & !mask;
        let removed = old & mask != 0;
        self.cardinality -= removed as u32;
        removed
    }

    pub fn cardinality(&self) -> u32 {
        self.cardinality
    }

    pub fn is_empty(&self) -> bool {
        self.cardinality == 0
    }

    pub fn num_runs(&self) -> u32 {
        let mut runs: u32 = 0;
        let mut prev: u64 = 0;
        for &w in self.words.iter() {
            // `w & !(w << 1)` marks each set bit whose lower neighbor is clear — a run start
            // within the word; the correction removes the double-count when a run spans the
            // boundary (this word's bit 0 set and the previous word's bit 63 set are one run).
            runs += (w & !(w << 1)).count_ones();
            if (w & 1) == 1 && (prev >> 63) == 1 {
                runs -= 1;
            }
            prev = w;
        }
        runs
    }

    pub(crate) fn words(&self) -> &[u64; 1024] {
        &self.words
    }

    /// Build from precomputed words + cardinality (the P5 kernels compute both directly). Caller
    /// guarantees `cardinality` equals the popcount of `words`.
    pub(crate) fn from_words(words: Box<[u64; 1024]>, cardinality: u32) -> Self {
        debug_assert_eq!(
            cardinality,
            words.iter().map(|w| w.count_ones()).sum::<u32>()
        );
        Self { words, cardinality }
    }
}

impl Default for BitmapContainer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    #[test]
    fn insert_remove_contains_boundaries() {
        let mut b = BitmapContainer::new();
        assert!(b.insert(0));
        assert!(b.insert(65535));
        assert!(!b.insert(0));
        assert_eq!(b.cardinality(), 2);
        assert!(b.contains(0));
        assert!(b.contains(65535));
        assert!(!b.contains(1));
        assert!(b.remove(0));
        assert!(!b.remove(0));
        assert_eq!(b.cardinality(), 1);
    }

    #[test]
    fn num_runs_word_boundary() {
        // A run spanning the word-0 / word-1 boundary: bits 63 and 64 both set, contiguous.
        let mut b = BitmapContainer::new();
        b.insert(63);
        b.insert(64);
        // One run of two adjacent bits crossing the boundary — exercises the −1 correction.
        assert_eq!(b.num_runs(), 1);

        // Two separated runs, one of them straddling the boundary.
        let mut b2 = BitmapContainer::new();
        for v in [10u16, 11, 12, 63, 64, 65] {
            b2.insert(v);
        }
        assert_eq!(b2.num_runs(), 2);

        // Non-adjacent across boundary (bit 63 set, bit 64 clear, bit 65 set): two runs, no correction.
        let mut b3 = BitmapContainer::new();
        b3.insert(63);
        b3.insert(65);
        assert_eq!(b3.num_runs(), 2);
    }

    proptest! {
        // Cross-representation agreement: build a bitmap always, an array when ≤4096.
        #[test]
        fn bitmap_agrees_with_array(vals in prop::collection::btree_set(any::<u16>(), 0..=8192)) {
            let model: BTreeSet<u16> = vals;
            let mut b = BitmapContainer::new();
            let mut a = ArrayContainer::new();
            for &v in &model {
                b.insert(v);
                if model.len() <= 4096 {
                    a.insert(v);
                }
            }
            prop_assert_eq!(b.cardinality() as usize, model.len());
            for &m in &model {
                prop_assert!(b.contains(m));
            }
            for probe in 0..256u32 {
                let v = (probe.wrapping_mul(2654435761) & 0xFFFF) as u16;
                prop_assert_eq!(b.contains(v), model.contains(&v));
            }
            if model.len() <= 4096 {
                prop_assert_eq!(a.cardinality(), b.cardinality());
                // to_array(from_array(a)) == a
                let round = BitmapContainer::from_array(&a).to_array();
                prop_assert_eq!(round.as_slice(), a.as_slice());
            }
        }
    }
}
