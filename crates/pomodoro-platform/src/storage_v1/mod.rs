//! Linux V1 storage boundary. Acquire a lock before load/initialization/recovery;
//! owning a lock alone does not authorize overwriting an unread or invalid file.
//! Load policy and atomic save are implemented in subsequent Phase 2 units.

mod location;
mod lock;

pub use location::StorageLocation;
pub use lock::{LockedStorage, StorageLockError};
