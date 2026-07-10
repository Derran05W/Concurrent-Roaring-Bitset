//! P7: `ConcurrentRoaringBitmap` — a sharded `parking_lot::RwLock` wrapper (Wave 1). Each shard
//! owns a whole sequential `RoaringBitmap` holding only the keys that hash to it, so reads and
//! writes to different shards never contend.

use crate::bitmap::{split, RoaringBitmap};
use parking_lot::RwLock;

// One shard per 128-byte cache line (Apple Silicon's line size, and ≥ x86's 64 B adjacent-line
// prefetch pair). Unpadded, four 32-byte shards share one line, and even a *reader* RMWs its
// lock word to register itself — so readers of neighbouring shards false-share and the
// read-heavy scaling runs stall on line bouncing that has nothing to do with real contention.
#[repr(align(128))]
struct Shard(RwLock<RoaringBitmap>);

pub struct ConcurrentRoaringBitmap {
    shards: Box<[Shard]>,
    mask: usize, // num_shards - 1; num_shards is a power of two (§2.6), so this masks the key.
}

impl Default for ConcurrentRoaringBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentRoaringBitmap {
    pub fn new() -> Self {
        // 64 shards: the §2.6 default.
        Self::with_shard_count(64)
    }

    pub fn with_shard_count(n: usize) -> Self {
        // Power of two so `key & mask` is a valid uniform shard index (§2.6).
        assert!(n.is_power_of_two(), "shard count must be a power of two");
        let shards = (0..n)
            .map(|_| Shard(RwLock::new(RoaringBitmap::new())))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            mask: n - 1,
        }
    }

    fn shard(&self, key: u16) -> &RwLock<RoaringBitmap> {
        // Low bits of the key: real-world integer sets are typically clustered, so consecutive keys
        // must round-robin across shards; taking *high* bits would pile a clustered dataset into
        // shard 0 (§2.6).
        &self.shards[(key as usize) & self.mask].0
    }

    pub fn insert(&self, x: u32) -> bool {
        let (key, _) = split(x);
        // The inner map sees the full u32; it only ever stores this shard's keys.
        self.shard(key).write().insert(x)
    }

    pub fn remove(&self, x: u32) -> bool {
        let (key, _) = split(x);
        self.shard(key).write().remove(x)
    }

    pub fn contains(&self, x: u32) -> bool {
        let (key, _) = split(x);
        self.shard(key).read().contains(x)
    }

    pub fn len(&self) -> u64 {
        // Read-lock one shard at a time; never hold all locks at once. Consequence: this is
        // per-shard-atomic, not linearizable across shards — a concurrent writer to an
        // already-counted shard is not reflected (documented §2.6 semantic).
        self.shards.iter().map(|s| s.0.read().len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        // Same one-at-a-time discipline as `len`: per-shard-atomic, not globally linearizable.
        self.shards.iter().all(|s| s.0.read().is_empty())
    }

    pub fn snapshot(&self) -> RoaringBitmap {
        // Clone each shard under a brief read lock, release, then merge outside the locks. Shards
        // partition the key space by `key & mask`, so the clones are key-disjoint and merge by plain
        // concatenation + sort (see `RoaringBitmap::from_shards`) — no kernel merge. Per-shard-atomic,
        // not a single global point-in-time image (§2.6).
        let clones = self.shards.iter().map(|s| s.0.read().clone());
        RoaringBitmap::from_shards(clones.collect::<Vec<_>>())
    }

    pub fn and(&self, other: &Self) -> RoaringBitmap {
        // Snapshot both sides first, then run the sequential kernel. No two locks are ever held at
        // once and no lock spans both objects, so no lock-ordering deadlock is possible.
        self.snapshot().and(&other.snapshot())
    }

    pub fn or(&self, other: &Self) -> RoaringBitmap {
        // Same deadlock-freedom argument as `and`.
        self.snapshot().or(&other.snapshot())
    }

    pub fn optimize(&self) {
        for s in self.shards.iter() {
            s.0.write().optimize();
        }
    }
}
