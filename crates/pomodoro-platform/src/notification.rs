use std::{error::Error, fmt, io, process::Command};

use pomodoro_core::SessionKind;

/// Linuxの`notify-send`を利用するデスクトップ通知。
#[derive(Debug, Default, Clone, Copy)]
pub struct NotifySendNotifier;

impl NotifySendNotifier {
    /// セッション完了をデスクトップへ通知する。
    ///
    /// # Errors
    ///
    /// `notify-send`を起動できない、またはコマンドが失敗した場合に
    /// [`NotificationError`]を返す。
    pub fn session_completed(self, completed: SessionKind) -> Result<(), NotificationError> {
        let (summary, body) = match completed {
            SessionKind::Focus => ("集中タイム完了", "休憩しましょう。"),
            SessionKind::ShortBreak | SessionKind::LongBreak => {
                ("休憩完了", "次の集中タイムを始められます。")
            }
        };

        let status = Command::new("notify-send")
            .args(["--app-name", "Pomodoro", summary, body])
            .status()
            .map_err(NotificationError::Launch)?;
        if status.success() {
            Ok(())
        } else {
            Err(NotificationError::UnsuccessfulExit(status.code()))
        }
    }
}

#[derive(Debug)]
pub enum NotificationError {
    Launch(io::Error),
    UnsuccessfulExit(Option<i32>),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(error) => write!(formatter, "could not launch notify-send: {error}"),
            Self::UnsuccessfulExit(code) => {
                write!(formatter, "notify-send exited unsuccessfully: {code:?}")
            }
        }
    }
}

impl Error for NotificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Launch(error) => Some(error),
            Self::UnsuccessfulExit(_) => None,
        }
    }
}
