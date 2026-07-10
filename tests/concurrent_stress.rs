//! P7 concurrent stress tests for `ConcurrentRoaringBitmap`. `loom` is out of scope (§0.3); the
//! risk is covered by the simplicity of the lock pattern plus these two stress patterns.

use concurrent_roaring::bitmap::datasets;
use concurrent_roaring::ConcurrentRoaringBitmap;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::BTreeSet;
use std::hint::black_box;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Disjoint-partition lost-update detector: 8 threads each own a residue class of the clustered
/// dataset (thread `t` inserts every value `v` with `v % 8 == t`), so no two threads ever insert
/// the same value. If the sharded locking dropped an update, the final cardinality would fall short
/// of the dataset's unique count.
#[test]
fn disjoint_partition_no_lost_update() {
    let data = Arc::new(datasets::clustered());
    let unique: BTreeSet<u32> = data.iter().copied().collect();
    let map = Arc::new(ConcurrentRoaringBitmap::new());

    let mut handles = Vec::new();
    for t in 0..8u32 {
        let map = Arc::clone(&map);
        let data = Arc::clone(&data);
        handles.push(thread::spawn(move || {
            for &v in data.iter() {
                if v % 8 == t {
                    map.insert(v);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(
        map.len(),
        unique.len() as u64,
        "lost updates: cardinality short"
    );

    // 10,000 sampled unique values must all be present.
    let uniq_vec: Vec<u32> = unique.iter().copied().collect();
    let mut rng = StdRng::seed_from_u64(0x05A1_11E5);
    for _ in 0..10_000 {
        let v = uniq_vec[rng.random_range(0..uniq_vec.len())];
        assert!(map.contains(v), "sampled value {v} missing");
    }

    map.snapshot().assert_invariants();
}

/// Contended smoke test: 4 writers (random insert/remove) + 4 readers (contains) hammer the same
/// key range for 2 seconds. No panics, and the final snapshot must satisfy every invariant.
#[test]
fn contended_smoke() {
    let map = Arc::new(ConcurrentRoaringBitmap::new());
    let deadline = Instant::now() + Duration::from_secs(2);

    let mut handles = Vec::new();
    for t in 0..4u64 {
        let map = Arc::clone(&map);
        handles.push(thread::spawn(move || {
            let mut rng = StdRng::seed_from_u64(0x0000_0117 ^ t);
            while Instant::now() < deadline {
                let v = rng.random_range(0..2_000_000);
                if rng.random_bool(0.5) {
                    map.insert(v);
                } else {
                    map.remove(v);
                }
            }
        }));
    }
    for t in 0..4u64 {
        let map = Arc::clone(&map);
        handles.push(thread::spawn(move || {
            let mut rng = StdRng::seed_from_u64(0x0000_0DED ^ t);
            while Instant::now() < deadline {
                let v = rng.random_range(0..2_000_000);
                black_box(map.contains(v));
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    map.snapshot().assert_invariants();
}
