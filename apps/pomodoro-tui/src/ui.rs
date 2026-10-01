//! Snapshot-derived rendering. Drawing never samples a clock or changes state.

use pomodoro_core::{
    CurrentTask, InterruptionKind, PomodoroState, ProgressState, SessionKind, TimerState,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::{
    app::{App, InputContext},
    controller::{Clock, CompletionNotifier, SaveStore},
    startup_gate::StartupGate,
    ui_footer::{Footer, controls, count_rows},
    ui_history::{draw_history, format_work_time},
    ui_settings::{centered, draw_settings},
    ui_task::draw_task,
    ui_text, ui_theme,
    ui_timer::{BigClock, CLOCK_HEIGHT, clock_width, format_time},
};

/// Returns whether the recovery timestamp, loss warning and consent keys fit.
/// A terminal too small to show these facts must not allow recovery consent.
pub fn draw_startup(frame: &mut Frame<'_>, gate: &StartupGate) -> bool {
    frame.render_widget(Block::default().style(ui_theme::base()), frame.area());
    let visible = !matches!(gate, StartupGate::Recovery(_))
        || (frame.area().width >= 50 && frame.area().height >= 9);
    let area = centered(frame.area(), 82, if visible { 14 } else { 9 });
    let color = if matches!(
        gate,
        StartupGate::SaveFailed { .. } | StartupGate::ConfirmUnsaved { .. }
    ) {
        ui_theme::ERROR
    } else {
        ui_theme::WAITING
    };
    let lines = if visible {
        let mut lines = gate
            .prompt_lines()
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        match gate {
            StartupGate::Recovery(_) => {
                lines[3] = controls(&[("y", "Recover"), ("n/q/Esc", "Exit unchanged")], color);
            }
            StartupGate::SaveFailed { .. } => {
                lines[1] = controls(&[("r", "Retry save"), ("Q", "Confirm unsaved exit")], color);
            }
            StartupGate::ConfirmUnsaved { .. } => {
                lines[1] = controls(&[("y", "Exit unsaved"), ("n / Esc", "Back")], color);
            }
            _ => {}
        }
        lines
    } else {
        vec![
            Line::from("Enlarge the terminal"),
            controls(&[("Esc", "Exit")], color),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::new().fg(color))
            .wrap(Wrap { trim: false })
            .block(ui_theme::panel(" Startup & recovery ").title_style(Style::new().fg(color))),
        area,
    );
    visible
}

pub fn draw<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
) {
    frame.render_widget(Block::default().style(ui_theme::base()), frame.area());
    if app.input_context() == InputContext::History {
        draw_history(frame, app);
        return;
    }
    let color = if matches!(
        app.input_context(),
        InputContext::SaveBlocked | InputContext::ConfirmUnsavedExit
    ) {
        ui_theme::ERROR
    } else {
        ui_theme::accent(app.state().snapshot())
    };
    let area = centered(frame.area(), 82, 30);
    let block = ui_theme::panel(" >_ POMODORO ").title_style(Style::new().fg(color));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    draw_main(frame, app, inner, color);
    if let Some(settings) = app.settings() {
        draw_settings(frame, settings, app.message());
    }
    if let Some(task) = app.task_edit() {
        draw_task(frame, task, app.message());
    }
}

struct MainView<'a> {
    status: String,
    time: String,
    total: String,
    task: &'a str,
    round: String,
    percent: u16,
}

impl<'a> MainView<'a> {
    fn new<S: SaveStore, C: Clock, N: CompletionNotifier>(app: &'a App<S, C, N>) -> Self {
        let snapshot = app.state().snapshot();
        let (kind, status, remaining, total, task) = timer_view(snapshot);
        let status = if app.pending_state().is_some() || app.shutdown_failed() {
            format!(
                "Save pending | Last saved: {} · {status}",
                session_label(kind)
            )
        } else {
            format!("{} · {status}", session_label(kind))
        };
        Self {
            status,
            time: format_time(remaining),
            total: format_time(total),
            task: if kind.is_work() {
                task.map_or("Not set", CurrentTask::as_str)
            } else {
                "—"
            },
            round: format!(
                "Round {}/{}",
                snapshot.round_progress.completed_focuses_in_round,
                snapshot.settings.focuses_before_long_break()
            ),
            percent: u16::try_from(total.saturating_sub(remaining).saturating_mul(100) / total)
                .unwrap_or(100),
        }
    }
}

fn draw_main<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
    area: Rect,
    color: Color,
) {
    let view = MainView::new(app);
    let status_rows = count_rows(vec![Line::from(view.status.as_str())], area.width);
    let mut footer = Footer::new(app, false, color);
    let big_rows = status_rows
        .saturating_add(CLOCK_HEIGHT + 6)
        .saturating_add(footer.rows(area.width));
    let large = area.width >= clock_width(&view.time) && big_rows <= area.height;
    if !large {
        footer = Footer::new(app, true, color);
    }
    let minimal_rows = status_rows
        .saturating_add(3)
        .saturating_add(footer.critical_rows(area.width));
    if !large && (minimal_rows > area.height || usize::from(area.width) < view.time.len()) {
        let prompt = match app.input_context() {
            InputContext::ConfirmUnsavedExit => "Enlarge terminal to confirm unsaved exit.",
            InputContext::SaveBlocked => "Enlarge terminal to show save recovery controls.",
            _ => "Enlarge terminal to show timer and controls.",
        };
        frame.render_widget(
            Paragraph::new(prompt)
                .style(Style::new().fg(ui_theme::WAITING))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let mut rest = area;
    frame.render_widget(
        Paragraph::new(view.status.as_str())
            .style(Style::new().fg(color))
            .wrap(Wrap { trim: false }),
        take(&mut rest, status_rows),
    );
    if large {
        draw_large_body(frame, app, &view, &mut rest, color, footer.rows(area.width));
    } else {
        draw_compact_body(frame, &view, &mut rest, color);
    }
    frame.render_widget(
        Paragraph::new(footer.lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::new().fg(ui_theme::RULE)),
            ),
        rest,
    );
}

fn draw_large_body<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
    view: &MainView<'_>,
    rest: &mut Rect,
    color: Color,
    footer_rows: u16,
) {
    let stats = summary_lines(app, rest.width);
    let stats_rows = count_rows(stats.clone(), rest.width);
    let available = rest.height.saturating_sub(CLOCK_HEIGHT + 6 + footer_rows);
    let show_stats = stats_rows <= available;
    let spare = available.saturating_sub(if show_stats { stats_rows } else { 0 });
    frame.render_widget(
        Paragraph::new(view.round.as_str())
            .alignment(Alignment::Right)
            .style(Style::new().fg(ui_theme::MUTED)),
        take(rest, 1),
    );
    if spare > 0 {
        take(rest, 1);
    }
    frame.render_widget(
        BigClock {
            text: &view.time,
            style: Style::new().fg(color).bg(ui_theme::BACKGROUND),
        },
        take(rest, CLOCK_HEIGHT),
    );
    frame.render_widget(
        Paragraph::new(format!("{} remaining / {}", view.time, view.total))
            .alignment(Alignment::Center)
            .style(Style::new().fg(ui_theme::MUTED)),
        take(rest, 1),
    );
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(color).bg(ui_theme::TRACK))
            .label("")
            .percent(view.percent),
        take(rest, 1),
    );
    if spare > 1 {
        take(rest, 1);
    }
    let task = vec![
        Line::styled("CURRENT TASK", Style::new().fg(ui_theme::MUTED)),
        Line::from(ui_text::shorten(view.task, usize::from(rest.width))),
    ];
    frame.render_widget(Paragraph::new(task), take(rest, 2));
    if show_stats {
        if spare > 2 {
            take(rest, 1);
        }
        frame.render_widget(
            Paragraph::new(stats).wrap(Wrap { trim: false }),
            take(rest, stats_rows),
        );
    }
}

fn draw_compact_body(frame: &mut Frame<'_>, view: &MainView<'_>, rest: &mut Rect, color: Color) {
    let clock = if view.time.len() + view.total.len() + 3 <= usize::from(rest.width) {
        format!("{} / {}", view.time, view.total)
    } else {
        view.time.clone()
    };
    frame.render_widget(
        Paragraph::new(clock).style(Style::new().fg(color)),
        take(rest, 1),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "Task: {}",
            ui_text::shorten(view.task, usize::from(rest.width.saturating_sub(6)))
        )),
        take(rest, 1),
    );
}

fn take(area: &mut Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    let result = Rect::new(area.x, area.y, area.width, height);
    area.y += height;
    area.height -= height;
    result
}

fn summary_lines<S: SaveStore, C: Clock, N: CompletionNotifier>(
    app: &App<S, C, N>,
    width: u16,
) -> Vec<Line<'static>> {
    let summary = match app.reflection() {
        Ok(summary) => summary,
        Err(error) => {
            return vec![Line::styled(
                format!("Could not summarize history: {error}"),
                Style::new().fg(ui_theme::ERROR),
            )];
        }
    };
    let metrics = [
        ("Work total", format_work_time(summary.work_ms)),
        (
            "Focus completed",
            summary.completed_focus_sessions.to_string(),
        ),
        ("Distractions", summary.distractions.to_string()),
        ("Returns", summary.returns.to_string()),
    ];
    let minimum = metrics
        .iter()
        .map(|(label, value)| label.len().max(value.len()) + 2)
        .max()
        .unwrap_or(1);
    let columns = if usize::from(width) / 4 >= minimum {
        4
    } else if usize::from(width) / 2 >= minimum {
        2
    } else {
        1
    };
    let cell_width = usize::from(width) / columns;
    let mut lines = vec![Line::styled(
        "─ History / Total ─",
        Style::new().fg(ui_theme::MUTED),
    )];
    for group in metrics.chunks(columns) {
        let mut labels = Vec::new();
        let mut values = Vec::new();
        for (label, value) in group {
            labels.push(Span::styled(
                format!("{label:<cell_width$}"),
                Style::new().fg(ui_theme::MUTED),
            ));
            values.push(Span::raw(format!("{value:<cell_width$}")));
        }
        lines.push(Line::from(labels));
        lines.push(Line::from(values));
    }
    lines
}

pub(super) fn timer_view(
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

pub(super) const fn session_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Focus => "Focus",
        SessionKind::QuickStart => "Quick Start",
        SessionKind::ShortBreak => "Short Break",
        SessionKind::LongBreak => "Long Break",
    }
}
