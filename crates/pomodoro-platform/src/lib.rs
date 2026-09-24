//! Time, persistence, and notification adapters for native environments.

mod notification;
// The native V1 file boundary is currently Linux-only.
#[cfg_attr(
    not(any(test, target_os = "linux")),
    expect(dead_code, reason = "V1 storage is currently Linux-only")
)]
mod schema_v1;
mod storage_location;
mod storage_lock_error;
#[cfg(target_os = "linux")]
mod storage_v1;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage_location::StorageLocation;
pub use storage_lock_error::StorageLockError;
#[cfg(target_os = "linux")]
pub use storage_v1::{
    LoadError, LoadOutcome, LoadProblem, LockedStorage, PendingSave, RecoveryCandidate,
    RecoverySave, SaveError, SaveStage, SavedState, WritableStorage,
};
pub use time::{ObservationClock, TimeError};
