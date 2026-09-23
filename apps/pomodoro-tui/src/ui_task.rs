use ratatui::{
    Frame,
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::ui_settings::centered;

pub(crate) fn draw_task(frame: &mut Frame<'_>, text: &str, message: &str) {
    let area = centered(frame.area(), 72, 9);
    let available = usize::from(area.width.saturating_sub(4));
    let value = if text.is_empty() {
        "(empty)".to_owned()
    } else {
        visible_tail(text, available)
    };
    let mut lines = vec![
        Line::from("Edit the task for your next Focus"),
        Line::from(""),
        Line::from(value),
        Line::from(""),
        Line::from("Enter: Save    Esc: Cancel"),
    ];
    if !message.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.to_owned(),
            Style::default().fg(Color::Yellow),
        )));
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Current Task · unsaved edit ")
                    .style(Style::default().bg(Color::Black)),
            ),
        area,
    );
}

// Editing only appends and removes at the end. Show that end when the full
// value does not fit, while retaining the complete draft in App.
fn visible_tail(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if Line::from(text).width() <= width {
        return text.to_owned();
    }
    let mut tail = String::new();
    let mut used = 1;
    for ch in text.chars().rev() {
        let char_width = Span::raw(ch.to_string()).width();
        if used + char_width > width {
            break;
        }
        tail.insert(0, ch);
        used += char_width;
    }
    format!("…{tail}")
}
