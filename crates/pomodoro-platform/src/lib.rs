//! Time, persistence, and notification adapters for native environments.

mod notification;
// The V1 load boundary consumes decode; encode awaits atomic save.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "V1 encode is staged ahead of atomic save")
)]
mod schema_v1;
mod storage;
#[cfg(target_os = "linux")]
mod storage_v1;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage::{NativeStorage, PersistedState, StorageError};
#[cfg(target_os = "linux")]
pub use storage_v1::{
    LoadError, LoadOutcome, LoadProblem, LockedStorage, RecoveryCandidate, SavedState,
    StorageLocation, StorageLockError, WritableStorage,
};
pub use time::{TimeError, local_date_at, unix_time_millis};
