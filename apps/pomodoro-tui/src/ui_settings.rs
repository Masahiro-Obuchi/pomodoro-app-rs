use crate::{
    settings::{SettingsDraft, SettingsField},
    ui_footer::{controls, count_rows},
    ui_theme,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

pub(crate) fn draw_settings(frame: &mut Frame<'_>, settings: &SettingsDraft, message: &str) {
    let area = centered(frame.area(), 58, 18);
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
            ui_theme::key(ui_theme::FOCUS)
        } else {
            Style::default().fg(ui_theme::TEXT)
        };
        lines.push(Line::from(Span::styled(
            format!(" {marker} {label}: {value} "),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(controls(
        &[("↑/↓", "Select"), ("←/→", "Adjust"), ("Enter", "Save")],
        ui_theme::FOCUS,
    ));
    lines.push(controls(&[("Esc or s", "Cancel")], ui_theme::FOCUS));
    if !message.is_empty() {
        lines.push(Line::styled(
            message.to_owned(),
            Style::new().fg(ui_theme::WAITING),
        ));
    }

    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2);
    if count_rows(lines.clone(), inner_width) > inner_height {
        // Keep the selected value and every key visible while navigating in a
        // narrow terminal; other fields can be reached with the same keys.
        let (label, value) = setting_text(settings, settings.selected());
        lines = vec![
            Line::styled(label, Style::new().fg(ui_theme::MUTED)),
            Line::styled(value, ui_theme::key(ui_theme::FOCUS)),
            controls(&[("↑/↓", "Select"), ("←/→", "Adjust")], ui_theme::FOCUS),
            controls(
                &[("Enter", "Save"), ("Esc or s", "Cancel")],
                ui_theme::FOCUS,
            ),
        ];
        if !message.is_empty() {
            lines.push(Line::styled(
                message.to_owned(),
                Style::new().fg(ui_theme::WAITING),
            ));
        }
        if count_rows(lines.clone(), inner_width) > inner_height {
            lines = vec![
                Line::from("Enlarge terminal to edit settings"),
                controls(&[("Esc or s", "Cancel")], ui_theme::FOCUS),
            ];
        }
    }

    let area = centered(
        frame.area(),
        58,
        count_rows(lines.clone(), inner_width)
            .saturating_add(4)
            .min(18),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false })
            .block(ui_theme::panel(" Settings ")),
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
