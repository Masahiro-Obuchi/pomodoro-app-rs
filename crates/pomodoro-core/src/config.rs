use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

use crate::SessionKind;

const MAX_SESSION_SECONDS: u64 = 24 * 60 * 60;

/// 各セッションの長さと、長休憩に入るまでの集中回数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerConfig {
    focus_seconds: u64,
    short_break_seconds: u64,
    long_break_seconds: u64,
    focuses_before_long_break: u32,
}

impl TimerConfig {
    /// 検証済みの設定を作る。
    ///
    /// # Errors
    ///
    /// セッション時間が0秒または24時間を超える場合、および長休憩までの
    /// 集中回数が0の場合に[`ConfigError`]を返す。
    pub fn new(
        focus_seconds: u64,
        short_break_seconds: u64,
        long_break_seconds: u64,
        focuses_before_long_break: u32,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            focus_seconds,
            short_break_seconds,
            long_break_seconds,
            focuses_before_long_break,
        };
        config.validate()?;
        Ok(config)
    }

    /// デシリアライズした設定も含め、値が利用可能か検証する。
    ///
    /// # Errors
    ///
    /// セッション時間または長休憩までの集中回数が許容範囲外の場合に
    /// [`ConfigError`]を返す。
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_duration("focus_seconds", self.focus_seconds)?;
        validate_duration("short_break_seconds", self.short_break_seconds)?;
        validate_duration("long_break_seconds", self.long_break_seconds)?;

        if self.focuses_before_long_break == 0 {
            return Err(ConfigError::ZeroFocusesBeforeLongBreak);
        }

        Ok(())
    }

    #[must_use]
    pub const fn focus_seconds(&self) -> u64 {
        self.focus_seconds
    }

    #[must_use]
    pub const fn short_break_seconds(&self) -> u64 {
        self.short_break_seconds
    }

    #[must_use]
    pub const fn long_break_seconds(&self) -> u64 {
        self.long_break_seconds
    }

    #[must_use]
    pub const fn focuses_before_long_break(&self) -> u32 {
        self.focuses_before_long_break
    }

    #[must_use]
    pub const fn duration_seconds(&self, session: SessionKind) -> u64 {
        match session {
            SessionKind::Focus => self.focus_seconds,
            SessionKind::ShortBreak => self.short_break_seconds,
            SessionKind::LongBreak => self.long_break_seconds,
        }
    }

    pub(crate) const fn duration_millis(&self, session: SessionKind) -> u64 {
        self.duration_seconds(session) * 1_000
    }
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            focus_seconds: 25 * 60,
            short_break_seconds: 5 * 60,
            long_break_seconds: 15 * 60,
            focuses_before_long_break: 4,
        }
    }
}

fn validate_duration(field: &'static str, seconds: u64) -> Result<(), ConfigError> {
    if seconds == 0 {
        return Err(ConfigError::ZeroDuration { field });
    }
    if seconds > MAX_SESSION_SECONDS {
        return Err(ConfigError::DurationTooLong {
            field,
            maximum_seconds: MAX_SESSION_SECONDS,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    ZeroDuration {
        field: &'static str,
    },
    DurationTooLong {
        field: &'static str,
        maximum_seconds: u64,
    },
    ZeroFocusesBeforeLongBreak,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDuration { field } => write!(formatter, "{field} must be greater than zero"),
            Self::DurationTooLong {
                field,
                maximum_seconds,
            } => write!(
                formatter,
                "{field} must be at most {maximum_seconds} seconds"
            ),
            Self::ZeroFocusesBeforeLongBreak => {
                formatter.write_str("focuses_before_long_break must be greater than zero")
            }
        }
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_the_agreed_pomodoro_durations() {
        let config = TimerConfig::default();

        assert_eq!(config.focus_seconds(), 25 * 60);
        assert_eq!(config.short_break_seconds(), 5 * 60);
        assert_eq!(config.long_break_seconds(), 15 * 60);
        assert_eq!(config.focuses_before_long_break(), 4);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn rejects_zero_duration() {
        assert_eq!(
            TimerConfig::new(0, 300, 900, 4),
            Err(ConfigError::ZeroDuration {
                field: "focus_seconds"
            })
        );
    }

    #[test]
    fn rejects_zero_focuses_before_long_break() {
        assert_eq!(
            TimerConfig::new(1_500, 300, 900, 0),
            Err(ConfigError::ZeroFocusesBeforeLongBreak)
        );
    }
}
