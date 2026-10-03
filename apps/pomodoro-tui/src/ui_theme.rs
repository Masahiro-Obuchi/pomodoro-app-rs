//! Shared Neon Focus colors. State names accompany every state color.
//! Crossterm suppresses emitted color parameters when `NO_COLOR` is nonempty.

use pomodoro_core::{PomodoroState, ProgressState, SessionKind, TimerState};
use ratatui::{
    style::{Color, Style},
    widgets::{Block, Borders},
};

pub(crate) const BACKGROUND: Color = Color::Rgb(12, 17, 16);
pub(crate) const TEXT: Color = Color::Rgb(229, 238, 232);
pub(crate) const MUTED: Color = Color::Rgb(145, 164, 154);
pub(crate) const RULE: Color = Color::Rgb(52, 70, 59);
pub(crate) const FOCUS: Color = Color::Rgb(169, 243, 107);
pub(crate) const BREAK: Color = Color::Rgb(115, 219, 223);
pub(crate) const WAITING: Color = Color::Rgb(237, 194, 118);
pub(crate) const ERROR: Color = Color::Rgb(255, 133, 133);
pub(crate) const TRACK: Color = Color::Rgb(35, 51, 39);

pub(crate) const fn base() -> Style {
    Style::new().fg(TEXT).bg(BACKGROUND)
}

pub(crate) const fn key(color: Color) -> Style {
    Style::new().fg(BACKGROUND).bg(color)
}

pub(crate) fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(RULE))
        .title(title)
        .title_style(Style::new().fg(FOCUS))
        .style(base())
}

pub(crate) const fn accent(snapshot: &PomodoroState) -> Color {
    let kind = match &snapshot.state {
        ProgressState::Ready { next_kind, .. } => *next_kind,
        ProgressState::Active { session, timer } => {
            if matches!(timer, TimerState::Interrupted { .. }) {
                return WAITING;
            }
            session.kind
        }
        ProgressState::AwaitingQuickStartDecision { .. } => return WAITING,
    };
    match kind {
        SessionKind::Focus | SessionKind::QuickStart => FOCUS,
        SessionKind::ShortBreak | SessionKind::LongBreak => BREAK,
    }
}
