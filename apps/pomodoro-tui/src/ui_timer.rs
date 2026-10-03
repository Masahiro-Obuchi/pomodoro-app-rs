//! A seven-pixel clock packed into four terminal rows; it never samples time.

use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

pub(crate) const CLOCK_HEIGHT: u16 = 4;

pub(crate) fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1_000);
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn clock_width(text: &str) -> u16 {
    let width = text
        .chars()
        .map(|ch| if ch == ':' { 4 } else { 6 })
        .sum::<usize>();
    u16::try_from(width.saturating_sub(1)).unwrap_or(u16::MAX)
}

pub(crate) struct BigClock<'a> {
    pub(crate) text: &'a str,
    pub(crate) style: Style,
}

impl Widget for BigClock<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let width = clock_width(self.text);
        if area.width < width || area.height < CLOCK_HEIGHT {
            return;
        }
        let left = area.x + (area.width - width) / 2;
        for row in 0..CLOCK_HEIGHT {
            let mut line = String::new();
            for ch in self.text.chars() {
                let (bits, columns) = glyph(ch);
                let upper = bits[usize::from(row) * 2];
                let lower = bits.get(usize::from(row) * 2 + 1).copied().unwrap_or(0);
                for column in (0..columns).rev() {
                    line.push(
                        match (upper & (1 << column) != 0, lower & (1 << column) != 0) {
                            (true, true) => '█',
                            (true, false) => '▀',
                            (false, true) => '▄',
                            (false, false) => ' ',
                        },
                    );
                }
                line.push(' ');
            }
            line.pop();
            buffer.set_string(left, area.y + row, line, self.style);
        }
    }
}

fn glyph(ch: char) -> ([u8; 7], u8) {
    let rows = match ch {
        '0' => [31, 17, 17, 17, 17, 17, 31],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [30, 1, 1, 14, 16, 16, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [18, 18, 18, 31, 2, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [15, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 30],
        ':' => return ([0, 2, 2, 0, 2, 2, 0], 3),
        _ => [0; 7],
    };
    (rows, 5)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    use super::*;

    #[test]
    fn durations_keep_fractional_seconds_and_all_supported_minute_digits() {
        for (ms, text) in [
            (0, "00:00"),
            (1, "00:01"),
            (999, "00:01"),
            (1_000, "00:01"),
            (599_000, "09:59"),
            (600_000, "10:00"),
            (6_000_000, "100:00"),
            (86_400_000, "1440:00"),
        ] {
            assert_eq!(format_time(ms), text);
        }
        assert_eq!(clock_width("25:00"), 27);
        assert_eq!(clock_width("1440:00"), 39);
    }

    #[test]
    fn clock_is_centered_and_never_draws_a_partial_number() {
        for (width, height, visible) in [(39, 4, true), (38, 4, false), (39, 3, false)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(
                        BigClock {
                            text: "1440:00",
                            style: Style::new().fg(Color::Green),
                        },
                        frame.area(),
                    );
                })
                .unwrap();
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .any(|cell| cell.symbol() == "█"),
                visible
            );
        }
    }
}
