//! Snapshot-derived rendering. Drawing never samples a clock or changes state.

use pomodoro_core::{
    CurrentTask, InterruptionKind, PomodoroState, ProgressState, SessionKind, TimerState,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::{
    app::{App, InputContext},
    controller::{Clock, CompletionNotifier, SaveStore},
    startup_gate::StartupGate,
    ui_history::{draw_history, format_work_time},
    ui_settings::{centered, draw_settings},
    ui_task::draw_task,
};

/// Returns whether the recovery timestamp, loss warning and consent keys fit.
/// A terminal too small to show these facts must not allow recovery consent.
pub fn draw_startup(frame: &mut Frame<'_>, gate: &StartupGate) -> bool {
    let recovery_prompt_visible = !matches!(gate, StartupGate::Recovery(_))
        || (frame.area().width >= 50 && frame.area().height >= 9);
    if !recovery_prompt_visible {
        frame.render_widget(
            Paragraph::new("Enlarge the terminal\nEsc: Exit")
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Startup & recovery "),
                ),
            centered(frame.area(), 82, 9),
        );
        return false;
    }
    let lines = gate.prompt_lines().join("\n");
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Startup & recovery "),
        ),
        centered(frame.area(), 82, 14),
    );
    recovery_prompt_visible
}

pub fn draw<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
) {
    if app.input_context() == InputContext::History {
        draw_history(frame, app);
        return;
    }
    let area = centered(frame.area(), 82, 25);
    let compact_sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(0),
        Constraint::Length(0),
        Constraint::Min(0),
    ])
    .split(area);
    if draw_recovery_size_warning(frame, app, area, compact_sections[4]) {
        return;
    }
    let full_sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(6),
    ])
    .split(area);
    let full_footer_rows = full_sections[4].height.saturating_sub(2);
    let full_footer_width = full_sections[4].width.saturating_sub(2);
    let needed_footer_rows = Paragraph::new(footer_lines(app, false))
        .wrap(Wrap { trim: false })
        .line_count(full_footer_width);
    let compact = area.height < 20
        || full_footer_width == 0
        || needed_footer_rows > usize::from(full_footer_rows);
    let sections = if compact {
        compact_sections
    } else {
        full_sections
    };
    let snapshot = app.state().snapshot();
    let (kind, status, remaining_ms, total_ms, task) = timer_view(snapshot);
    let title = if app.pending_state().is_some() {
        format!(
            "Save pending | Last saved: {} · {status}",
            session_label(kind)
        )
    } else {
        format!("{} · {status}", session_label(kind))
    };
    frame.render_widget(panel(title, " Pomodoro "), sections[0]);
    let seconds = remaining_ms.div_ceil(1_000);
    frame.render_widget(
        panel(
            format!(
                "{:02}:{:02}   Task: {}",
                seconds / 60,
                seconds % 60,
                task.map_or("Not set", CurrentTask::as_str)
            ),
            " Timer ",
        ),
        sections[1],
    );
    if !compact {
        let percent = total_ms.saturating_sub(remaining_ms).saturating_mul(100) / total_ms;
        frame.render_widget(
            Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::LightCyan))
                .percent(u16::try_from(percent).unwrap_or(100)),
            sections[2],
        );
        let history = match app.reflection() {
            Ok(summary) => format!(
                "Total: Focus completed {} / Work {}\nDistractions {} / Returns {}   Round {}/{}",
                summary.completed_focus_sessions,
                format_work_time(summary.work_ms),
                summary.distractions,
                summary.returns,
                snapshot.round_progress.completed_focuses_in_round,
                snapshot.settings.focuses_before_long_break(),
            ),
            Err(error) => format!("Could not summarize history: {error}"),
        };
        frame.render_widget(panel(history, " History "), sections[3]);
    }
    let footer = footer_lines(app, compact);
    frame.render_widget(
        Paragraph::new(footer)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" Controls ")),
        sections[4],
    );
    if let Some(settings) = app.settings() {
        draw_settings(frame, settings);
    }
    if let Some(task) = app.task_edit() {
        draw_task(frame, task, app.message());
    }
}

fn draw_recovery_size_warning<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
    area: Rect,
    footer_area: Rect,
) -> bool {
    let recovery_hint_count = match app.input_context() {
        InputContext::SaveBlocked => usize::from(app.pending_state().is_some()) + 1,
        InputContext::ConfirmUnsavedExit => 2,
        _ => return false,
    };
    let footer_width = footer_area.width.saturating_sub(2);
    let available_rows = footer_area.height.saturating_sub(2);
    let required_rows = Paragraph::new(
        footer_lines(app, true)
            .into_iter()
            .take(recovery_hint_count)
            .collect::<Vec<_>>(),
    )
    .wrap(Wrap { trim: false })
    .line_count(footer_width);
    if footer_width > 0 && required_rows <= usize::from(available_rows) {
        return false;
    }
    let prompt = if app.input_context() == InputContext::ConfirmUnsavedExit {
        "Enlarge terminal to confirm unsaved exit."
    } else {
        "Enlarge terminal to show save recovery controls."
    };
    frame.render_widget(Paragraph::new(prompt).wrap(Wrap { trim: false }), area);
    true
}

fn footer_lines<S: SaveStore, C: Clock, N: CompletionNotifier>(
    app: &App<S, C, N>,
    compact: bool,
) -> Vec<Line<'_>> {
    let mut footer = vec![];
    match app.input_context() {
        InputContext::Closed
        | InputContext::Settings
        | InputContext::Task
        | InputContext::History => {}
        InputContext::ConfirmUnsavedExit => {
            footer.push(Line::from(
                "Some changes are not saved. Exit without saving?",
            ));
            footer.push(Line::from("y: Exit unsaved   n / Esc: Back"));
        }
        InputContext::SaveBlocked => {
            if let Some(pending) = app.pending_state() {
                let (kind, status, ..) = timer_view(pending.snapshot());
                footer.push(Line::from(format!(
                    "Unconfirmed save: {} · {status}",
                    session_label(kind)
                )));
            }
            if !compact {
                footer.push(Line::from("Timer and actions are paused."));
            }
            footer.push(Line::from("r: Retry save   Q: Confirm unsaved exit"));
        }
        InputContext::Normal => {
            let [session, common] = app.normal_hint_lines();
            footer.push(Line::from(session));
            footer.push(Line::from(common));
            if matches!(
                app.state().snapshot().state,
                ProgressState::AwaitingQuickStartDecision { .. }
            ) {
                footer.push(Line::from(if compact {
                    "Choice time excluded."
                } else {
                    "Choice time is not counted. Continue starts a full Focus."
                }));
            }
            if app.show_help() {
                footer.push(Line::from(match app.state().snapshot().state {
                    ProgressState::Ready { .. } => {
                        "Settings are available while ready. Paused time is not counted."
                    }
                    ProgressState::Active { .. } => {
                        "Reset keeps task and type; Skip advances; Cancel goes to Focus."
                    }
                    ProgressState::AwaitingQuickStartDecision { .. } => {
                        if compact {
                            "Settings unavailable during f/c choice."
                        } else {
                            "f: Finish; c: Continue to Focus. Settings unavailable."
                        }
                    }
                }));
            }
        }
    }
    if !app.message().is_empty() {
        footer.push(Line::from(app.message()));
    }
    footer
}

fn panel(content: String, title: &str) -> Paragraph<'_> {
    Paragraph::new(content)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title))
}

fn timer_view(
    snapshot: &PomodoroState,
) -> (SessionKind, &'static str, u64, u64, Option<&CurrentTask>) {
    match &snapshot.state {
        ProgressState::Ready {
            next_kind,
            current_task_draft,
        } => {
            let total = snapshot.settings.duration_seconds(*next_kind) * 1_000;
            (
                *next_kind,
                "Ready",
                total,
                total,
                current_task_draft.as_ref(),
            )
        }
        ProgressState::Active { session, timer } => {
            let status = match timer {
                TimerState::Running { .. } => "Running",
                TimerState::Interrupted { interruption } => match interruption.kind {
                    InterruptionKind::Pause => "Paused",
                    InterruptionKind::Distraction => "Distracted · Awaiting Return",
                    InterruptionKind::AppExit => "Stopped on exit · Awaiting Resume",
                    InterruptionKind::ObservationGap => "Timing gap · Awaiting Resume",
                },
            };
            (
                session.kind,
                status,
                session.remaining_ms(),
                session.planned_duration_ms,
                session.current_task.as_ref(),
            )
        }
        ProgressState::AwaitingQuickStartDecision { current_task, .. } => (
            SessionKind::QuickStart,
            "Choose finish or continue",
            0,
            snapshot.settings.duration_seconds(SessionKind::QuickStart) * 1_000,
            current_task.as_ref(),
        ),
    }
}

const fn session_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Focus => "Focus",
        SessionKind::QuickStart => "Quick Start",
        SessionKind::ShortBreak => "Short Break",
        SessionKind::LongBreak => "Long Break",
    }
}
