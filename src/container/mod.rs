//! `Container` enum, dispatch, conversion policy, normalization, and set-op kernels.

pub mod array;
pub mod bitmap;
pub mod run;

use array::ArrayContainer;
use bitmap::BitmapContainer;
use run::RunContainer;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Container {
    Array(ArrayContainer),
    Bitmap(BitmapContainer),
    Run(RunContainer),
}

impl Container {
    /// A fresh container holding exactly `v`. Every key starts life as an `ArrayContainer` (§2.4).
    pub fn single(v: u16) -> Container {
        let mut a = ArrayContainer::new();
        a.insert(v);
        Container::Array(a)
    }

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
            Container::Run(r) => {
                let added = r.insert(v);
                if added {
                    self.demote_run_if_bloated();
                }
                added
            }
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
            Container::Run(r) => {
                // A remove can split a run and thus *increase* the run count, so re-check after.
                let removed = r.remove(v);
                if removed {
                    self.demote_run_if_bloated();
                }
                removed
            }
        }
    }

    /// A run list bigger than a bitmap has lost its reason to exist: `4 × num_runs > 8192` (§2.4).
    fn demote_run_if_bloated(&mut self) {
        if let Container::Run(r) = self {
            if 4 * r.num_runs() > 8192 {
                *self = Container::Bitmap(r.to_bitmap());
            }
        }
    }

    pub fn contains(&self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.contains(v),
            Container::Bitmap(b) => b.contains(v),
            Container::Run(r) => r.contains(v),
        }
    }

    pub fn cardinality(&self) -> u32 {
        match self {
            Container::Array(a) => a.cardinality(),
            Container::Bitmap(b) => b.cardinality(),
            Container::Run(r) => r.cardinality(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Container::Array(a) => a.is_empty(),
            Container::Bitmap(b) => b.is_empty(),
            Container::Run(r) => r.is_empty(),
        }
    }

    pub fn num_runs(&self) -> u32 {
        match self {
            Container::Array(a) => a.num_runs(),
            Container::Bitmap(b) => b.num_runs(),
            Container::Run(r) => r.num_runs(),
        }
    }

    /// Smallest-of-three (§2.4): convert iff the smallest valid representation is *strictly*
    /// smaller than the current one; ties keep the current representation (avoids thrashing).
    pub fn optimize(&mut self) {
        let card = self.cardinality();
        let num_runs = self.num_runs();
        // Byte sizes: array 2×card (only ≤4096), bitmap 8192, run 4×num_runs.
        let current_size = match self {
            Container::Array(_) => 2 * card,
            Container::Bitmap(_) => 8192,
            Container::Run(_) => 4 * num_runs,
        };
        let mut best_size = current_size;
        let mut target: Option<Repr> = None;
        if card <= 4096 && 2 * card < best_size {
            best_size = 2 * card;
            target = Some(Repr::Array);
        }
        if 8192 < best_size {
            best_size = 8192;
            target = Some(Repr::Bitmap);
        }
        if 4 * num_runs < best_size {
            target = Some(Repr::Run);
        }
        match target {
            None => {}
            Some(Repr::Array) => {
                let a = match self {
                    Container::Bitmap(b) => b.to_array(),
                    Container::Run(r) => r.to_array(),
                    Container::Array(_) => return,
                };
                *self = Container::Array(a);
            }
            Some(Repr::Bitmap) => {
                let b = match self {
                    Container::Array(a) => BitmapContainer::from_array(a),
                    Container::Run(r) => r.to_bitmap(),
                    Container::Bitmap(_) => return,
                };
                *self = Container::Bitmap(b);
            }
            Some(Repr::Run) => {
                let r = match self {
                    Container::Array(a) => RunContainer::from_array(a),
                    Container::Bitmap(b) => RunContainer::from_bitmap(b),
                    Container::Run(_) => return,
                };
                *self = Container::Run(r);
            }
        }
    }
}

impl Container {
    /// Assert this container's structural invariants (§2.3 table). Used by
    /// `RoaringBitmap::assert_invariants` from integration tests; recomputes cached fields and
    /// compares them against the stored caches.
    pub(crate) fn assert_invariants(&self) {
        match self {
            Container::Array(a) => {
                let s = a.as_slice();
                // Stored inside a RoaringBitmap, an array never exceeds the bitmap threshold.
                assert!(
                    s.len() <= 4096,
                    "array cardinality {} exceeds 4096",
                    s.len()
                );
                for w in s.windows(2) {
                    assert!(w[0] < w[1], "array values not strictly increasing");
                }
            }
            Container::Bitmap(b) => {
                let popcount: u32 = b.words().iter().map(|w| w.count_ones()).sum();
                assert_eq!(
                    popcount,
                    b.cardinality(),
                    "bitmap cached cardinality != popcount"
                );
                // A stored bitmap always sits strictly above the array threshold.
                assert!(
                    b.cardinality() > 4096,
                    "stored bitmap cardinality {} not > 4096",
                    b.cardinality()
                );
            }
            Container::Run(r) => {
                let mut card: u32 = 0;
                let mut prev_end: Option<u32> = None;
                for run in r.runs() {
                    // Boundary math in u32: start + len can reach 65535 (§P3 rule).
                    let start = run.start as u32;
                    let end = start + run.len as u32;
                    if let Some(pe) = prev_end {
                        // Sorted, non-overlapping AND non-adjacent: a gap of ≥1 must separate runs,
                        // else they would have been merged into one run.
                        assert!(start > pe + 1, "runs overlap or are adjacent");
                    }
                    card += run.len as u32 + 1;
                    prev_end = Some(end);
                }
                assert_eq!(card, r.cardinality(), "run cached cardinality != Σ(len+1)");
            }
        }
    }
}

enum Repr {
    Array,
    Bitmap,
    Run,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    /// A set built as a union of small ranges, so `optimize` can pick the Run representation.
    fn range_set() -> impl Strategy<Value = BTreeSet<u16>> {
        prop::collection::vec((any::<u16>(), 0u16..64), 0..=64).prop_map(|ranges| {
            let mut s = BTreeSet::new();
            for (start, len) in ranges {
                let end = (start as u32 + len as u32).min(65535);
                for v in start as u32..=end {
                    s.insert(v as u16);
                }
            }
            s
        })
    }

    fn repr_size(c: &Container) -> u32 {
        match c {
            Container::Array(_) => 2 * c.cardinality(),
            Container::Bitmap(_) => 8192,
            Container::Run(_) => 4 * c.num_runs(),
        }
    }

    #[test]
    fn optimize_selects_run_then_mutation_demotes_to_bitmap() {
        // A single long run: optimize prefers Run (4 bytes) over bitmap (8192).
        let mut c = Container::Array(ArrayContainer::new());
        for v in 0u16..=9000 {
            c.insert(v);
        }
        c.optimize();
        assert!(matches!(c, Container::Run(_)));

        // Removing every other value splits the run repeatedly; once the run list exceeds the
        // bitmap in size the container demotes to Bitmap.
        let mut v = 1u16;
        while matches!(c, Container::Run(_)) && v < 9000 {
            c.remove(v);
            v = v.saturating_add(2);
        }
        assert!(matches!(c, Container::Bitmap(_)));
    }

    #[test]
    fn run_insert_of_isolated_values_demotes_to_bitmap() {
        let mut c = Container::Array(ArrayContainer::new());
        for v in 0u16..=8000 {
            c.insert(v);
        }
        c.optimize();
        assert!(matches!(c, Container::Run(_)));
        // Isolated inserts (gap of 1) each add a new run until the list bloats past a bitmap.
        let mut v = 8002u16;
        while matches!(c, Container::Run(_)) && v < 65534 {
            c.insert(v);
            v = v.saturating_add(2);
        }
        assert!(matches!(c, Container::Bitmap(_)));
    }

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
        // optimize: never increases representation size, preserves membership/cardinality, and is
        // idempotent (a second call changes nothing).
        #[test]
        fn optimize_shrinks_and_is_idempotent(model in range_set()) {
            let mut c = Container::Array(ArrayContainer::new());
            for &v in &model {
                c.insert(v);
            }
            let size_before = repr_size(&c);
            let card_before = c.cardinality();
            c.optimize();
            prop_assert!(repr_size(&c) <= size_before);
            prop_assert_eq!(c.cardinality(), card_before);
            for &m in &model {
                prop_assert!(c.contains(m));
            }
            for probe in 0..256u32 {
                let v = (probe.wrapping_mul(40503) & 0xFFFF) as u16;
                prop_assert_eq!(c.contains(v), model.contains(&v));
            }
            let snapshot = c.clone();
            c.optimize();
            prop_assert_eq!(c, snapshot);
        }

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
