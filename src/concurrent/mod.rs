//! Concurrent Roaring bitmap variants: a sharded `RwLock` type, and a lock-free-read type.

pub mod epoch;
pub mod sharded;
pub mod snapshot;

pub use epoch::EpochRoaringBitmap;
pub use sharded::ConcurrentRoaringBitmap;
pub use snapshot::SnapshotRoaringBitmap;
