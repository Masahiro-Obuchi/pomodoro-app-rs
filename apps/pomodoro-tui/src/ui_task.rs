use ratatui::{
    Frame,
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

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
        Line::from(value),
        Line::from("Enter: Save"),
        Line::from("Esc: Cancel"),
    ];
    if !message.is_empty() {
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
    for grapheme in text.graphemes(true).rev() {
        let grapheme_width = Span::raw(grapheme).width();
        if used + grapheme_width > width {
            break;
        }
        tail.insert_str(0, grapheme);
        used += grapheme_width;
    }
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn render_editor(width: u16, height: u16, message: &str) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw_task(frame, "draft", message))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut output = String::new();
        for y in 0..height {
            for x in 0..width {
                output.push_str(buffer[(x, y)].symbol());
            }
            output.push('\n');
        }
        output
    }

    #[test]
    fn errors_and_controls_remain_visible_in_narrow_editor() {
        for (width, height) in [(30, 25), (24, 17)] {
            for (message, visible) in [
                (
                    "Task must be a single line; paste was rejected.",
                    "rejected.",
                ),
                (
                    "Task cannot contain control characters; paste was rejected.",
                    "control",
                ),
            ] {
                let screen = render_editor(width, height, message);
                assert!(screen.contains("draft"), "{width}x{height}: {screen}");
                assert!(screen.contains("Enter: Save"), "{width}x{height}: {screen}");
                assert!(screen.contains("Esc: Cancel"), "{width}x{height}: {screen}");
                assert!(screen.contains(visible), "{width}x{height}: {screen}");
                assert!(screen.contains("rejected."), "{width}x{height}: {screen}");
            }
        }
    }

    #[test]
    fn long_input_tail_keeps_combining_marks_and_emoji_together() {
        assert_eq!(visible_tail("abcde\u{301}", 3), "…de\u{301}");
        assert_eq!(visible_tail("prefix👩‍👩‍👧‍👦", 3), "…👩‍👩‍👧‍👦");
    }
}
