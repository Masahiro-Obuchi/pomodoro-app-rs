//! Linux V1 storage boundary. Acquire a lock before load/initialization/recovery;
//! owning a lock alone does not authorize overwriting an unread or invalid file.
//! Load policy grants normal writes only after a valid V1/new-store check.
//! Normal atomic save retains a fixed candidate on failure. Retry and explicit
//! recovery follow in subsequent Phase 2 units.

mod atomic_save;
mod load;
mod location;
mod lock;

pub use atomic_save::{PendingSave, SaveError, SaveStage};
pub use load::{
    LoadError, LoadOutcome, LoadProblem, RecoveryCandidate, SavedState, WritableStorage,
};
pub use location::StorageLocation;
pub use lock::{LockedStorage, StorageLockError};
