// These adapters are shared with larger benchmark targets.
#[allow(dead_code)]
#[path = "../benches/support/mod.rs"]
mod support;
use proptest::prelude::*;
use roaring::RoaringBitmap as RefBitmap;
use std::collections::BTreeSet;
use support::*;

fn differential<B: Sequential>(ops: &[(u8, u32)]) {
    let mut b = B::default();
    let mut model = BTreeSet::new();
    for &(op, x) in ops {
        match op {
            0 => assert_eq!(b.insert(x), model.insert(x)),
            1 => assert_eq!(b.remove(x), model.remove(&x)),
            2 => assert_eq!(b.contains(x), model.contains(&x)),
            _ => b.optimize(),
        }
        assert_eq!(b.len(), model.len() as u64);
    }
    b.validate();
    assert!(model.iter().all(|&x| b.contains(x)));
}
proptest! {
    #[test]
    fn external_adapters_match_set(ops in prop::collection::vec((0u8..4, prop_oneof![0u32..10_000, any::<u32>()]), 0..1000)) {
        differential::<concurrent_roaring::RoaringBitmap>(&ops);
        differential::<RefBitmap>(&ops);
        differential::<croaring::Bitmap>(&ops);
    }
}
fn check_pairs<B: Sequential>() {
    let values: Vec<_> = (0..10000).map(|x| x * 3).collect();
    for overlap in [0, 50, 100] {
        for skew in [1, 8] {
            let (av, bv) = pair(&values, overlap, skew);
            let ra: RefBitmap = av.iter().copied().collect();
            let rb: RefBitmap = bv.iter().copied().collect();
            assert_eq!((&ra & &rb).len(), rb.len() * overlap as u64 / 100);
            let a = build::<B>(&av, true);
            let b = build::<B>(&bv, true);
            verify(&a.and(&b), &(&ra & &rb));
            verify(&a.or(&b), &(&ra | &rb));
        }
    }
}
#[test]
fn controlled_pairs_and_exact_probes() {
    check_pairs::<concurrent_roaring::RoaringBitmap>();
    check_pairs::<RefBitmap>();
    check_pairs::<croaring::Bitmap>();
    let data: Vec<_> = (0..10000).collect();
    for hits in [0, 50, 100] {
        let p = probes(&data, hits, 10000);
        assert_eq!(p.iter().filter(|&&x| x < 10000).count(), hits * 100);
    }
}
fn concurrent_contract<B: Concurrent>() {
    let b = B::default();
    std::thread::scope(|s| {
        for t in 0..8 {
            let b = &b;
            s.spawn(move || {
                for i in 0..1000 {
                    let x = (i << 16) | t;
                    assert!(b.insert(x));
                    assert!(!b.insert(x));
                    assert!(b.contains(x));
                }
            });
        }
    });
    assert_eq!(b.len(), 8000);
    b.optimize();
    std::thread::scope(|s| {
        for t in 0..8 {
            let b = &b;
            s.spawn(move || {
                for i in 0..1000 {
                    let x = (i << 16) | t;
                    assert!(b.remove(x));
                    assert!(!b.remove(x));
                }
            });
        }
    });
    assert_eq!(b.len(), 0);
}
#[test]
fn external_concurrent_wrappers_preserve_updates() {
    concurrent_contract::<ShardedRoaring>();
    concurrent_contract::<ShardedCroaring>();
    concurrent_contract::<dashmap::DashSet<u32>>();
}

#[test]
fn direct_key_lookup_survives_gaps_and_boundaries() {
    use concurrent_roaring::RoaringBitmap;
    for keys in [
        vec![],
        vec![65535],
        vec![0, 1, 2, 3],
        vec![100, 101, 102],
        vec![10, 12, 13, 15],
        vec![0, 64, 128, 192],
    ] {
        let mut b = RoaringBitmap::new();
        let mut reference = RefBitmap::new();
        for k in keys {
            for low in [0, 65535] {
                let x = (k << 16) | low;
                b.insert(x);
                reference.insert(x);
            }
        }
        for key in 0..=65535u32 {
            for low in [0, 1, 65535] {
                let x = (key << 16) | low;
                assert_eq!(b.contains(x), reference.contains(x));
            }
        }
        b.optimize();
        let removed: Vec<_> = reference.iter().step_by(2).collect();
        for x in removed {
            assert_eq!(b.remove(x), reference.remove(x));
        }
        for key in 0..=65535u32 {
            for low in [0, 65535] {
                let x = (key << 16) | low;
                assert_eq!(b.contains(x), reference.contains(x));
            }
        }
        b.assert_invariants();
    }
}

#[test]
fn full_key_span_and_interior_key_deletion_match_reference() {
    let mut ours = concurrent_roaring::RoaringBitmap::new();
    let mut reference = RefBitmap::new();
    for key in 0..=u16::MAX as u32 {
        let x = (key << 16) | 17;
        ours.insert(x);
        reference.insert(x);
    }
    for key in 0..=u16::MAX as u32 {
        assert!(ours.contains((key << 16) | 17));
        assert!(!ours.contains(key << 16));
    }
    assert!(ours.remove((32768 << 16) | 17));
    reference.remove((32768 << 16) | 17);
    for key in 0..=u16::MAX as u32 {
        let x = (key << 16) | 17;
        assert_eq!(ours.contains(x), reference.contains(x));
    }
    ours.insert((32768 << 16) | 17);
    ours.optimize();
    ours.assert_invariants();
    for key in 0..=u16::MAX as u32 {
        assert!(ours.contains((key << 16) | 17));
    }
}

#[test]
fn sparse_benchmark_fills_high_key_space() {
    let values = data("sparse");
    let keys: BTreeSet<_> = values.iter().map(|x| x >> 16).collect();
    assert_eq!(keys.len(), u16::MAX as usize + 1);
    let unique: RefBitmap = values.into_iter().collect();
    assert_eq!(unique.len(), 999_872);
}
