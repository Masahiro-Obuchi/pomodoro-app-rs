//! Snapshot-derived rendering. Drawing never samples a clock or changes state.

use pomodoro_core::{
    CurrentTask, InterruptionKind, PomodoroState, ProgressState, SessionKind, TimerState,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::{
    app::App,
    controller::{Clock, CompletionNotifier, SaveStore},
    ui_settings::{centered, draw_settings},
};

pub fn draw<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
) {
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(6),
    ])
    .split(centered(frame.area(), 82, 25));
    let snapshot = app.state().snapshot();
    let (kind, status, remaining_ms, total_ms, task) = timer_view(snapshot);
    let title = if app.pending_state().is_some() {
        format!(
            "保存待ち・計時保留 | 最後に保存した状態: {} · {status}",
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
                "{:02}:{:02}   作業: {}",
                seconds / 60,
                seconds % 60,
                task.map_or("未設定", CurrentTask::as_str)
            ),
            " タイマー ",
        ),
        sections[1],
    );
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
            "累計: 集中完了 {}回 / 作業 {}分\n脱線 {}回 / 復帰 {}回   ラウンド: {}/{}",
            summary.completed_focus_sessions,
            summary.work_ms / 60_000,
            summary.distractions,
            summary.returns,
            snapshot.round_progress.completed_focuses_in_round,
            snapshot.settings.focuses_before_long_break(),
        ),
        Err(error) => format!("履歴を集計できませんでした: {error}"),
    };
    frame.render_widget(panel(history, " 記録 "), sections[3]);
    let footer = footer_lines(app);
    frame.render_widget(
        Paragraph::new(footer)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" 操作 ")),
        sections[4],
    );
    if let Some(settings) = app.settings() {
        draw_settings(frame, settings);
    }
}

fn footer_lines<S: SaveStore, C: Clock, N: CompletionNotifier>(
    app: &App<S, C, N>,
) -> Vec<Line<'_>> {
    let mut footer = vec![];
    if app.confirming_unsaved_exit() {
        footer.push(Line::from(
            "保存を確認できていない変更があります。未保存のまま終了しますか？",
        ));
        footer.push(Line::from("y: 未保存で終了   n / Esc: 戻る"));
    } else if app.pending_state().is_some() || app.shutdown_failed() {
        if let Some(pending) = app.pending_state() {
            let (kind, status, ..) = timer_view(pending.snapshot());
            footer.push(Line::from(format!(
                "未確定の保存候補: {} · {status}",
                session_label(kind)
            )));
        }
        footer.push(Line::from("計時と通常操作を保留しています。"));
        footer.push(Line::from("r: 保存を再試行   Q: 未保存終了の確認"));
    } else {
        footer.push(Line::from(match &app.state().snapshot().state {
            ProgressState::AwaitingQuickStartDecision { .. } => {
                "f: Quick Startを終了   c: Focusへ継続   q: 保存して終了"
            }
            ProgressState::Active {
                timer: TimerState::Interrupted { interruption },
                ..
            } if interruption.kind == InterruptionKind::Distraction => {
                "Space: 作業に戻る（Return）   r: リセット   n: スキップ"
            }
            ProgressState::Active {
                timer: TimerState::Interrupted { .. },
                ..
            } => "Space: 再開   r: リセット   n: スキップ",
            _ => "Space: 開始/一時停止   r: リセット   n: スキップ",
        }));
        footer.push(Line::from("s: 設定   ?: ヘルプ   q: 保存して終了"));
        if app.show_help() {
            footer.push(Line::from(
                "設定は待機中のみ変更できます。中断中の時間は加算しません。",
            ));
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
                "待機中",
                total,
                total,
                current_task_draft.as_ref(),
            )
        }
        ProgressState::Active { session, timer } => {
            let status = match timer {
                TimerState::Running { .. } => "実行中",
                TimerState::Interrupted { interruption } => match interruption.kind {
                    InterruptionKind::Pause => "一時停止中",
                    InterruptionKind::Distraction => "脱線中・Return待ち",
                    InterruptionKind::AppExit => "終了から復元・再開待ち",
                    InterruptionKind::ObservationGap => "観測空白・再開待ち",
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
            "終了/継続の選択待ち",
            0,
            snapshot.settings.duration_seconds(SessionKind::QuickStart) * 1_000,
            current_task.as_ref(),
        ),
    }
}

const fn session_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Focus => "集中タイム",
        SessionKind::QuickStart => "Quick Start",
        SessionKind::ShortBreak => "短い休憩",
        SessionKind::LongBreak => "長い休憩",
    }
}
