//! Time, persistence, and notification adapters for native environments.

mod notification;
mod schema_v1;
mod storage_location;
mod storage_lock_error;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod storage_v1;
mod time;

pub use notification::{DesktopNotifier, NotificationError};
pub use storage_location::StorageLocation;
pub use storage_lock_error::StorageLockError;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use storage_v1::{
    LoadError, LoadOutcome, LoadProblem, LockedStorage, PendingSave, RecoveryCandidate,
    RecoverySave, SaveError, SaveStage, SavedState, WritableStorage,
};
pub use time::{ObservationClock, TimeError};
