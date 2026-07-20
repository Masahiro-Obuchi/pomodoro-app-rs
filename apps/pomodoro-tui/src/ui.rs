use pomodoro_core::{SessionKind, TimerStatus};
use pomodoro_platform::local_date_at;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::app::App;

pub fn draw(frame: &mut Frame<'_>, app: &App, now_ms: u64) {
    let area = centered(frame.area(), 70, 22);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(area);

    draw_header(frame, app, sections[0]);
    draw_timer(frame, app, now_ms, sections[1]);
    draw_progress(frame, app, now_ms, sections[2]);
    draw_history(frame, app, now_ms, sections[3]);
    draw_footer(frame, app, sections[4]);
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let timer = &app.state().timer;
    let title = format!(
        " {} · {} ",
        session_label(timer.session()),
        status_label(timer.status())
    );
    let paragraph = Paragraph::new(title)
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(session_color(timer.session()))
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL).title(" Pomodoro "));
    frame.render_widget(paragraph, area);
}

fn draw_timer(frame: &mut Frame<'_>, app: &App, now_ms: u64, area: Rect) {
    let seconds = app.state().timer.remaining_seconds(now_ms);
    let time = format!("{:02}:{:02}", seconds / 60, seconds % 60);
    let paragraph = Paragraph::new(time)
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
    frame.render_widget(paragraph, area);
}

fn draw_progress(frame: &mut Frame<'_>, app: &App, now_ms: u64, area: Rect) {
    let timer = &app.state().timer;
    let total_ms = timer.config().duration_seconds(timer.session()) * 1_000;
    let remaining_ms = timer.remaining_millis(now_ms).min(total_ms);
    let elapsed_percent = (total_ms - remaining_ms).saturating_mul(100) / total_ms;
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
        .gauge_style(Style::default().fg(session_color(timer.session())))
        .percent(u16::try_from(elapsed_percent).unwrap_or(100));
    frame.render_widget(gauge, area);
}

fn draw_history(frame: &mut Frame<'_>, app: &App, now_ms: u64, area: Rect) {
    let today = local_date_at(now_ms).unwrap_or_else(|_| "---- -- --".to_owned());
    let summary = app
        .state()
        .history
        .summary(&today)
        .copied()
        .unwrap_or_default();
    let timer = &app.state().timer;
    let line = format!(
        "今日: {}回 / {}分    ラウンド: {}/{}",
        summary.completed_focus_sessions,
        summary.focused_seconds / 60,
        timer.completed_focuses_in_round(),
        timer.config().focuses_before_long_break()
    );
    frame.render_widget(
        Paragraph::new(line)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let content = if app.show_help() {
        vec![
            Line::from("Space: 開始/一時停止/再開   r: リセット   n: スキップ"),
            Line::from("?: ヘルプを閉じる   q: 状態を保存して終了"),
        ]
    } else if app.message().is_empty() {
        vec![Line::from(vec![
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": 操作   r: リセット   n: スキップ   ?: ヘルプ   q: 終了"),
        ])]
    } else {
        vec![Line::from(app.message().to_owned())]
    };

    frame.render_widget(
        Paragraph::new(content)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(" 操作 ")),
        area,
    );
}

const fn session_label(session: SessionKind) -> &'static str {
    match session {
        SessionKind::Focus => "集中タイム",
        SessionKind::ShortBreak => "短い休憩",
        SessionKind::LongBreak => "長い休憩",
    }
}

const fn status_label(status: TimerStatus) -> &'static str {
    match status {
        TimerStatus::Idle => "待機中",
        TimerStatus::Running => "実行中",
        TimerStatus::Paused => "一時停止中",
    }
}

const fn session_color(session: SessionKind) -> Color {
    match session {
        SessionKind::Focus => Color::LightRed,
        SessionKind::ShortBreak => Color::LightGreen,
        SessionKind::LongBreak => Color::LightBlue,
    }
}

fn centered(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.min(max_width);
    let height = area.height.min(max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
