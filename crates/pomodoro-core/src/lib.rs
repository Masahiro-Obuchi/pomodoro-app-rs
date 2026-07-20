//! UI- and operating-system-independent Pomodoro timer logic.

mod config;
mod history;
mod session;
mod timer;

pub use config::{ConfigError, TimerConfig};
pub use history::{CURRENT_HISTORY_SCHEMA_VERSION, DailySummary, History, HistoryError};
pub use session::SessionKind;
pub use timer::{Action, PomodoroTimer, TimerError, TimerEvent, TimerState, TimerStatus};
