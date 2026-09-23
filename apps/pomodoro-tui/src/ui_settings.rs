use crate::settings::{SettingsDraft, SettingsField};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(crate) fn draw_settings(frame: &mut Frame<'_>, settings: &SettingsDraft) {
    let area = centered(frame.area(), 58, 14);
    let fields = [
        SettingsField::FocusDuration,
        SettingsField::ShortBreakDuration,
        SettingsField::LongBreakDuration,
        SettingsField::FocusesBeforeLongBreak,
    ];
    let mut lines = Vec::with_capacity(7);

    for field in fields {
        let selected = field == settings.selected();
        let marker = if selected { "▶" } else { " " };
        let (label, value) = setting_text(settings, field);
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!(" {marker} {label}: {value} "),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("↑/↓: Select   ←/→: Adjust   Enter: Save"));
    lines.push(Line::from("Esc or s: Cancel"));

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Settings ")
                    .style(Style::default().bg(Color::Black)),
            ),
        area,
    );
}

fn setting_text(settings: &SettingsDraft, field: SettingsField) -> (&'static str, String) {
    match field {
        SettingsField::FocusDuration => {
            ("Focus duration", format_duration(settings.focus_seconds()))
        }
        SettingsField::ShortBreakDuration => (
            "Short break",
            format_duration(settings.short_break_seconds()),
        ),
        SettingsField::LongBreakDuration => {
            ("Long break", format_duration(settings.long_break_seconds()))
        }
        SettingsField::FocusesBeforeLongBreak => (
            "Focuses before long break",
            format!("{} sessions", settings.focuses_before_long_break()),
        ),
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds % 60 == 0 {
        format!("{} min", seconds / 60)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

pub(crate) fn centered(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.min(max_width);
    let height = area.height.min(max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
