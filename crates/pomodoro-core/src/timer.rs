use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

use crate::{ConfigError, SessionKind, TimerConfig};

/// Persistable internal timer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum TimerState {
    Idle { remaining_ms: u64 },
    Running { deadline_ms: u64 },
    Paused { remaining_ms: u64 },
}

/// Timer status without internal timing data, suitable for display and input handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerStatus {
    Idle,
    Running,
    Paused,
}

/// An action name used when reporting an invalid state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Pause,
    Resume,
}

/// An event emitted by the core for frontend consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TimerEvent {
    SessionCompleted {
        session: SessionKind,
        duration_seconds: u64,
        completed_at_ms: u64,
    },
    SessionSkipped {
        session: SessionKind,
    },
}

/// A UI-independent Pomodoro state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PomodoroTimer {
    config: TimerConfig,
    session: SessionKind,
    state: TimerState,
    completed_focuses_in_round: u32,
}

impl PomodoroTimer {
    /// Creates an idle focus session from a validated configuration.
    ///
    /// # Errors
    ///
    /// Returns [`TimerError::InvalidConfig`] if a configuration value is outside the
    /// supported range.
    pub fn new(config: TimerConfig) -> Result<Self, TimerError> {
        config.validate()?;
        Ok(Self {
            state: TimerState::Idle {
                remaining_ms: config.duration_millis(SessionKind::Focus),
            },
            config,
            session: SessionKind::Focus,
            completed_focuses_in_round: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &TimerConfig {
        &self.config
    }

    #[must_use]
    pub const fn session(&self) -> SessionKind {
        self.session
    }

    #[must_use]
    pub const fn state(&self) -> TimerState {
        self.state
    }

    #[must_use]
    pub const fn status(&self) -> TimerStatus {
        match self.state {
            TimerState::Idle { .. } => TimerStatus::Idle,
            TimerState::Running { .. } => TimerStatus::Running,
            TimerState::Paused { .. } => TimerStatus::Paused,
        }
    }

    #[must_use]
    pub const fn completed_focuses_in_round(&self) -> u32 {
        self.completed_focuses_in_round
    }

    /// Returns the remaining time in milliseconds at `now_ms`.
    #[must_use]
    pub const fn remaining_millis(&self, now_ms: u64) -> u64 {
        match self.state {
            TimerState::Idle { remaining_ms } | TimerState::Paused { remaining_ms } => remaining_ms,
            TimerState::Running { deadline_ms } => deadline_ms.saturating_sub(now_ms),
        }
    }

    /// Returns the remaining whole seconds, rounded up for display.
    #[must_use]
    pub const fn remaining_seconds(&self, now_ms: u64) -> u64 {
        self.remaining_millis(now_ms).div_ceil(1_000)
    }

    /// Starts an idle session.
    ///
    /// # Errors
    ///
    /// Returns [`TimerError`] if the timer is not idle or if calculating the deadline
    /// would overflow.
    pub fn start(&mut self, now_ms: u64) -> Result<(), TimerError> {
        let TimerState::Idle { remaining_ms } = self.state else {
            return Err(TimerError::InvalidTransition {
                action: Action::Start,
                status: self.status(),
            });
        };

        self.state = TimerState::Running {
            deadline_ms: deadline(now_ms, remaining_ms)?,
        };
        Ok(())
    }

    /// Pauses a running session.
    ///
    /// # Errors
    ///
    /// Returns [`TimerError`] if the timer is not running or the deadline has already
    /// elapsed.
    pub fn pause(&mut self, now_ms: u64) -> Result<(), TimerError> {
        let TimerState::Running { deadline_ms } = self.state else {
            return Err(TimerError::InvalidTransition {
                action: Action::Pause,
                status: self.status(),
            });
        };
        let remaining_ms = deadline_ms.saturating_sub(now_ms);
        if remaining_ms == 0 {
            return Err(TimerError::SessionAlreadyElapsed);
        }

        self.state = TimerState::Paused { remaining_ms };
        Ok(())
    }

    /// Resumes a paused session.
    ///
    /// # Errors
    ///
    /// Returns [`TimerError`] if the timer is not paused or if calculating the new
    /// deadline would overflow.
    pub fn resume(&mut self, now_ms: u64) -> Result<(), TimerError> {
        let TimerState::Paused { remaining_ms } = self.state else {
            return Err(TimerError::InvalidTransition {
                action: Action::Resume,
                status: self.status(),
            });
        };

        self.state = TimerState::Running {
            deadline_ms: deadline(now_ms, remaining_ms)?,
        };
        Ok(())
    }

    /// Restores the current session's full duration and returns it to idle.
    pub fn reset(&mut self) {
        self.state = TimerState::Idle {
            remaining_ms: self.config.duration_millis(self.session),
        };
    }

    /// Advances without counting the current session toward history.
    pub fn skip(&mut self) -> TimerEvent {
        let skipped = self.session;
        self.advance_session(false);
        TimerEvent::SessionSkipped { session: skipped }
    }

    /// Applies the current time and completes an elapsed session exactly once.
    pub fn tick(&mut self, now_ms: u64) -> Option<TimerEvent> {
        let TimerState::Running { deadline_ms } = self.state else {
            return None;
        };
        if now_ms < deadline_ms {
            return None;
        }

        let completed = self.session;
        let duration_seconds = self.config.duration_seconds(completed);
        self.advance_session(true);

        Some(TimerEvent::SessionCompleted {
            session: completed,
            duration_seconds,
            completed_at_ms: deadline_ms,
        })
    }

    /// Validates that deserialized state is consistent with its configuration.
    ///
    /// # Errors
    ///
    /// Returns [`TimerError`] if the configuration, completed focus count, or remaining
    /// time is invalid.
    pub fn validate(&self) -> Result<(), TimerError> {
        self.config.validate()?;
        if self.completed_focuses_in_round > self.config.focuses_before_long_break() {
            return Err(TimerError::InvalidCompletedFocusCount {
                found: self.completed_focuses_in_round,
                maximum: self.config.focuses_before_long_break(),
            });
        }

        let expected_maximum = self.config.duration_millis(self.session);
        match self.state {
            TimerState::Idle { remaining_ms } | TimerState::Paused { remaining_ms }
                if remaining_ms == 0 || remaining_ms > expected_maximum =>
            {
                Err(TimerError::InvalidRemainingTime {
                    found_ms: remaining_ms,
                    maximum_ms: expected_maximum,
                })
            }
            _ => Ok(()),
        }
    }

    fn advance_session(&mut self, completed: bool) {
        self.session = match self.session {
            SessionKind::Focus => {
                if completed {
                    self.completed_focuses_in_round += 1;
                }

                if self.completed_focuses_in_round >= self.config.focuses_before_long_break() {
                    SessionKind::LongBreak
                } else {
                    SessionKind::ShortBreak
                }
            }
            SessionKind::ShortBreak => SessionKind::Focus,
            SessionKind::LongBreak => {
                self.completed_focuses_in_round = 0;
                SessionKind::Focus
            }
        };
        self.state = TimerState::Idle {
            remaining_ms: self.config.duration_millis(self.session),
        };
    }
}

fn deadline(now_ms: u64, remaining_ms: u64) -> Result<u64, TimerError> {
    now_ms
        .checked_add(remaining_ms)
        .ok_or(TimerError::TimestampOverflow)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerError {
    InvalidConfig(ConfigError),
    InvalidTransition { action: Action, status: TimerStatus },
    SessionAlreadyElapsed,
    TimestampOverflow,
    InvalidCompletedFocusCount { found: u32, maximum: u32 },
    InvalidRemainingTime { found_ms: u64, maximum_ms: u64 },
}

impl fmt::Display for TimerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(error) => write!(formatter, "invalid timer config: {error}"),
            Self::InvalidTransition { action, status } => {
                write!(formatter, "cannot {action:?} timer while it is {status:?}")
            }
            Self::SessionAlreadyElapsed => {
                formatter.write_str("session has already elapsed; tick the timer first")
            }
            Self::TimestampOverflow => formatter.write_str("timer deadline overflowed"),
            Self::InvalidCompletedFocusCount { found, maximum } => write!(
                formatter,
                "completed focus count {found} exceeds configured maximum {maximum}"
            ),
            Self::InvalidRemainingTime {
                found_ms,
                maximum_ms,
            } => write!(
                formatter,
                "remaining time {found_ms} ms is outside 1..={maximum_ms} ms"
            ),
        }
    }
}

impl Error for TimerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidConfig(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ConfigError> for TimerError {
    fn from(error: ConfigError) -> Self {
        Self::InvalidConfig(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_config() -> TimerConfig {
        TimerConfig::new(10, 5, 20, 4).unwrap()
    }

    #[test]
    fn starts_pauses_resumes_and_completes_without_sleeping() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();

        assert_eq!(timer.status(), TimerStatus::Idle);
        assert_eq!(timer.remaining_seconds(1_000), 10);

        timer.start(1_000).unwrap();
        assert_eq!(timer.remaining_seconds(4_000), 7);

        timer.pause(4_000).unwrap();
        assert_eq!(timer.status(), TimerStatus::Paused);
        assert_eq!(timer.remaining_seconds(100_000), 7);

        timer.resume(10_000).unwrap();
        assert_eq!(timer.tick(16_999), None);
        assert_eq!(
            timer.tick(17_000),
            Some(TimerEvent::SessionCompleted {
                session: SessionKind::Focus,
                duration_seconds: 10,
                completed_at_ms: 17_000,
            })
        );
        assert_eq!(timer.session(), SessionKind::ShortBreak);
        assert_eq!(timer.status(), TimerStatus::Idle);
        assert_eq!(timer.remaining_seconds(17_000), 5);
        assert_eq!(timer.completed_focuses_in_round(), 1);
    }

    #[test]
    fn reset_restores_the_full_current_session() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();
        timer.start(0).unwrap();
        timer.pause(3_000).unwrap();

        timer.reset();

        assert_eq!(timer.status(), TimerStatus::Idle);
        assert_eq!(timer.remaining_seconds(100_000), 10);
    }

    #[test]
    fn skipped_focus_is_not_counted() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();

        assert_eq!(
            timer.skip(),
            TimerEvent::SessionSkipped {
                session: SessionKind::Focus
            }
        );
        assert_eq!(timer.completed_focuses_in_round(), 0);
        assert_eq!(timer.session(), SessionKind::ShortBreak);
    }

    #[test]
    fn fourth_completed_focus_is_followed_by_a_long_break() {
        let config = TimerConfig::new(1, 1, 1, 4).unwrap();
        let mut timer = PomodoroTimer::new(config).unwrap();
        let mut now = 0;

        for completed in 1..=4 {
            timer.start(now).unwrap();
            now += 1_000;
            assert!(matches!(
                timer.tick(now),
                Some(TimerEvent::SessionCompleted {
                    session: SessionKind::Focus,
                    ..
                })
            ));

            if completed < 4 {
                assert_eq!(timer.session(), SessionKind::ShortBreak);
                timer.skip();
                assert_eq!(timer.session(), SessionKind::Focus);
            }
        }

        assert_eq!(timer.session(), SessionKind::LongBreak);
        assert_eq!(timer.completed_focuses_in_round(), 4);

        timer.skip();
        assert_eq!(timer.session(), SessionKind::Focus);
        assert_eq!(timer.completed_focuses_in_round(), 0);
    }

    #[test]
    fn a_large_time_jump_completes_only_the_running_session() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();
        timer.start(1_000).unwrap();

        assert!(timer.tick(10_000_000).is_some());
        assert_eq!(timer.tick(10_000_000), None);
        assert_eq!(timer.session(), SessionKind::ShortBreak);
        assert_eq!(timer.status(), TimerStatus::Idle);
    }

    #[test]
    fn rejects_invalid_transitions() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();

        assert_eq!(
            timer.pause(0),
            Err(TimerError::InvalidTransition {
                action: Action::Pause,
                status: TimerStatus::Idle,
            })
        );
        timer.start(0).unwrap();
        assert_eq!(
            timer.start(0),
            Err(TimerError::InvalidTransition {
                action: Action::Start,
                status: TimerStatus::Running,
            })
        );
    }

    #[test]
    fn reports_timestamp_overflow_without_mutating_state() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();
        let original = timer.clone();

        assert_eq!(timer.start(u64::MAX), Err(TimerError::TimestampOverflow));
        assert_eq!(timer, original);
    }

    #[test]
    fn active_timer_round_trips_through_json() {
        let mut timer = PomodoroTimer::new(short_config()).unwrap();
        timer.start(1_000).unwrap();

        let json = serde_json::to_string(&timer).unwrap();
        let restored: PomodoroTimer = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, timer);
        assert_eq!(restored.validate(), Ok(()));
        assert_eq!(restored.remaining_seconds(4_000), 7);
    }
}
