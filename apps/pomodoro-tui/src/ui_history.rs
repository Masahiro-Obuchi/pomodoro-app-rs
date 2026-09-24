//! Read-only reflection view over the adopted application state.

use ratatui::{
    Frame,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    app::App,
    controller::{Clock, CompletionNotifier, SaveStore},
    ui_settings::centered,
};

pub(crate) fn draw_history<S: SaveStore, C: Clock, N: CompletionNotifier>(
    frame: &mut Frame<'_>,
    app: &App<S, C, N>,
) {
    let area = centered(frame.area(), 54, 10);
    let content = match app.reflection() {
        Ok(summary) => format!(
            "Recorded work: {}\nFocus completed: {}\nDistractions: {}\nReturns: {}\nh/Esc: Back",
            format_work_time(summary.work_ms),
            summary.completed_focus_sessions,
            summary.distractions,
            summary.returns,
        ),
        Err(error) => format!("Could not summarize history: {error}\nh/Esc: Back"),
    };
    let content_width = area.width.saturating_sub(2);
    let required_rows = Paragraph::new(content.as_str())
        .wrap(Wrap { trim: false })
        .line_count(content_width)
        .saturating_add(2);
    if content_width == 0 || required_rows > usize::from(area.height) {
        frame.render_widget(
            Paragraph::new("h/Esc: Back\nEnlarge terminal to view history")
                .wrap(Wrap { trim: false }),
            frame.area(),
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" History ")),
        area,
    );
}

pub(crate) fn format_work_time(work_ms: u64) -> String {
    let seconds = work_ms / 1_000;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3_600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_work_time;

    #[test]
    fn recorded_work_shows_seconds_without_changing_subsecond_values() {
        for (work_ms, expected) in [
            (0, "0:00:00"),
            (999, "0:00:00"),
            (59_000, "0:00:59"),
            (61_000, "0:01:01"),
            (27 * 60_000, "0:27:00"),
            (3_600_000, "1:00:00"),
        ] {
            assert_eq!(format_work_time(work_ms), expected);
        }
    }
}
