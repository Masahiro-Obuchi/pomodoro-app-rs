//! User decisions before the normal controller may receive input.

use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use pomodoro_core::Timestamp;
use pomodoro_platform::{RecoveryCandidate, SaveError, WritableStorage};

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

    /// Handles only startup keys. The recovery timestamp is sampled at consent,
    /// while retries retain the candidate's original timestamp.
    ///
    /// # Errors
    /// Returns a candidate validation error before any recovery file is written.
    pub fn handle_key(self, key: KeyCode, at: Timestamp) -> Result<Self, SaveError> {
        match self {
            Self::Recovery(candidate) => match key {
                KeyCode::Char('y') => {
                    Ok(Self::Saving(StartupSave::confirm_recovery(*candidate, at)?))
                }
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
                        || format!("Unix時刻 {saved_at} ms"),
                        |at| at.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                    );
                vec![
                    "保存本体を読み込めません。バックアップから復旧できます。".into(),
                    format!("復旧元の保存日時: {date}"),
                    "この日時以降の記録が失われる可能性があります。".into(),
                    "y: 復旧して続行   n / q / Esc: 変更せず終了".into(),
                    format!("復旧元: {}", candidate.location().backup_path().display()),
                    format!("原因: {}", candidate.primary_problem()),
                ]
            }
            Self::SaveFailed { error, .. } => vec![
                "起動時の保存を確認できません。通常操作と計時を保留しています。".into(),
                "r: 同じ保存候補を再試行   Q: 未保存終了の確認".into(),
                format!("保存エラー: {error}"),
            ],
            Self::ConfirmUnsaved { error, .. } => vec![
                "起動時の保存を確認できていません。未保存のまま終了しますか？".into(),
                "y: 未保存で終了   n / Esc: 戻る".into(),
                format!("保存エラー: {error}"),
            ],
            Self::Saving(_) => vec!["起動時の状態を保存しています。".into()],
            Self::Ready(_) | Self::Exited(_) => Vec::new(),
        }
    }
}
