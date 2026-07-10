//! `Container` enum, dispatch, conversion policy, normalization, and set-op kernels.

pub mod array;
pub mod bitmap;
pub mod run;

use array::ArrayContainer;
use bitmap::BitmapContainer;

/// A roaring container. The `Run` variant is introduced in P3; growing a crate-private enum is a
/// non-breaking internal change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Container {
    Array(ArrayContainer),
    Bitmap(BitmapContainer),
}

impl Container {
    pub fn insert(&mut self, v: u16) -> bool {
        match self {
            Container::Array(a) => {
                // Pre-convert on the 4097th distinct value: 4096×2 B = 8192 B = bitmap size, so the
                // array only wins strictly below 4096. Converting first avoids growing the Vec to
                // 4097 then copying it (§2.4 row 1).
                if a.cardinality() == 4096 && !a.contains(v) {
                    let mut b = BitmapContainer::from_array(a);
                    let added = b.insert(v);
                    *self = Container::Bitmap(b);
                    added
                } else {
                    a.insert(v)
                }
            }
            Container::Bitmap(b) => b.insert(v),
        }
    }

    pub fn remove(&mut self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.remove(v),
            Container::Bitmap(b) => {
                let removed = b.remove(v);
                // Crossing back to exactly 4096 means the array is now the smaller representation;
                // `==` is correct because remove changes the count by exactly one (§2.4 row 2).
                if removed && b.cardinality() == 4096 {
                    *self = Container::Array(b.to_array());
                }
                removed
            }
        }
    }

    pub fn contains(&self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.contains(v),
            Container::Bitmap(b) => b.contains(v),
        }
    }

    pub fn cardinality(&self) -> u32 {
        match self {
            Container::Array(a) => a.cardinality(),
            Container::Bitmap(b) => b.cardinality(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Container::Array(a) => a.is_empty(),
            Container::Bitmap(b) => b.is_empty(),
        }
    }

    pub fn num_runs(&self) -> u32 {
        match self {
            Container::Array(a) => a.num_runs(),
            Container::Bitmap(b) => b.num_runs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn array_to_bitmap_to_array_thresholds() {
        let mut c = Container::Array(ArrayContainer::new());
        for v in 0u16..5000 {
            assert!(c.insert(v));
        }
        assert!(matches!(c, Container::Bitmap(_)));
        assert_eq!(c.cardinality(), 5000);
        for v in 0u16..5000 {
            assert!(c.contains(v));
        }

        // Remove 904 values (5000 → 4096) to land exactly on the array threshold.
        for v in 0u16..904 {
            assert!(c.remove(v));
        }
        assert_eq!(c.cardinality(), 4096);
        assert!(matches!(c, Container::Array(_)));
        for v in 904u16..5000 {
            assert!(c.contains(v));
        }
        for v in 0u16..904 {
            assert!(!c.contains(v));
        }
    }

    proptest! {
        // Insert enough distinct values to force the bitmap variant, then remove back across the
        // threshold; membership must survive both conversions.
        #[test]
        fn threshold_membership_preserved(
            extra in prop::collection::btree_set(0u16..20000, 4097..=6000)) {
            let mut c = Container::Array(ArrayContainer::new());
            for &v in &extra {
                c.insert(v);
            }
            prop_assert!(matches!(c, Container::Bitmap(_)));
            prop_assert_eq!(c.cardinality() as usize, extra.len());

            // Remove down to 4096 (or as close as the set allows above it).
            let target = 4096usize;
            let mut sorted: Vec<u16> = extra.iter().copied().collect();
            let to_remove = extra.len() - target;
            for &v in sorted.iter().take(to_remove) {
                prop_assert!(c.remove(v));
            }
            prop_assert!(matches!(c, Container::Array(_)));
            prop_assert_eq!(c.cardinality() as usize, target);
            for &v in sorted.iter().skip(to_remove) {
                prop_assert!(c.contains(v));
            }
            sorted.truncate(to_remove);
            for &v in &sorted {
                prop_assert!(!c.contains(v));
            }
        }
    }
}
