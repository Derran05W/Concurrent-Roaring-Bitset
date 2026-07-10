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

/// A random set of `u32` values built with the same dense/sparse value mix as the op strategy.
fn set_strategy() -> impl Strategy<Value = Vec<u32>> {
    prop::collection::vec(value(), 0..2000)
}

#[test]
fn operators_delegate_to_methods() {
    let mut a = RoaringBitmap::new();
    let mut b = RoaringBitmap::new();
    for x in [1u32, 2, 3, 100, 70_000, 70_001] {
        a.insert(x);
    }
    for x in [2u32, 3, 4, 70_001, 200_000] {
        b.insert(x);
    }
    // Operator forms must agree with the named methods.
    assert_eq!((&a & &b).len(), a.and(&b).len());
    assert_eq!((&a | &b).len(), a.or(&b).len());

    let mut c = a.clone();
    c &= &b;
    assert_eq!(c.len(), a.and(&b).len());
    let mut d = a.clone();
    d |= &b;
    assert_eq!(d.len(), a.or(&b).len());
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

    /// `and`/`or` against the reference crate. Our operand `a` is `optimize`d so the Run kernels
    /// participate (the `roaring` 0.10 crate has no run containers / run-optimize, so the
    /// reference is left as the plain oracle). Equality check: equal cardinality plus every
    /// reference element contained in ours implies set equality.
    #[test]
    fn setops_match_roaring_crate(va in set_strategy(), vb in set_strategy()) {
        let mut a = RoaringBitmap::new();
        let mut ra = RefBitmap::new();
        for &x in &va {
            a.insert(x);
            ra.insert(x);
        }
        let mut b = RoaringBitmap::new();
        let mut rb = RefBitmap::new();
        for &x in &vb {
            b.insert(x);
            rb.insert(x);
        }
        // Force Run participation on our operand (lossless; the reference stays a plain oracle).
        a.optimize();

        let ours_and = a.and(&b);
        let ref_and = &ra & &rb;
        prop_assert_eq!(ours_and.len(), ref_and.len(), "and cardinality mismatch");
        for x in ref_and.iter() {
            prop_assert!(ours_and.contains(x), "and: value {} missing from ours", x);
        }
        ours_and.assert_invariants();

        let ours_or = a.or(&b);
        let ref_or = &ra | &rb;
        prop_assert_eq!(ours_or.len(), ref_or.len(), "or cardinality mismatch");
        for x in ref_or.iter() {
            prop_assert!(ours_or.contains(x), "or: value {} missing from ours", x);
        }
        ours_or.assert_invariants();
    }

    /// Algebraic laws on sampled probes: `a ∩ b ⊆ a` and `⊆ b`; `a ⊆ a ∪ b` and `b ⊆ a ∪ b`;
    /// both ops commute in cardinality.
    #[test]
    fn setops_algebraic(va in set_strategy(), vb in set_strategy()) {
        let mut a = RoaringBitmap::new();
        for &x in &va {
            a.insert(x);
        }
        let mut b = RoaringBitmap::new();
        for &x in &vb {
            b.insert(x);
        }
        let and_ab = a.and(&b);
        let or_ab = a.or(&b);
        for &x in va.iter().chain(vb.iter()) {
            if and_ab.contains(x) {
                prop_assert!(a.contains(x) && b.contains(x), "and superset violated at {}", x);
            }
            if a.contains(x) || b.contains(x) {
                prop_assert!(or_ab.contains(x), "or subset violated at {}", x);
            }
        }
        prop_assert_eq!(and_ab.len(), b.and(&a).len(), "and not commutative in len");
        prop_assert_eq!(or_ab.len(), b.or(&a).len(), "or not commutative in len");
        and_ab.assert_invariants();
        or_ab.assert_invariants();
    }
}
