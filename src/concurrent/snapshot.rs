//! P8a: `SnapshotRoaringBitmap` — lock-free reads via `arc-swap` (Wave 2). Each shard holds an
//! immutable `RoaringBitmap` behind an `ArcSwap`; readers load the current pointer with no lock,
//! writers serialize on a per-shard mutex and publish a mutated clone (single-writer RCU). The
//! clone-per-write cost is the deliberate tradeoff being measured (a read-optimized structure).

use crate::bitmap::{split, RoaringBitmap};
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use std::sync::Arc;

struct Shard {
    current: ArcSwap<RoaringBitmap>,
    // Single-writer RCU serialization: two concurrent read-copy-update writers on one shard would
    // each clone the same base snapshot, and the second `store` would silently discard the first's
    // update (a lost update). The mutex serializes the read-modify-write; readers never take it and
    // are unaffected by it.
    write: Mutex<()>,
}

pub struct SnapshotRoaringBitmap {
    shards: Box<[Shard]>,
    mask: usize, // num_shards - 1; num_shards is a power of two (§2.6), so this masks the key.
}

impl Default for SnapshotRoaringBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotRoaringBitmap {
    pub fn new() -> Self {
        // 64 shards: the §2.6 default.
        Self::with_shard_count(64)
    }

    pub fn with_shard_count(n: usize) -> Self {
        // Power of two so `key & mask` is a valid uniform shard index (§2.6).
        assert!(n.is_power_of_two(), "shard count must be a power of two");
        let shards = (0..n)
            .map(|_| Shard {
                current: ArcSwap::from_pointee(RoaringBitmap::new()),
                write: Mutex::new(()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            mask: n - 1,
        }
    }

    fn shard(&self, key: u16) -> &Shard {
        // Low bits of the key: real-world integer sets are typically clustered, so consecutive keys
        // must round-robin across shards; taking *high* bits would pile a clustered dataset into
        // shard 0 (§2.6).
        &self.shards[(key as usize) & self.mask]
    }

    pub fn contains(&self, x: u32) -> bool {
        let (key, _) = split(x);
        // `load()` returns a cheap guard — no full `Arc` clone on the read hot path.
        let g = self.shard(key).current.load();
        g.contains(x)
    }

    pub fn insert(&self, x: u32) -> bool {
        self.update(x, true)
    }

    pub fn remove(&self, x: u32) -> bool {
        self.update(x, false)
    }

    /// Shared RCU write path. `present` is the membership the op wants to achieve for `x`
    /// (`true` = insert, `false` = remove). Returns whether the structure changed.
    fn update(&self, x: u32, present: bool) -> bool {
        let (key, _) = split(x);
        let shard = self.shard(key);
        let _guard = shard.write.lock();
        let cur = shard.current.load_full();
        // No-op short-circuit: skip the O(shard) clone when the op changes nothing — this is what
        // keeps duplicate-heavy workloads sane (a repeated insert or an absent remove never
        // allocates a new snapshot).
        if cur.contains(x) == present {
            return false;
        }
        let mut next = (*cur).clone();
        let changed = if present {
            next.insert(x)
        } else {
            next.remove(x)
        };
        shard.current.store(Arc::new(next));
        changed
    }

    pub fn len(&self) -> u64 {
        // Load each shard's snapshot independently — per-shard-atomic, not linearizable across
        // shards: a concurrent writer to an already-counted shard is not reflected (§2.6).
        self.shards.iter().map(|s| s.current.load().len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        // Same per-shard-atomic discipline as `len` (§2.6).
        self.shards.iter().all(|s| s.current.load().is_empty())
    }

    pub fn snapshot(&self) -> RoaringBitmap {
        // Load each shard's immutable `Arc` — no locks at all. Shards partition the key space by
        // `key & mask`, so the clones are key-disjoint and reassemble by plain concatenation + sort
        // (`RoaringBitmap::from_shards`). Per-shard-atomic, not a single global image (§2.6).
        RoaringBitmap::from_shards(self.shards.iter().map(|s| (*s.current.load_full()).clone()))
    }

    pub fn and(&self, other: &Self) -> RoaringBitmap {
        // Snapshot both sides, then run the sequential kernel. The read/snapshot path takes no
        // locks at all, so no lock-ordering deadlock across the two objects is possible.
        self.snapshot().and(&other.snapshot())
    }

    pub fn or(&self, other: &Self) -> RoaringBitmap {
        // Same deadlock-freedom argument as `and`.
        self.snapshot().or(&other.snapshot())
    }

    pub fn optimize(&self) {
        // `optimize` mutates, so it goes through the RCU write path: serialize on the shard mutex,
        // clone, optimize the clone, publish.
        for s in self.shards.iter() {
            let _guard = s.write.lock();
            let mut next = (*s.current.load_full()).clone();
            next.optimize();
            s.current.store(Arc::new(next));
        }
    }
}
