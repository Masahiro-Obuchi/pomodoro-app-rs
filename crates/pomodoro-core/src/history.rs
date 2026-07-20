use std::{collections::BTreeMap, error::Error, fmt};

use serde::{Deserialize, Serialize};

use crate::{SessionKind, TimerEvent};

pub const CURRENT_HISTORY_SCHEMA_VERSION: u32 = 1;

/// A lightweight summary for one local calendar day.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailySummary {
    pub completed_focus_sessions: u32,
    pub focused_seconds: u64,
}

/// Versioned daily history keyed by dates in `YYYY-MM-DD` format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    schema_version: u32,
    days: BTreeMap<String, DailySummary>,
}

impl History {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub fn days(&self) -> &BTreeMap<String, DailySummary> {
        &self.days
    }

    #[must_use]
    pub fn summary(&self, local_date: &str) -> Option<&DailySummary> {
        self.days.get(local_date)
    }

    /// Records only completed focus-session events in the daily history.
    ///
    /// Returns `true` when the event was recorded and `false` when a completed break
    /// or skipped session was ignored.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError`] if the date is not in `YYYY-MM-DD` format or if an
    /// aggregate value would overflow.
    pub fn record_event(
        &mut self,
        local_date: &str,
        event: &TimerEvent,
    ) -> Result<bool, HistoryError> {
        let TimerEvent::SessionCompleted {
            session: SessionKind::Focus,
            duration_seconds,
            ..
        } = event
        else {
            return Ok(false);
        };

        self.record_focus(local_date, *duration_seconds)?;
        Ok(true)
    }

    /// Adds a completed focus session directly to the daily history.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError`] if the date is not in `YYYY-MM-DD` format or if an
    /// aggregate value would overflow.
    pub fn record_focus(
        &mut self,
        local_date: &str,
        duration_seconds: u64,
    ) -> Result<(), HistoryError> {
        validate_local_date(local_date)?;

        let current = self.days.get(local_date).copied().unwrap_or_default();
        let completed_focus_sessions = current
            .completed_focus_sessions
            .checked_add(1)
            .ok_or(HistoryError::CounterOverflow)?;
        let focused_seconds = current
            .focused_seconds
            .checked_add(duration_seconds)
            .ok_or(HistoryError::CounterOverflow)?;
        self.days.insert(
            local_date.to_owned(),
            DailySummary {
                completed_focus_sessions,
                focused_seconds,
            },
        );
        Ok(())
    }

    /// Validates deserialized history before it is used.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError`] if the schema version is unsupported or a date key is
    /// invalid.
    pub fn validate(&self) -> Result<(), HistoryError> {
        if self.schema_version != CURRENT_HISTORY_SCHEMA_VERSION {
            return Err(HistoryError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: CURRENT_HISTORY_SCHEMA_VERSION,
            });
        }

        for date in self.days.keys() {
            validate_local_date(date)?;
        }
        Ok(())
    }
}

impl Default for History {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_HISTORY_SCHEMA_VERSION,
            days: BTreeMap::new(),
        }
    }
}

fn validate_local_date(local_date: &str) -> Result<(), HistoryError> {
    let bytes = local_date.as_bytes();
    let has_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());

    if has_shape {
        Ok(())
    } else {
        Err(HistoryError::InvalidLocalDate(local_date.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    InvalidLocalDate(String),
    CounterOverflow,
    UnsupportedSchemaVersion { found: u32, supported: u32 },
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLocalDate(date) => {
                write!(formatter, "local date must use YYYY-MM-DD format: {date}")
            }
            Self::CounterOverflow => formatter.write_str("history counter overflowed"),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "unsupported history schema version {found}; expected {supported}"
            ),
        }
    }
}

impl Error for HistoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus_completed(duration_seconds: u64) -> TimerEvent {
        TimerEvent::SessionCompleted {
            session: SessionKind::Focus,
            duration_seconds,
            completed_at_ms: 1_000,
        }
    }

    #[test]
    fn aggregates_completed_focus_sessions_by_day() {
        let mut history = History::new();

        assert_eq!(
            history.record_event("2026-07-21", &focus_completed(1_500)),
            Ok(true)
        );
        assert_eq!(
            history.record_event("2026-07-21", &focus_completed(600)),
            Ok(true)
        );

        assert_eq!(
            history.summary("2026-07-21"),
            Some(&DailySummary {
                completed_focus_sessions: 2,
                focused_seconds: 2_100,
            })
        );
    }

    #[test]
    fn ignores_breaks_and_skipped_sessions() {
        let mut history = History::new();
        let break_event = TimerEvent::SessionCompleted {
            session: SessionKind::ShortBreak,
            duration_seconds: 300,
            completed_at_ms: 1_000,
        };
        let skipped_event = TimerEvent::SessionSkipped {
            session: SessionKind::Focus,
        };

        assert_eq!(history.record_event("2026-07-21", &break_event), Ok(false));
        assert_eq!(
            history.record_event("2026-07-21", &skipped_event),
            Ok(false)
        );
        assert!(history.days().is_empty());
    }

    #[test]
    fn round_trips_through_the_documented_json_shape() {
        let mut history = History::new();
        history.record_focus("2026-07-21", 1_500).unwrap();

        let json = serde_json::to_string_pretty(&history).unwrap();
        let restored: History = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, history);
        assert_eq!(restored.validate(), Ok(()));
        assert!(json.contains("\"schema_version\": 1"));
    }

    #[test]
    fn rejects_dates_with_the_wrong_shape() {
        let mut history = History::new();

        assert_eq!(
            history.record_focus("21/07/2026", 1_500),
            Err(HistoryError::InvalidLocalDate("21/07/2026".to_owned()))
        );
    }

    #[test]
    fn overflow_does_not_partially_update_a_summary() {
        let mut history = History::new();
        history.days.insert(
            "2026-07-21".to_owned(),
            DailySummary {
                completed_focus_sessions: 7,
                focused_seconds: u64::MAX,
            },
        );

        assert_eq!(
            history.record_focus("2026-07-21", 1),
            Err(HistoryError::CounterOverflow)
        );
        assert_eq!(
            history.summary("2026-07-21"),
            Some(&DailySummary {
                completed_focus_sessions: 7,
                focused_seconds: u64::MAX,
            })
        );
    }
}
