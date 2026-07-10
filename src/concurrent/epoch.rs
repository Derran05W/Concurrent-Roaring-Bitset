//! `EpochRoaringBitmap`: lock-free reads via `crossbeam-epoch`. Each shard holds an immutable
//! `RoaringBitmap` behind an `Atomic` pointer; readers pin an epoch and load the pointer with no
//! lock, writers serialize on a per-shard mutex and publish a mutated clone (single-writer RCU).
//! Retired snapshots are reclaimed by epoch GC once no pinned reader can still observe them — the
//! manual-reclamation counterpart to P8a's `Arc` refcounting, with the same clone-per-write cost.

use crate::bitmap::{split, RoaringBitmap};
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};
use parking_lot::Mutex;
use std::sync::atomic::Ordering;

// 128-byte alignment gives each shard its own cache line (see the P8a note): unpadded, a writer's
// pointer swap would invalidate a line shared with readers of unrelated shards.
#[repr(align(128))]
struct Shard {
    current: Atomic<RoaringBitmap>,
    // Single-writer RCU serialization: two concurrent read-copy-update writers on one shard would
    // each clone the same base snapshot, and the second `swap` would silently discard the first's
    // update (a lost update). The mutex serializes the read-modify-write; readers never take it.
    write: Mutex<()>,
}

pub struct EpochRoaringBitmap {
    shards: Box<[Shard]>,
    mask: usize, // num_shards - 1; num_shards is a power of two, so this masks the key.
}

impl Default for EpochRoaringBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl EpochRoaringBitmap {
    pub fn new() -> Self {
        // 64 shards by default.
        Self::with_shard_count(64)
    }

    pub fn with_shard_count(n: usize) -> Self {
        // Power of two so `key & mask` is a valid uniform shard index.
        assert!(n.is_power_of_two(), "shard count must be a power of two");
        let shards = (0..n)
            .map(|_| Shard {
                current: Atomic::new(RoaringBitmap::new()),
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
        // Low bits of the key: real-world integer sets are typically clustered, so consecutive
        // keys round-robin across shards; high bits would pile a clustered dataset into shard 0.
        &self.shards[(key as usize) & self.mask]
    }

    pub fn contains(&self, x: u32) -> bool {
        let (key, _) = split(x);
        let guard = epoch::pin();
        // Acquire pairs with the writer's Release swap so a reader that sees the new pointer also
        // sees the fully-built clone behind it.
        let shared = self.shard(key).current.load(Ordering::Acquire, &guard);
        // SAFETY: the pointer is never null after construction (every shard is initialized with a
        // map and only ever swapped for another non-null map), and epoch pinning guarantees no
        // `defer_destroy`ed snapshot is freed while this guard is live.
        let map = unsafe { shared.deref() };
        map.contains(x)
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
        let _wguard = shard.write.lock();
        let guard = epoch::pin();
        // Acquire load under the writer lock: this thread is the only writer, but it must still
        // observe the latest published clone to build the next one on top of it.
        let shared = shard.current.load(Ordering::Acquire, &guard);
        // SAFETY: same invariant as `contains` — non-null after construction, kept alive by the pin.
        let cur = unsafe { shared.deref() };
        // No-op short-circuit: skip the O(shard) clone when the op changes nothing — this is what
        // keeps duplicate-heavy workloads sane (a repeated insert or an absent remove never
        // allocates a new snapshot).
        if cur.contains(x) == present {
            return false;
        }
        let mut next = cur.clone();
        let changed = if present {
            next.insert(x)
        } else {
            next.remove(x)
        };
        // Release publishes the completed clone before the pointer becomes visible to readers.
        let old = shard
            .current
            .swap(Owned::new(next), Ordering::Release, &guard);
        // SAFETY: after the swap no new reader can load `old`, and epoch GC waits out every reader
        // that pinned before the swap, so the retired snapshot outlives its last observer.
        unsafe {
            guard.defer_destroy(old);
        }
        changed
    }

    pub fn len(&self) -> u64 {
        // Load each shard's snapshot independently — per-shard-atomic, not linearizable across
        // shards: a concurrent writer to an already-counted shard is not reflected.
        let guard = epoch::pin();
        self.shards
            .iter()
            .map(|s| {
                let shared = s.current.load(Ordering::Acquire, &guard);
                // SAFETY: non-null after construction, kept alive by the pin.
                unsafe { shared.deref() }.len()
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        // Same per-shard-atomic discipline as `len`.
        let guard = epoch::pin();
        self.shards.iter().all(|s| {
            let shared = s.current.load(Ordering::Acquire, &guard);
            // SAFETY: non-null after construction, kept alive by the pin.
            unsafe { shared.deref() }.is_empty()
        })
    }

    pub fn snapshot(&self) -> RoaringBitmap {
        // Load and clone each shard's map under one pin — no writer locks. Shards partition the key
        // space by `key & mask`, so the clones are key-disjoint and reassemble by plain
        // concatenation + sort (`RoaringBitmap::from_shards`). Per-shard-atomic, not a global image.
        let guard = epoch::pin();
        RoaringBitmap::from_shards(self.shards.iter().map(|s| {
            let shared = s.current.load(Ordering::Acquire, &guard);
            // SAFETY: non-null after construction, kept alive by the pin.
            unsafe { shared.deref() }.clone()
        }))
    }

    pub fn and(&self, other: &Self) -> RoaringBitmap {
        // Snapshot both sides, then run the sequential kernel. The read/snapshot path takes no
        // writer locks, so no lock-ordering deadlock across the two objects is possible.
        self.snapshot().and(&other.snapshot())
    }

    pub fn or(&self, other: &Self) -> RoaringBitmap {
        // Same deadlock-freedom argument as `and`.
        self.snapshot().or(&other.snapshot())
    }

    pub fn optimize(&self) {
        // `optimize` mutates, so it goes through the RCU write path: serialize on the shard mutex,
        // clone, optimize the clone, swap it in, retire the old snapshot.
        for s in self.shards.iter() {
            let _wguard = s.write.lock();
            let guard = epoch::pin();
            let shared = s.current.load(Ordering::Acquire, &guard);
            // SAFETY: non-null after construction, kept alive by the pin.
            let mut next = unsafe { shared.deref() }.clone();
            next.optimize();
            let old = s.current.swap(Owned::new(next), Ordering::Release, &guard);
            // SAFETY: same argument as `update` — no new reader can load `old` after the swap.
            unsafe {
                guard.defer_destroy(old);
            }
        }
    }
}

impl Drop for EpochRoaringBitmap {
    fn drop(&mut self) {
        let guard = epoch::pin();
        for s in self.shards.iter() {
            let old = s.current.swap(Shared::null(), Ordering::Relaxed, &guard);
            if !old.is_null() {
                // SAFETY: `&mut self` in `Drop` proves no concurrent readers exist, so the retired
                // snapshot can be freed immediately rather than deferred to epoch GC.
                unsafe {
                    drop(old.into_owned());
                }
            }
        }
    }
}
