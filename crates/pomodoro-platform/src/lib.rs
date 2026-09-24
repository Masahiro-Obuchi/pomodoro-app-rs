//! Time, persistence, and notification adapters for native environments.

mod notification;
// The native V1 file boundary is currently supported on Linux and macOS.
#[cfg_attr(
    not(any(test, target_os = "linux", target_os = "macos")),
    expect(dead_code, reason = "V1 storage is currently Unix-only")
)]
mod schema_v1;
mod storage_location;
mod storage_lock_error;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod storage_v1;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage_location::StorageLocation;
pub use storage_lock_error::StorageLockError;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use storage_v1::{
    LoadError, LoadOutcome, LoadProblem, LockedStorage, PendingSave, RecoveryCandidate,
    RecoverySave, SaveError, SaveStage, SavedState, WritableStorage,
};
pub use time::{ObservationClock, TimeError};
