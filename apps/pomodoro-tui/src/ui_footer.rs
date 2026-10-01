use pomodoro_core::ProgressState;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    app::{App, ControlHint, InputContext},
    controller::{Clock, CompletionNotifier, SaveStore},
    ui::{session_label, timer_view},
    ui_text, ui_theme,
};

pub(super) struct Footer {
    pub(super) lines: Vec<Line<'static>>,
    critical_lines: usize,
}

impl Footer {
    pub(super) fn new<S: SaveStore, C: Clock, N: CompletionNotifier>(
        app: &App<S, C, N>,
        compact: bool,
        color: Color,
    ) -> Self {
        let mut lines = match app.input_context() {
            InputContext::ConfirmUnsavedExit => vec![
                Line::styled(
                    "Some changes are not saved. Exit without saving?",
                    Style::new().fg(ui_theme::ERROR),
                ),
                controls(
                    &[("y", "Exit unsaved"), ("n / Esc", "Back")],
                    ui_theme::ERROR,
                ),
            ],
            InputContext::SaveBlocked => {
                let mut lines = Vec::new();
                if let Some(pending) = app.pending_state() {
                    let (kind, status, ..) = timer_view(pending.snapshot());
                    lines.push(Line::styled(
                        format!("Unconfirmed save: {} · {status}", session_label(kind)),
                        Style::new().fg(ui_theme::WAITING),
                    ));
                }
                lines.push(controls(
                    &[("r", "Retry save"), ("Q", "Confirm unsaved exit")],
                    ui_theme::ERROR,
                ));
                lines
            }
            InputContext::Normal => app
                .normal_hints()
                .into_iter()
                .map(|hints| normal_controls(&hints, color))
                .collect(),
            _ => Vec::new(),
        };
        let critical_lines = lines.len();
        add_explanation(app, compact, &mut lines);
        if !app.message().is_empty() {
            lines.push(Line::styled(
                app.message().to_owned(),
                Style::new().fg(
                    if matches!(
                        app.input_context(),
                        InputContext::SaveBlocked | InputContext::ConfirmUnsavedExit
                    ) {
                        ui_theme::ERROR
                    } else {
                        ui_theme::TEXT
                    },
                ),
            ));
        }
        Self {
            lines,
            critical_lines,
        }
    }

    pub(super) fn rows(&self, width: u16) -> u16 {
        count_rows(self.lines.clone(), width)
    }

    pub(super) fn critical_rows(&self, width: u16) -> u16 {
        count_rows(self.lines[..self.critical_lines].to_vec(), width)
    }
}

pub(super) fn count_rows(lines: Vec<Line<'_>>, width: u16) -> u16 {
    if width == 0 {
        return u16::MAX;
    }
    u16::try_from(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .line_count(width),
    )
    .unwrap_or(u16::MAX)
}

pub(crate) fn controls(hints: &[(&str, &str)], color: Color) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, description)) in hints.iter().enumerate() {
        if index != 0 {
            spans.push(Span::raw("   "));
        }
        spans.extend(ui_text::hint(key, description, color));
    }
    Line::from(spans)
}

fn normal_controls(hints: &[ControlHint], color: Color) -> Line<'static> {
    controls(
        &hints
            .iter()
            .map(|hint| (hint.key, hint.description))
            .collect::<Vec<_>>(),
        color,
    )
}

fn add_explanation<S: SaveStore, C: Clock, N: CompletionNotifier>(
    app: &App<S, C, N>,
    compact: bool,
    lines: &mut Vec<Line<'static>>,
) {
    if app.input_context() == InputContext::SaveBlocked && !compact {
        lines.push(Line::styled(
            "Timer and actions are paused.",
            Style::new().fg(ui_theme::MUTED),
        ));
    }
    if app.input_context() != InputContext::Normal {
        return;
    }
    if matches!(
        app.state().snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ) {
        lines.push(Line::styled(
            if compact {
                "Choice time excluded."
            } else {
                "Choice time is not counted. Continue starts a full Focus."
            },
            Style::new().fg(ui_theme::MUTED),
        ));
    }
    if app.show_help() {
        lines.push(Line::styled(
            match app.state().snapshot().state {
                ProgressState::Ready { .. } => {
                    "Settings are available while ready. Paused time is not counted."
                }
                ProgressState::Active { .. } => {
                    "Reset keeps task and type; Skip advances; Cancel goes to Focus."
                }
                ProgressState::AwaitingQuickStartDecision { .. } if compact => {
                    "Settings unavailable during f/c choice."
                }
                ProgressState::AwaitingQuickStartDecision { .. } => {
                    "f: Finish; c: Continue to Focus. Settings unavailable."
                }
            },
            Style::new().fg(ui_theme::MUTED),
        ));
    }
}
