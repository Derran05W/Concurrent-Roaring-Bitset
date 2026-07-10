//! Concurrent stress tests for each concurrent type. `loom` model checking is out of scope; the
//! risk is covered by the simplicity of each lock/RCU pattern plus these two stress patterns.
//! Both patterns are stamped against every concurrent type by the `stress_suite!` macro so the
//! bodies are written once rather than triplicated.

use concurrent_roaring::bitmap::datasets;
use concurrent_roaring::{ConcurrentRoaringBitmap, SnapshotRoaringBitmap};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::BTreeSet;
use std::hint::black_box;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Stamps the two stress patterns for one concurrent type into its own module. `$ty` must expose
/// `new`, `insert`, `remove`, `contains`, `len`, and `snapshot` — the shared concurrent surface.
macro_rules! stress_suite {
    ($name:ident, $ty:ty) => {
        mod $name {
            use super::*;

            /// Disjoint-partition lost-update detector: 8 threads each own a residue class of the
            /// clustered dataset (thread `t` inserts every value `v` with `v % 8 == t`), so no two
            /// threads ever insert the same value. If the concurrency machinery dropped an update,
            /// the final cardinality would fall short of the dataset's unique count.
            #[test]
            fn disjoint_partition_no_lost_update() {
                let data = Arc::new(datasets::clustered());
                let unique: BTreeSet<u32> = data.iter().copied().collect();
                let map = Arc::new(<$ty>::new());

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

            /// Contended smoke test: 4 writers (random insert/remove) + 4 readers (contains) hammer
            /// the same key range for 2 seconds. No panics, and the final snapshot must satisfy
            /// every invariant.
            #[test]
            fn contended_smoke() {
                let map = Arc::new(<$ty>::new());
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
        }
    };
}

stress_suite!(sharded, ConcurrentRoaringBitmap);
stress_suite!(snapshot, SnapshotRoaringBitmap);
