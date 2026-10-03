//! Snapshot-derived rendering. Drawing never samples a clock or changes state.

use pomodoro_core::{
    CurrentTask, InterruptionKind, PomodoroState, ProgressState, SessionKind, TimerState,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
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
    let block = ui_theme::panel(" >_ POMODORO ")
        .title_style(Style::new().fg(color))
        .padding(Padding::horizontal(u16::from(area.width >= 60)));
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
    let header = header_lines(&view, area.width, color);
    let mut footer = Footer::new(app, false, color);
    let big_rows = count_rows(header.clone(), area.width)
        .saturating_add(CLOCK_HEIGHT + 7)
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
    let header = if large {
        header
    } else {
        vec![Line::styled(view.status.as_str(), Style::new().fg(color))]
    };
    let header_rows = count_rows(header.clone(), area.width);
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: false }),
        take(&mut rest, header_rows),
    );
    let footer_padding = if large {
        draw_large_body(frame, app, &view, &mut rest, color, footer.rows(area.width))
    } else {
        draw_compact_body(frame, &view, &mut rest, color);
        0
    };
    frame.render_widget(
        Paragraph::new(footer.lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::new().fg(ui_theme::RULE))
                    .padding(Padding::new(0, 0, footer_padding, 0)),
            ),
        rest,
    );
}

fn header_lines<'a>(view: &'a MainView<'_>, width: u16, color: Color) -> Vec<Line<'a>> {
    let status = Span::styled(view.status.as_str(), Style::new().fg(color));
    let round = Span::styled(view.round.as_str(), Style::new().fg(ui_theme::MUTED));
    let used = status.width() + round.width();
    if used + 2 <= usize::from(width) {
        vec![Line::from(vec![
            status,
            Span::raw(" ".repeat(usize::from(width) - used)),
            round,
        ])]
    } else {
        vec![
            Line::from(status),
            Line::from(round).alignment(Alignment::Right),
        ]
    }
}

fn draw_large_body<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
    view: &MainView<'_>,
    rest: &mut Rect,
    color: Color,
    footer_rows: u16,
) -> u16 {
    let stats = summary_lines(app, rest.width);
    let stats_rows = count_rows(stats.clone(), rest.width);
    // Reserve the clock gaps, footer rule and every footer row first.
    // Extra rows separate sections; saved totals take priority during recovery.
    let available = rest.height.saturating_sub(CLOCK_HEIGHT + 7 + footer_rows);
    let show_stats = stats_rows.saturating_add(1) <= available;
    let spare = available.saturating_sub(if show_stats { stats_rows + 1 } else { 0 });
    take(rest, 2);
    frame.render_widget(
        BigClock {
            text: &view.time,
            style: Style::new().fg(color).bg(ui_theme::BACKGROUND),
        },
        take(rest, CLOCK_HEIGHT),
    );
    take(rest, 1 + u16::from(spare > 3));
    draw_progress(frame, take(rest, 1), view.percent, color);
    if spare > 0 {
        take(rest, 1);
    }
    let task = vec![
        Line::styled("CURRENT TASK", Style::new().fg(ui_theme::MUTED)),
        Line::from(ui_text::shorten(view.task, usize::from(rest.width))),
    ];
    frame.render_widget(Paragraph::new(task), take(rest, 2));
    if spare > 1 {
        take(rest, 1);
    }
    if show_stats {
        draw_rule(frame, take(rest, 1));
        if spare > 4 {
            take(rest, 1);
        }
        frame.render_widget(
            Paragraph::new(stats).wrap(Wrap { trim: false }),
            take(rest, stats_rows),
        );
    }
    if spare > 2 {
        take(rest, 1);
    }
    u16::from(spare > 5)
}

fn draw_rule(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(ui_theme::RULE)),
        area,
    );
}

fn draw_progress(frame: &mut Frame<'_>, area: Rect, percent: u16, color: Color) {
    let width = usize::from(area.width);
    let filled = width * usize::from(percent.min(100)) / 100;
    let line = Line::from(vec![
        Span::styled("▂".repeat(filled), Style::new().fg(color)),
        Span::styled("▂".repeat(width - filled), Style::new().fg(ui_theme::TRACK)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_compact_body(frame: &mut Frame<'_>, view: &MainView<'_>, rest: &mut Rect, color: Color) {
    let caption = format!("REMAINING / {}", view.time);
    let clock = if caption.len() <= usize::from(rest.width) {
        caption
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
    let mut lines = Vec::new();
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
