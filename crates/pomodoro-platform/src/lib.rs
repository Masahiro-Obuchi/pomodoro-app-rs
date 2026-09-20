//! Time, persistence, and notification adapters for native environments.

mod notification;
// The V1 storage boundary will consume this codec before the TUI cutover.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "V1 codec is staged ahead of the storage boundary")
)]
mod schema_v1;
mod storage;
#[cfg(target_os = "linux")]
mod storage_v1;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage::{NativeStorage, PersistedState, StorageError};
#[cfg(target_os = "linux")]
pub use storage_v1::{LockedStorage, StorageLocation, StorageLockError};
pub use time::{TimeError, local_date_at, unix_time_millis};
