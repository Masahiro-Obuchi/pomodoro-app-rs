//! User decisions before the normal controller may receive input.

use std::error::Error;

use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use pomodoro_core::Timestamp;
use pomodoro_platform::{RecoveryCandidate, SaveError, TimeError, WritableStorage};

use crate::controller::{ExitOutcome, Startup, StartupSave};

#[derive(Debug)]
pub enum StartupGate {
    Recovery(Box<RecoveryCandidate>),
    Saving(StartupSave),
    SaveFailed { save: StartupSave, error: SaveError },
    ConfirmUnsaved { save: StartupSave, error: SaveError },
    Ready(Box<WritableStorage>),
    Exited(ExitOutcome),
}

impl From<Startup> for StartupGate {
    fn from(startup: Startup) -> Self {
        match startup {
            Startup::Saving(save) => Self::Saving(save),
            Startup::RecoveryRequired(candidate) => Self::Recovery(candidate),
        }
    }
}

impl StartupGate {
    /// Attempts the already prepared candidate once. A failure keeps its lock and
    /// exact candidate available for an explicit retry or unsaved exit.
    #[must_use]
    pub fn advance(self) -> Self {
        match self {
            Self::Saving(save) => match save.save() {
                Ok(store) => Self::Ready(Box::new(store)),
                Err(failure) => Self::SaveFailed {
                    save: failure.startup,
                    error: failure.error,
                },
            },
            other => other,
        }
    }

    /// Handles only startup keys. The clock is sampled only for visible recovery
    /// consent. Retry, cancellation and unsaved-exit choices remain available
    /// even when the wall clock cannot provide a new timestamp.
    ///
    /// # Errors
    /// Returns a clock or candidate validation error before any recovery file is written.
    pub fn handle_key(
        self,
        key: KeyCode,
        recovery_prompt_visible: bool,
        read_time: impl FnOnce() -> Result<Timestamp, TimeError>,
    ) -> Result<Self, Box<dyn Error>> {
        match self {
            Self::Recovery(candidate) => match key {
                KeyCode::Char('y') if recovery_prompt_visible => Ok(Self::Saving(
                    StartupSave::confirm_recovery(*candidate, read_time()?)?,
                )),
                KeyCode::Char('n' | 'q') | KeyCode::Esc => {
                    Ok(Self::Exited(Startup::RecoveryRequired(candidate).cancel()))
                }
                _ => Ok(Self::Recovery(candidate)),
            },
            Self::SaveFailed { save, error } => match key {
                KeyCode::Char('r') => Ok(Self::Saving(save)),
                KeyCode::Char('Q' | 'q') => Ok(Self::ConfirmUnsaved { save, error }),
                _ => Ok(Self::SaveFailed { save, error }),
            },
            Self::ConfirmUnsaved { save, error } => match key {
                KeyCode::Char('y') => Ok(Self::Exited(save.exit_without_saving())),
                KeyCode::Char('n') | KeyCode::Esc => Ok(Self::SaveFailed { save, error }),
                _ => Ok(Self::ConfirmUnsaved { save, error }),
            },
            other => Ok(other),
        }
    }

    #[must_use]
    pub fn prompt_lines(&self) -> Vec<String> {
        match self {
            Self::Recovery(candidate) => {
                let saved_at = candidate.backup().saved_at().0;
                let date = i64::try_from(saved_at)
                    .ok()
                    .and_then(DateTime::<Utc>::from_timestamp_millis)
                    .map_or_else(
                        || format!("Unix time {saved_at} ms"),
                        |at| at.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                    );
                vec![
                    "Primary save unreadable; backup available.".into(),
                    format!("Backup saved: {date}"),
                    "Changes after this time may be lost.".into(),
                    "y: Recover   n/q/Esc: Exit unchanged".into(),
                    format!("Backup: {}", candidate.location().backup_path().display()),
                    format!("Cause: {}", candidate.primary_problem()),
                ]
            }
            Self::SaveFailed { error, .. } => vec![
                "Startup save unconfirmed. Timer and actions are paused.".into(),
                "r: Retry save   Q: Confirm unsaved exit".into(),
                format!("Save error: {error}"),
            ],
            Self::ConfirmUnsaved { error, .. } => vec![
                "Startup save unconfirmed. Exit without saving?".into(),
                "y: Exit unsaved   n / Esc: Back".into(),
                format!("Save error: {error}"),
            ],
            Self::Saving(_) => vec!["Saving startup state.".into()],
            Self::Ready(_) | Self::Exited(_) => Vec::new(),
        }
    }
}
