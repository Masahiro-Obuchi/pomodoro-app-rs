//! Existing timer API, isolated until the Phase 2 V1 storage cutover.
//!
//! This is not a migration layer: no conversion of saved state or synchronization
//! with the new domain is provided. Existing platform/TUI callers use this module
//! explicitly so the workspace stays buildable during the cutover.

#[path = "history.rs"]
mod history;
#[path = "session.rs"]
mod session;
#[path = "timer.rs"]
mod timer;

pub use history::{CURRENT_HISTORY_SCHEMA_VERSION, DailySummary, History, HistoryError};
pub use session::SessionKind;
pub use timer::{Action, PomodoroTimer, TimerError, TimerEvent, TimerState, TimerStatus};

impl From<SessionKind> for crate::SessionKind {
    fn from(kind: SessionKind) -> Self {
        match kind {
            SessionKind::Focus => Self::Focus,
            SessionKind::ShortBreak => Self::ShortBreak,
            SessionKind::LongBreak => Self::LongBreak,
        }
    }
}
