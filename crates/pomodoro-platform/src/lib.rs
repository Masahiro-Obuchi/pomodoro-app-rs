//! ネイティブ環境向けの時刻、保存、通知アダプター。

mod notification;
mod storage;
mod time;

pub use notification::{NotificationError, NotifySendNotifier};
pub use storage::{NativeStorage, PersistedState, StorageError};
pub use time::{TimeError, local_date_at, unix_time_millis};
