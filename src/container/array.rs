//! `ArrayContainer`: sorted `Vec<u16>` for low-cardinality containers.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArrayContainer {
    values: Vec<u16>,
}

impl ArrayContainer {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn cardinality(&self) -> u32 {
        self.values.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn contains(&self, v: u16) -> bool {
        self.values.binary_search(&v).is_ok()
    }

    pub fn insert(&mut self, v: u16) -> bool {
        match self.values.binary_search(&v) {
            Ok(_) => false,
            Err(idx) => {
                self.values.insert(idx, v);
                true
            }
        }
    }

    pub fn remove(&mut self, v: u16) -> bool {
        match self.values.binary_search(&v) {
            Ok(idx) => {
                self.values.remove(idx);
                true
            }
            Err(_) => false,
        }
    }

    pub fn num_runs(&self) -> u32 {
        if self.values.is_empty() {
            return 0;
        }
        // A new run starts at index 0 and at each gap (value jumps by >1).
        let mut runs = 1;
        for pair in self.values.windows(2) {
            if pair[1] > pair[0] + 1 {
                runs += 1;
            }
        }
        runs
    }

    pub(crate) fn as_slice(&self) -> &[u16] {
        &self.values
    }

    /// Build directly from a Vec the caller guarantees is sorted and duplicate-free (the P5 set-op
    /// kernels produce exactly such vecs). Avoids the O(n²) of repeated insert-at-index.
    pub(crate) fn from_sorted_vec(values: Vec<u16>) -> Self {
        debug_assert!(values.windows(2).all(|w| w[0] < w[1]));
        Self { values }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    #[test]
    fn insert_remove_contains_boundaries() {
        let mut a = ArrayContainer::new();
        assert!(a.insert(0));
        assert!(a.insert(65535));
        assert!(a.contains(0));
        assert!(a.contains(65535));
        assert!(!a.contains(1));
        assert_eq!(a.cardinality(), 2);

        // duplicate insert
        assert!(!a.insert(0));
        assert_eq!(a.cardinality(), 2);

        // remove present / absent
        assert!(a.remove(0));
        assert!(!a.remove(0));
        assert!(!a.remove(12345));
        assert!(!a.contains(0));
        assert!(a.contains(65535));
    }

    #[test]
    fn interleaved_keeps_sorted() {
        let mut a = ArrayContainer::new();
        for v in [500u16, 3, 60000, 3, 12, 500, 0] {
            a.insert(v);
        }
        assert!(a.as_slice().windows(2).all(|w| w[0] < w[1]));
        assert_eq!(a.as_slice(), &[0, 3, 12, 500, 60000]);
    }

    #[test]
    fn num_runs_basic() {
        let mut a = ArrayContainer::new();
        assert_eq!(a.num_runs(), 0);
        for v in [0u16, 1, 2, 5, 6, 100] {
            a.insert(v);
        }
        // runs: {0,1,2}, {5,6}, {100} => 3
        assert_eq!(a.num_runs(), 3);
    }

    proptest! {
        #[test]
        fn array_matches_btreeset(ops in prop::collection::vec(
            (any::<bool>(), any::<u16>()), 0..1024)) {
            let mut model = BTreeSet::new();
            let mut a = ArrayContainer::new();
            for (is_insert, v) in ops {
                if is_insert {
                    prop_assert_eq!(a.insert(v), model.insert(v));
                } else {
                    prop_assert_eq!(a.remove(v), model.remove(&v));
                }
                prop_assert_eq!(a.contains(v), model.contains(&v));
            }
            prop_assert_eq!(a.cardinality() as usize, model.len());
            for m in &model {
                prop_assert!(a.contains(*m));
            }
            for probe in 0..64u32 {
                let v = (probe.wrapping_mul(1009) & 0xFFFF) as u16;
                prop_assert_eq!(a.contains(v), model.contains(&v));
            }
        }

        #[test]
        fn array_stays_sorted(ops in prop::collection::vec(
            (any::<bool>(), any::<u16>()), 0..1024)) {
            let mut a = ArrayContainer::new();
            for (is_insert, v) in ops {
                if is_insert { a.insert(v); } else { a.remove(v); }
                prop_assert!(a.as_slice().windows(2).all(|w| w[0] < w[1]));
            }
        }
    }
}
