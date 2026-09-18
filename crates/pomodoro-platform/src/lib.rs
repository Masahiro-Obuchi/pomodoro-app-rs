//! Time, persistence, and notification adapters for native environments.

mod notification;
// Phase 2-3 will consume these private values in the complete V1 envelope.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "V1 value codecs are staged ahead of the full envelope"
    )
)]
mod schema_v1;
mod storage;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage::{NativeStorage, PersistedState, StorageError};
pub use time::{TimeError, local_date_at, unix_time_millis};
