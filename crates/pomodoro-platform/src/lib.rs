//! Time, persistence, and notification adapters for native environments.

mod notification;
// The V1 storage boundary will consume this codec before the TUI cutover.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "V1 codec is staged ahead of the storage boundary")
)]
mod schema_v1;
mod storage;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage::{NativeStorage, PersistedState, StorageError};
pub use time::{TimeError, local_date_at, unix_time_millis};
