use std::{error::Error, fmt, io, process::Command};

use pomodoro_core::SessionKind;

/// Desktop notifications delivered through Linux `notify-send`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NotifySendNotifier;

impl NotifySendNotifier {
    /// Sends a desktop notification for a completed session.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError`] if `notify-send` cannot be launched or exits
    /// unsuccessfully.
    pub fn session_completed(self, completed: SessionKind) -> Result<(), NotificationError> {
        let status = Command::new("notify-send")
            .args(notification_args(completed))
            .status()
            .map_err(NotificationError::Launch)?;
        if status.success() {
            Ok(())
        } else {
            Err(NotificationError::UnsuccessfulExit(status.code()))
        }
    }
}

fn notification_args(completed: SessionKind) -> [&'static str; 4] {
    let (summary, body) = match completed {
        SessionKind::Focus => ("Focus complete", "Take a break."),
        SessionKind::QuickStart => ("Quick Start complete", "Finish or continue to Focus."),
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            ("Break complete", "Ready for the next Focus.")
        }
    };
    ["--app-name", "Pomodoro", summary, body]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_notification_arguments_cover_every_session_kind() {
        for (kind, summary, body) in [
            (SessionKind::Focus, "Focus complete", "Take a break."),
            (
                SessionKind::QuickStart,
                "Quick Start complete",
                "Finish or continue to Focus.",
            ),
            (
                SessionKind::ShortBreak,
                "Break complete",
                "Ready for the next Focus.",
            ),
            (
                SessionKind::LongBreak,
                "Break complete",
                "Ready for the next Focus.",
            ),
        ] {
            assert_eq!(
                notification_args(kind),
                ["--app-name", "Pomodoro", summary, body],
                "{kind:?}"
            );
        }
    }
}
