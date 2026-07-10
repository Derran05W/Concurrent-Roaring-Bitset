//! Concurrent Roaring bitmap variants: P7 sharded `RwLock`, P8 lock-free reads.

pub mod sharded;
pub mod snapshot;

pub use sharded::ConcurrentRoaringBitmap;
pub use snapshot::SnapshotRoaringBitmap;
