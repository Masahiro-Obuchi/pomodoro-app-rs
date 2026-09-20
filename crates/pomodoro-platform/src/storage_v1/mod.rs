//! Linux V1 storage boundary. Acquire a lock before load/initialization/recovery;
//! owning a lock alone does not authorize overwriting an unread or invalid file.
//! Load policy grants normal writes only after a valid V1/new-store check.
//! Atomic save and explicit recovery follow in subsequent Phase 2 units.

mod load;
mod location;
mod lock;

pub use load::{
    LoadError, LoadOutcome, LoadProblem, RecoveryCandidate, SavedState, WritableStorage,
};
pub use location::StorageLocation;
pub use lock::{LockedStorage, StorageLockError};
