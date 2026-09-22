use pomodoro_core::TimerConfig;

const DURATION_STEP_SECONDS: u64 = 60;
const MIN_DURATION_SECONDS: u64 = 60;
const MAX_DURATION_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    FocusDuration,
    ShortBreakDuration,
    LongBreakDuration,
    FocusesBeforeLongBreak,
}

impl SettingsField {
    const fn next(self) -> Self {
        match self {
            Self::FocusDuration => Self::ShortBreakDuration,
            Self::ShortBreakDuration => Self::LongBreakDuration,
            Self::LongBreakDuration => Self::FocusesBeforeLongBreak,
            Self::FocusesBeforeLongBreak => Self::FocusDuration,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::FocusDuration => Self::FocusesBeforeLongBreak,
            Self::ShortBreakDuration => Self::FocusDuration,
            Self::LongBreakDuration => Self::ShortBreakDuration,
            Self::FocusesBeforeLongBreak => Self::LongBreakDuration,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    focus_seconds: u64,
    short_break_seconds: u64,
    long_break_seconds: u64,
    focuses_before_long_break: u32,
    selected: SettingsField,
}

impl SettingsDraft {
    pub(crate) fn from_config(config: &TimerConfig) -> Self {
        Self {
            focus_seconds: config.focus_seconds(),
            short_break_seconds: config.short_break_seconds(),
            long_break_seconds: config.long_break_seconds(),
            focuses_before_long_break: config.focuses_before_long_break(),
            selected: SettingsField::FocusDuration,
        }
    }

    #[must_use]
    pub const fn selected(&self) -> SettingsField {
        self.selected
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

    pub(crate) fn select_next(&mut self) {
        self.selected = self.selected.next();
    }

    pub(crate) fn select_previous(&mut self) {
        self.selected = self.selected.previous();
    }

    pub(crate) fn adjust(&mut self, increase: bool) {
        match self.selected {
            SettingsField::FocusDuration => {
                adjust_duration(&mut self.focus_seconds, increase);
            }
            SettingsField::ShortBreakDuration => {
                adjust_duration(&mut self.short_break_seconds, increase);
            }
            SettingsField::LongBreakDuration => {
                adjust_duration(&mut self.long_break_seconds, increase);
            }
            SettingsField::FocusesBeforeLongBreak => {
                self.focuses_before_long_break = if increase {
                    self.focuses_before_long_break.saturating_add(1)
                } else {
                    self.focuses_before_long_break.saturating_sub(1).max(1)
                };
            }
        }
    }

    pub(crate) fn build_config(&self) -> Result<TimerConfig, pomodoro_core::ConfigError> {
        TimerConfig::new(
            self.focus_seconds,
            self.short_break_seconds,
            self.long_break_seconds,
            self.focuses_before_long_break,
        )
    }
}

fn adjust_duration(seconds: &mut u64, increase: bool) {
    *seconds = if increase {
        seconds
            .saturating_add(DURATION_STEP_SECONDS)
            .min(MAX_DURATION_SECONDS)
    } else {
        seconds
            .saturating_sub(DURATION_STEP_SECONDS)
            .max(MIN_DURATION_SECONDS.min(*seconds))
    };
}
