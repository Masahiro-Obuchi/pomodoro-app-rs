//! Display-width-aware text and key hints shared by the terminal views.

use ratatui::{
    style::Color,
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui_theme;

pub(crate) fn hint(key: &str, description: &str, color: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_owned(), ui_theme::key(color)),
        Span::raw(format!(": {description}")),
    ]
}

pub(crate) fn shorten(text: &str, width: usize) -> String {
    if Line::from(text).width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut used = 1;
    for grapheme in text.graphemes(true) {
        let size = Span::raw(grapheme).width();
        if used + size > width {
            break;
        }
        output.push_str(grapheme);
        used += size;
    }
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortening_respects_graphemes_and_display_columns() {
        assert_eq!(shorten("原稿を書く", 5), "原稿…");
        assert_eq!(shorten("e\u{301}criture", 3), "e\u{301}c…");
        assert_eq!(shorten("👩‍👩‍👧‍👦task", 3), "👩‍👩‍👧‍👦…");
        assert_eq!(shorten("text", 0), "");
    }
}
