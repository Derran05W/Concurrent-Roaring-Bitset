use concurrent_roaring::bitmap::datasets;
use concurrent_roaring::{
    ConcurrentRoaringBitmap, EpochRoaringBitmap, RoaringBitmap, SnapshotRoaringBitmap,
};
use parking_lot::RwLock;
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use std::collections::{BTreeSet, HashSet};

pub const SEED: u64 = 0xFEED_BEEF;
// Match the public default so payload comparisons have identical shard partitioning.
pub const SHARDS: usize = 64;
pub const DATASETS: [&str; 4] = ["dense", "sparse", "clustered", "balanced"];

pub fn data(name: &str) -> Vec<u32> {
    match name {
        "dense" => datasets::dense(),
        "sparse" => datasets::sparse(),
        "clustered" => datasets::clustered(),
        "balanced" => (0..2_000_000u32)
            .map(|i| ((i % SHARDS as u32) << 16) | (i / SHARDS as u32))
            .collect(),
        _ => panic!("unknown dataset: {name}"),
    }
}

pub trait Sequential: Default + Clone {
    fn insert(&mut self, x: u32) -> bool;
    fn remove(&mut self, x: u32) -> bool;
    fn contains(&self, x: u32) -> bool;
    fn len(&self) -> u64;
    fn optimize(&mut self);
    fn and(&self, rhs: &Self) -> Self;
    fn or(&self, rhs: &Self) -> Self;
    fn validate(&self) {}
}

impl Sequential for RoaringBitmap {
    fn insert(&mut self, x: u32) -> bool {
        self.insert(x)
    }
    fn remove(&mut self, x: u32) -> bool {
        self.remove(x)
    }
    fn contains(&self, x: u32) -> bool {
        self.contains(x)
    }
    fn len(&self) -> u64 {
        self.len()
    }
    fn optimize(&mut self) {
        self.optimize()
    }
    fn and(&self, rhs: &Self) -> Self {
        self.and(rhs)
    }
    fn or(&self, rhs: &Self) -> Self {
        self.or(rhs)
    }
    fn validate(&self) {
        self.assert_invariants()
    }
}
impl Sequential for roaring::RoaringBitmap {
    fn insert(&mut self, x: u32) -> bool {
        self.insert(x)
    }
    fn remove(&mut self, x: u32) -> bool {
        self.remove(x)
    }
    fn contains(&self, x: u32) -> bool {
        self.contains(x)
    }
    fn len(&self) -> u64 {
        self.len()
    }
    fn optimize(&mut self) {
        self.optimize();
    }
    fn and(&self, rhs: &Self) -> Self {
        self & rhs
    }
    fn or(&self, rhs: &Self) -> Self {
        self | rhs
    }
}
impl Sequential for croaring::Bitmap {
    fn insert(&mut self, x: u32) -> bool {
        self.add_checked(x)
    }
    fn remove(&mut self, x: u32) -> bool {
        self.remove_checked(x)
    }
    fn contains(&self, x: u32) -> bool {
        self.contains(x)
    }
    fn len(&self) -> u64 {
        self.cardinality()
    }
    fn optimize(&mut self) {
        self.run_optimize();
    }
    fn and(&self, rhs: &Self) -> Self {
        self.and(rhs)
    }
    fn or(&self, rhs: &Self) -> Self {
        self.or(rhs)
    }
}
macro_rules! ordinary_set {
    ($t:ty) => {
        impl Sequential for $t {
            fn insert(&mut self, x: u32) -> bool {
                self.insert(x)
            }
            fn remove(&mut self, x: u32) -> bool {
                self.remove(&x)
            }
            fn contains(&self, x: u32) -> bool {
                self.contains(&x)
            }
            fn len(&self) -> u64 {
                self.len() as u64
            }
            fn optimize(&mut self) {}
            fn and(&self, rhs: &Self) -> Self {
                self.intersection(rhs).copied().collect()
            }
            fn or(&self, rhs: &Self) -> Self {
                self.union(rhs).copied().collect()
            }
        }
    };
}
ordinary_set!(HashSet<u32>);
ordinary_set!(BTreeSet<u32>);

pub fn build<B: Sequential>(values: &[u32], optimized: bool) -> B {
    let mut b = B::default();
    for &v in values {
        b.insert(v);
    }
    if optimized {
        b.optimize();
    }
    b
}

pub fn probes(values: &[u32], hit_percent: usize, count: usize) -> Vec<u32> {
    let oracle: roaring::RoaringBitmap = values.iter().copied().collect();
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut result = Vec::with_capacity(count);
    for _ in 0..count * hit_percent / 100 {
        result.push(values[rng.random_range(0..values.len())]);
    }
    while result.len() < count {
        let x = rng.random();
        if !oracle.contains(x) {
            result.push(x);
        }
    }
    result.shuffle(&mut rng);
    result
}

pub fn pair(values: &[u32], overlap: usize, skew: usize) -> (Vec<u32>, Vec<u32>) {
    let mut unique = values.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let n = unique.len() / 2;
    let m = n / skew;
    let shared = m * overlap / 100;
    let a = unique[..n].to_vec();
    let b = unique[..shared]
        .iter()
        .chain(&unique[n..n + m - shared])
        .copied()
        .collect();
    (a, b)
}

pub fn verify<B: Sequential>(b: &B, expected: &roaring::RoaringBitmap) {
    b.validate();
    assert_eq!(b.len(), expected.len());
    assert!(expected.iter().all(|x| b.contains(x)));
}

pub trait Concurrent: Default + Sync {
    fn insert(&self, x: u32) -> bool;
    fn remove(&self, x: u32) -> bool;
    fn contains(&self, x: u32) -> bool;
    fn len(&self) -> u64;
    fn optimize(&self);
}
macro_rules! ours_concurrent {
    ($t:ty) => {
        impl Concurrent for $t {
            fn insert(&self, x: u32) -> bool {
                self.insert(x)
            }
            fn remove(&self, x: u32) -> bool {
                self.remove(x)
            }
            fn contains(&self, x: u32) -> bool {
                self.contains(x)
            }
            fn len(&self) -> u64 {
                self.len()
            }
            fn optimize(&self) {
                self.optimize()
            }
        }
    };
}
ours_concurrent!(ConcurrentRoaringBitmap);
ours_concurrent!(SnapshotRoaringBitmap);
ours_concurrent!(EpochRoaringBitmap);

// Match the production padding so false sharing does not handicap the references.
#[repr(align(128))]
struct Padded<T>(RwLock<T>);
macro_rules! sharded_reference {
    ($name:ident, $payload:ty) => {
        pub struct $name {
            shards: Box<[Padded<$payload>]>,
        }
        impl Default for $name {
            fn default() -> Self {
                Self {
                    shards: (0..SHARDS)
                        .map(|_| Padded(RwLock::new(<$payload>::default())))
                        .collect(),
                }
            }
        }
        impl $name {
            fn shard(&self, x: u32) -> &RwLock<$payload> {
                // Low key bits spread clustered consecutive keys, exactly as in production.
                &self.shards[((x >> 16) as usize) & (SHARDS - 1)].0
            }
        }
        impl Concurrent for $name {
            fn insert(&self, x: u32) -> bool {
                Sequential::insert(&mut *self.shard(x).write(), x)
            }
            fn remove(&self, x: u32) -> bool {
                Sequential::remove(&mut *self.shard(x).write(), x)
            }
            fn contains(&self, x: u32) -> bool {
                Sequential::contains(&*self.shard(x).read(), x)
            }
            fn len(&self) -> u64 {
                // Per-shard atomic only, matching the production API; called after joining writers.
                self.shards
                    .iter()
                    .map(|s| Sequential::len(&*s.0.read()))
                    .sum()
            }
            fn optimize(&self) {
                for s in &self.shards {
                    Sequential::optimize(&mut *s.0.write());
                }
            }
        }
    };
}
sharded_reference!(ShardedRoaring, roaring::RoaringBitmap);
sharded_reference!(ShardedCroaring, croaring::Bitmap);
impl Concurrent for dashmap::DashSet<u32> {
    fn insert(&self, x: u32) -> bool {
        self.insert(x)
    }
    fn remove(&self, x: u32) -> bool {
        self.remove(&x).is_some()
    }
    fn contains(&self, x: u32) -> bool {
        self.contains(&x)
    }
    fn len(&self) -> u64 {
        self.len() as u64
    }
    fn optimize(&self) {}
}
