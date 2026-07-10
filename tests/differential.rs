//! Differential tests: our `RoaringBitmap` vs the published `roaring` crate on identical op streams.

use concurrent_roaring::RoaringBitmap;
use proptest::prelude::*;
use roaring::RoaringBitmap as RefBitmap;

#[derive(Debug, Clone)]
enum Op {
    Insert(u32),
    Remove(u32),
    Contains(u32),
}

/// Value strategy: the narrow arm forces dense keys (container conversions), the wide arm
/// exercises sparse keys across the full `u32` domain.
fn value() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..500_000, any::<u32>()]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        value().prop_map(Op::Insert),
        value().prop_map(Op::Remove),
        value().prop_map(Op::Contains),
    ]
}

#[test]
fn boundary_units() {
    let mut ours = RoaringBitmap::new();
    let mut refb = RefBitmap::new();
    for x in [0u32, u32::MAX, 0xFFFF, 0x1_0000] {
        assert_eq!(ours.insert(x), refb.insert(x));
    }
    for x in [0u32, u32::MAX, 0xFFFF, 0x1_0000, 1, 0xFFFE, 0x1_0001] {
        assert_eq!(
            ours.contains(x),
            refb.contains(x),
            "contains disagree at {x}"
        );
    }
    assert_eq!(ours.len(), refb.len());
    ours.assert_invariants();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Apply the same ≤3000-op stream to ours and the reference; every op's return value must
    /// match, then final cardinality and sampled membership must match.
    #[test]
    fn matches_roaring_crate(ops in prop::collection::vec(op(), 0..3000)) {
        let mut ours = RoaringBitmap::new();
        let mut refb = RefBitmap::new();
        let mut inserted: Vec<u32> = Vec::new();
        for o in &ops {
            match *o {
                Op::Insert(x) => {
                    prop_assert_eq!(ours.insert(x), refb.insert(x), "insert {} disagrees", x);
                    inserted.push(x);
                }
                Op::Remove(x) => {
                    prop_assert_eq!(ours.remove(x), refb.remove(x), "remove {} disagrees", x);
                }
                Op::Contains(x) => {
                    prop_assert_eq!(ours.contains(x), refb.contains(x), "contains {} disagrees", x);
                }
            }
        }
        prop_assert_eq!(ours.len(), refb.len());
        for (i, &x) in inserted.iter().enumerate() {
            if i % 64 == 0 {
                prop_assert_eq!(ours.contains(x), refb.contains(x));
            }
        }
        ours.assert_invariants();
    }

    /// Same stream but with `optimize()` interleaved mid-way and at the end — optimize must not
    /// alter membership or cardinality.
    #[test]
    fn optimize_preserves_semantics(ops in prop::collection::vec(op(), 0..3000)) {
        let mut ours = RoaringBitmap::new();
        let mut refb = RefBitmap::new();
        let mid = ops.len() / 2;
        for (idx, o) in ops.iter().enumerate() {
            match *o {
                Op::Insert(x) => {
                    prop_assert_eq!(ours.insert(x), refb.insert(x));
                }
                Op::Remove(x) => {
                    prop_assert_eq!(ours.remove(x), refb.remove(x));
                }
                Op::Contains(x) => {
                    prop_assert_eq!(ours.contains(x), refb.contains(x));
                }
            }
            if idx == mid {
                ours.optimize();
            }
        }
        ours.optimize();
        ours.assert_invariants();
        prop_assert_eq!(ours.len(), refb.len());
        // Membership must be identical after optimize: probe every value the reference holds via
        // a sample plus fixed boundary probes.
        for x in refb.iter().step_by(37) {
            prop_assert!(ours.contains(x), "value {} lost after optimize", x);
        }
        for x in [0u32, u32::MAX, 0xFFFF, 0x1_0000] {
            prop_assert_eq!(ours.contains(x), refb.contains(x));
        }
    }
}
