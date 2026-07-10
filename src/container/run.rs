//! `RunContainer`: sorted, non-overlapping, non-adjacent runs.

use super::array::ArrayContainer;
use super::bitmap::BitmapContainer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub start: u16,
    pub len: u16, // len = count - 1: a full 65536-value run must fit (count 65536 > u16::MAX)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunContainer {
    runs: Vec<Run>,
    // Cached because Σ(len+1) is O(runs), and cardinality() must stay O(1).
    cardinality: u32,
}

impl RunContainer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, v: u16) -> bool {
        // First run with start > v is at `idx`; the only run that could hold v is idx-1.
        let idx = self.runs.partition_point(|r| r.start <= v);
        if idx == 0 {
            return false;
        }
        let run = self.runs[idx - 1];
        // Boundary math in u32: start+len can reach 65535.
        v as u32 <= run.start as u32 + run.len as u32
    }

    pub fn insert(&mut self, v: u16) -> bool {
        let idx = self.runs.partition_point(|r| r.start <= v);
        if idx > 0 {
            let prev = self.runs[idx - 1];
            let prev_end = prev.start as u32 + prev.len as u32;
            if (v as u32) <= prev_end {
                return false; // already inside a run
            }
            if v as u32 == prev_end + 1 {
                self.runs[idx - 1].len += 1;
                // If v now bridges to the next run, absorb it (keeps runs non-adjacent).
                if idx < self.runs.len() && self.runs[idx].start as u32 == v as u32 + 1 {
                    let next = self.runs[idx];
                    // u32 math: prev already includes v, adding next's full count stays ≤ 65535.
                    let merged = self.runs[idx - 1].len as u32 + next.len as u32 + 1;
                    self.runs[idx - 1].len = merged as u16;
                    self.runs.remove(idx);
                }
                self.cardinality += 1;
                return true;
            }
        }
        if idx < self.runs.len() && v as u32 + 1 == self.runs[idx].start as u32 {
            self.runs[idx].start -= 1;
            self.runs[idx].len += 1;
            self.cardinality += 1;
            return true;
        }
        // Isolated value: a fresh single-value run at the partition point.
        self.runs.insert(idx, Run { start: v, len: 0 });
        self.cardinality += 1;
        true
    }

    pub fn remove(&mut self, v: u16) -> bool {
        let idx = self.runs.partition_point(|r| r.start <= v);
        if idx == 0 {
            return false;
        }
        let i = idx - 1;
        let run = self.runs[i];
        let end = run.start as u32 + run.len as u32;
        if (v as u32) > end {
            return false; // past this run's end — not present
        }
        if run.len == 0 {
            self.runs.remove(i);
        } else if v == run.start {
            self.runs[i].start += 1;
            self.runs[i].len -= 1;
        } else if v as u32 == end {
            self.runs[i].len -= 1;
        } else {
            // Interior removal splits the run: [start, v-1] then [v+1, end].
            let left_len = (v as u32 - run.start as u32 - 1) as u16;
            let right_start = v + 1;
            let right_len = (end - right_start as u32) as u16;
            self.runs[i].len = left_len;
            self.runs.insert(
                i + 1,
                Run {
                    start: right_start,
                    len: right_len,
                },
            );
        }
        self.cardinality -= 1;
        true
    }

    pub fn cardinality(&self) -> u32 {
        self.cardinality
    }

    pub fn is_empty(&self) -> bool {
        self.cardinality == 0
    }

    pub fn num_runs(&self) -> u32 {
        self.runs.len() as u32
    }

    pub fn from_array(a: &ArrayContainer) -> Self {
        let mut runs: Vec<Run> = Vec::new();
        let mut cardinality = 0u32;
        // Sorted, unique values (array invariant): extend the tail run or start a new one.
        for &v in a.as_slice() {
            cardinality += 1;
            if let Some(last) = runs.last_mut() {
                let end = last.start as u32 + last.len as u32;
                if v as u32 == end + 1 {
                    last.len += 1;
                    continue;
                }
            }
            runs.push(Run { start: v, len: 0 });
        }
        Self { runs, cardinality }
    }

    pub fn from_bitmap(b: &BitmapContainer) -> Self {
        let mut runs: Vec<Run> = Vec::new();
        let mut cardinality = 0u32;
        for (word_idx, &word) in b.words().iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let v = (word_idx as u32) * 64 + w.trailing_zeros();
                cardinality += 1;
                if let Some(last) = runs.last_mut() {
                    let end = last.start as u32 + last.len as u32;
                    if v == end + 1 {
                        last.len += 1;
                        w &= w - 1;
                        continue;
                    }
                }
                runs.push(Run {
                    start: v as u16,
                    len: 0,
                });
                w &= w - 1;
            }
        }
        Self { runs, cardinality }
    }

    pub fn to_array(&self) -> ArrayContainer {
        let mut values = Vec::with_capacity(self.cardinality as usize);
        for run in &self.runs {
            let end = run.start as u32 + run.len as u32;
            for v in run.start as u32..=end {
                values.push(v as u16);
            }
        }
        ArrayContainer::from_sorted_vec(values)
    }

    pub fn to_bitmap(&self) -> BitmapContainer {
        let mut b = BitmapContainer::new();
        for run in &self.runs {
            let end = run.start as u32 + run.len as u32;
            for v in run.start as u32..=end {
                b.insert(v as u16);
            }
        }
        b
    }

    pub(crate) fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Build from precomputed runs + cardinality (the set-op kernels emit sorted, non-overlapping,
    /// non-adjacent runs). Caller guarantees those invariants and that `cardinality = Σ(len+1)`.
    pub(crate) fn from_runs(runs: Vec<Run>, cardinality: u32) -> Self {
        Self { runs, cardinality }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    /// A set built as a union of small ranges, so runs actually occur.
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

    fn assert_run_invariants(r: &RunContainer) {
        let runs = r.runs();
        let mut expected_card = 0u32;
        for (i, run) in runs.iter().enumerate() {
            expected_card += run.len as u32 + 1;
            if i > 0 {
                let prev = runs[i - 1];
                let prev_end = prev.start as u32 + prev.len as u32;
                // sorted, non-overlapping, non-adjacent: next.start > prev_end + 1
                assert!(run.start as u32 > prev_end + 1, "runs adjacent/overlapping");
            }
        }
        assert_eq!(expected_card, r.cardinality(), "cached cardinality wrong");
    }

    #[test]
    fn insert_extend_merge_split() {
        let mut r = RunContainer::new();
        // Two separated runs.
        for v in [10u16, 11, 12, 20, 21] {
            assert!(r.insert(v));
        }
        assert_eq!(r.num_runs(), 2);
        // Fill the gap at 13..=19: eventually the two runs merge into one.
        for v in 13u16..=19 {
            assert!(r.insert(v));
        }
        assert_eq!(r.num_runs(), 1);
        assert_eq!(r.cardinality(), 12);
        assert_run_invariants(&r);
        // Interior removal splits.
        assert!(r.remove(15));
        assert_eq!(r.num_runs(), 2);
        assert!(!r.contains(15));
        assert_run_invariants(&r);
    }

    proptest! {
        // Tri-representation agreement: bitmap always, array when small, run from bitmap; all
        // to_*/from_* round-trips identity.
        #[test]
        fn tri_representation_agrees(model in range_set()) {
            let mut b = BitmapContainer::new();
            for &v in &model {
                b.insert(v);
            }
            let r = RunContainer::from_bitmap(&b);
            prop_assert_eq!(r.cardinality() as usize, model.len());
            for &m in &model {
                prop_assert!(r.contains(m));
            }
            for probe in 0..256u32 {
                let v = (probe.wrapping_mul(2654435761) & 0xFFFF) as u16;
                prop_assert_eq!(r.contains(v), model.contains(&v));
            }
            assert_run_invariants(&r);
            prop_assert_eq!(&r.to_bitmap(), &b);
            if model.len() <= 4096 {
                let mut a = ArrayContainer::new();
                for &v in &model {
                    a.insert(v);
                }
                prop_assert_eq!(&RunContainer::from_array(&a), &r);
                let r_as_array = r.to_array();
                prop_assert_eq!(r_as_array.as_slice(), a.as_slice());
            }
        }

        // Mutation vs BTreeSet model; values drawn from a small domain to force merge/split paths.
        #[test]
        fn run_mutation_matches_btreeset(
            ops in prop::collection::vec((any::<bool>(), 0u16..256), 0..512)) {
            let mut model = BTreeSet::new();
            let mut r = RunContainer::new();
            for (is_insert, v) in ops {
                if is_insert {
                    prop_assert_eq!(r.insert(v), model.insert(v));
                } else {
                    prop_assert_eq!(r.remove(v), model.remove(&v));
                }
                prop_assert_eq!(r.contains(v), model.contains(&v));
                assert_run_invariants(&r);
            }
            prop_assert_eq!(r.cardinality() as usize, model.len());
            for &m in &model {
                prop_assert!(r.contains(m));
            }
        }
    }
}
