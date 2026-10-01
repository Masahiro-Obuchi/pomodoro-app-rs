//! Contextual explanations derived from the displayed snapshot.

use pomodoro_core::{InterruptionKind, PomodoroState, ProgressState, SessionKind, TimerState};
use ratatui::{
    style::{Color, Style},
    text::Line,
};

use crate::{ui::timer_view, ui_theme, ui_timer::format_time};

struct Help {
    title: &'static str,
    details: [&'static str; 3],
    compact: [&'static str; 2],
}

pub(super) fn lines(snapshot: &PomodoroState, compact: bool, color: Color) -> Vec<Line<'static>> {
    let help = match &snapshot.state {
        ProgressState::Ready { next_kind, .. } => ready_help(*next_kind),
        ProgressState::Active { session, timer } => match timer {
            TimerState::Running { .. } => running_help(session.kind),
            TimerState::Interrupted { interruption } => interrupted_help(interruption.kind),
        },
        ProgressState::AwaitingQuickStartDecision { .. } => Help {
            title: "Finish or continue",
            details: [
                "f: Finish; c: Continue to Focus with the same task.",
                "Choice time is not counted. Settings unavailable here.",
                "h shows recorded totals; q saves this choice for later.",
            ],
            compact: ["Settings unavailable.", "Choice time excluded."],
        },
    };
    let mut lines = vec![Line::styled(
        if compact {
            "HELP".into()
        } else {
            format!("HELP · {}", help.title)
        },
        Style::new().fg(color),
    )];
    let details = if compact {
        help.compact.as_slice()
    } else {
        help.details.as_slice()
    };
    lines.extend(
        details
            .iter()
            .map(|text| Line::styled(*text, Style::new().fg(ui_theme::MUTED))),
    );
    if !compact {
        let (_, _, _, duration, _) = timer_view(snapshot);
        lines.push(Line::styled(
            format!("Session duration: {}", format_time(duration)),
            Style::new().fg(ui_theme::MUTED),
        ));
    }
    lines
}

fn ready_help(kind: SessionKind) -> Help {
    let mut help = Help {
        title: "Getting started",
        details: [
            "Space starts Focus. t edits an optional task; Enter saves, Esc cancels.",
            "2 starts two minutes; then f finishes or c starts a full Focus.",
            "Completed Focus/breaks auto-start while open; h shows totals.",
        ],
        compact: ["t: optional task.", "2: two-minute start."],
    };
    match kind {
        SessionKind::Focus => {}
        SessionKind::QuickStart => {
            help.title = "Restart Quick Start";
            help.details[0] = "Space restarts two minutes. t edits the optional task.";
            help.details[1] = "At zero: f finishes; c starts a full Focus with the same task.";
            help.details[2] = "h shows recorded totals; s edits session durations.";
            help.compact[1] = "Space: restart 2 min.";
        }
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            help.title = "Start a break";
            help.details = [
                "Space starts this break. Work total excludes break time.",
                "n goes to Focus; s edits session durations.",
                "Completed breaks auto-start Focus; h shows recorded totals.",
            ];
            help.compact = ["Space: start break.", "n: back to Focus."];
        }
    }
    help
}

fn running_help(kind: SessionKind) -> Help {
    let mut help = Help {
        title: "Focus controls",
        details: [
            "Space pauses; d reports a distraction. Both stop work time.",
            "r resets to the same type and task; n skips; x cancels to Focus start.",
            "The next break starts automatically; h shows recorded totals.",
        ],
        compact: ["d pauses work time.", "Next break auto-starts."],
    };
    match kind {
        SessionKind::Focus => {}
        SessionKind::QuickStart => {
            help.title = "Quick Start controls";
            help.details[2] = "At zero: f finishes; c starts a full Focus with the same task.";
            help.compact[1] = "Finish f / Continue c";
        }
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            help.title = "Break controls";
            help.details = [
                "Space pauses this break. Break time is excluded from Work total.",
                "r resets this break; n skips to Focus start; x cancels to Focus start.",
                "The next Focus starts automatically; h shows recorded totals.",
            ];
            help.compact = ["Break time excluded.", "Next Focus auto-starts"];
        }
    }
    help
}

fn interrupted_help(kind: InterruptionKind) -> Help {
    let mut help = Help {
        title: "Resume",
        details: [
            "Space resumes the remaining time. Paused time is not counted.",
            "r resets to the same type and task; n skips; x cancels to Focus start.",
            "h shows recorded totals. Settings are available at a Ready screen.",
        ],
        compact: ["Space resumes.", "Paused time excluded."],
    };
    match kind {
        InterruptionKind::Pause => {}
        InterruptionKind::Distraction => {
            help.title = "Return to your task";
            help.details[0] = "Space records a Return and resumes the remaining work time.";
            help.details[2] = "Distraction time is not counted; h shows recorded totals.";
            help.compact = ["Space records Return.", "Stopped time excluded."];
        }
        InterruptionKind::AppExit => {
            help.details[0] = "Space resumes the remaining time. Time while closed is not counted.";
            help.compact[1] = "Offline time excluded.";
        }
        InterruptionKind::ObservationGap => {
            help.details[0] = "Space resumes the remaining time. The timing gap is not counted.";
            help.compact[1] = "Gap time excluded.";
        }
    }
    help
}
