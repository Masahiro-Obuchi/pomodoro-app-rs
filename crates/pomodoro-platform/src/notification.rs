#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Command, Stdio};
use std::{error::Error, fmt, io};

use pomodoro_core::SessionKind;
#[cfg(windows)]
use tauri_winrt_notification::Toast;

/// Sends completion notices through the desktop facility of the current OS.
#[derive(Debug, Default, Clone, Copy)]
pub struct DesktopNotifier;

impl DesktopNotifier {
    /// Sends a desktop notification for a completed, already saved session.
    ///
    /// # Errors
    /// Returns an error when the OS notification adapter rejects the request.
    pub fn session_completed(self, completed: SessionKind) -> Result<(), NotificationError> {
        let content = notification_content(completed);
        #[cfg(target_os = "linux")]
        {
            let status = Command::new("notify-send")
                .args(["--app-name", "Pomodoro", content.summary, content.body])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(NotificationError::Launch)?;
            check_status(status)
        }
        #[cfg(target_os = "macos")]
        {
            // Pass copy as argv, not interpolated AppleScript source.
            let status = Command::new("/usr/bin/osascript")
                .args([
                    "-e",
                    "on run argv\n display notification (item 1 of argv) with title (item 2 of argv)\nend run",
                    "--",
                    content.body,
                    content.summary,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(NotificationError::Launch)?;
            check_status(status)
        }
        #[cfg(windows)]
        {
            // Unpackaged console executables have no registered AppUserModelID.
            // The built-in PowerShell identity works without installing a shortcut.
            Toast::new(Toast::POWERSHELL_APP_ID)
                .title(content.summary)
                .text1(content.body)
                .show()
                .map_err(|error| NotificationError::Backend(error.to_string()))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_status(status: std::process::ExitStatus) -> Result<(), NotificationError> {
    if status.success() {
        Ok(())
    } else {
        Err(NotificationError::UnsuccessfulExit(status.code()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NotificationContent {
    summary: &'static str,
    body: &'static str,
}

fn notification_content(completed: SessionKind) -> NotificationContent {
    let (summary, body) = match completed {
        SessionKind::Focus => ("Focus complete", "Take a break."),
        SessionKind::QuickStart => ("Quick Start complete", "Finish or continue to Focus."),
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            ("Break complete", "Ready for the next Focus.")
        }
    };
    NotificationContent { summary, body }
}

#[derive(Debug)]
pub enum NotificationError {
    Launch(io::Error),
    UnsuccessfulExit(Option<i32>),
    Backend(String),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(error) => {
                write!(formatter, "could not launch desktop notification: {error}")
            }
            Self::UnsuccessfulExit(code) => {
                write!(
                    formatter,
                    "desktop notification exited unsuccessfully: {code:?}"
                )
            }
            Self::Backend(error) => write!(formatter, "desktop notification failed: {error}"),
        }
    }
}

impl Error for NotificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Launch(error) => Some(error),
            Self::UnsuccessfulExit(_) | Self::Backend(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_notification_copy_covers_every_session_kind() {
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
                notification_content(kind),
                NotificationContent { summary, body },
                "{kind:?}"
            );
        }
    }
}
