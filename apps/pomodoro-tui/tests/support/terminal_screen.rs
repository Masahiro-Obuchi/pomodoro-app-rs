//! Screen assertions for the Linux executable's Ratatui/Crossterm output.
//!
//! Every current UI leaves the cursor unset. Ratatui draws all cell changes,
//! then Crossterm emits Hide before flushing. Use that command as the frame
//! boundary, retaining incomplete output only in the working parser. This is
//! specific to the raw Linux PTY; `ConPTY` may rewrite terminal commands.

const FRAME_END: &[u8] = b"\x1b[?25l";

pub struct TerminalScreen {
    parser: vt100::Parser,
    completed: vt100::Screen,
    end_matched: usize,
    has_frame: bool,
}

impl TerminalScreen {
    pub fn new(rows: u16, columns: u16) -> Self {
        let parser = vt100::Parser::new(rows, columns, 0);
        Self {
            completed: parser.screen().clone(),
            parser,
            end_matched: 0,
            has_frame: false,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let mut parsed = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            if byte == FRAME_END[self.end_matched] {
                self.end_matched += 1;
                if self.end_matched == FRAME_END.len() {
                    self.parser.process(&bytes[parsed..=index]);
                    self.completed = self.parser.screen().clone();
                    self.has_frame = true;
                    self.end_matched = 0;
                    parsed = index + 1;
                }
            } else {
                // ESC occurs only at the start of FRAME_END, so a mismatch
                // can start a new match but has no other overlapping prefix.
                self.end_matched = usize::from(byte == FRAME_END[0]);
            }
        }
        self.parser.process(&bytes[parsed..]);
    }

    pub fn screen(&self) -> &vt100::Screen {
        &self.completed
    }

    pub fn contains_all(&self, texts: &[&str]) -> bool {
        let visible = compact(&self.completed.contents());
        self.has_frame && texts.iter().all(|text| visible.contains(&compact(text)))
    }
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘'))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        io::{self, Write},
        rc::Rc,
    };

    use ratatui::{
        Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect,
        widgets::Paragraph,
    };

    use super::*;

    const READY: &[&str] = &["Focus · Ready", "Space: Start"];
    const HISTORY: &[&str] = &["Recorded work: 0:00:00", "h/Esc: Back"];

    #[derive(Clone, Default)]
    struct Capture(Rc<RefCell<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn frames() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let output = Capture::default();
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(output.clone()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
            },
        )
        .unwrap();
        let mut frames = Vec::new();
        for history in [false, true, false] {
            terminal
                .draw(|frame| {
                    if history {
                        frame.render_widget(
                            Paragraph::new("Recorded work: 0:00:00\n原稿を書く\nh/Esc: Back"),
                            Rect::new(10, 10, 50, 5),
                        );
                    } else {
                        frame.render_widget(Paragraph::new(READY[0]), Rect::new(0, 0, 80, 1));
                        frame.render_widget(Paragraph::new(READY[1]), Rect::new(0, 22, 80, 1));
                    }
                })
                .unwrap();
            let bytes = std::mem::take(&mut *output.0.borrow_mut());
            assert!(bytes.ends_with(FRAME_END), "draw must end with cursor Hide");
            frames.push(bytes);
        }
        let mut frames = frames.into_iter();
        (
            frames.next().unwrap(),
            frames.next().unwrap(),
            frames.next().unwrap(),
        )
    }

    #[test]
    fn mixed_history_and_old_controls_are_not_accepted_as_a_completed_frame() {
        let (ready, history, restored) = frames();
        let mut raw = vt100::Parser::new(24, 80, 0);
        raw.process(&ready);
        let split = history
            .iter()
            .position(|byte| {
                raw.process(&[*byte]);
                let visible = raw.screen().contents();
                visible.contains(HISTORY[0]) && visible.contains(READY[1])
            })
            .unwrap()
            + 1;
        assert!(
            split < history.len(),
            "fixture must contain a partial redraw"
        );

        let mut screen = TerminalScreen::new(24, 80);
        screen.process(&ready);
        screen.process(&history[..split]);
        assert!(!screen.contains_all(&[HISTORY[0]]));
        screen.process(&history[split..]);
        assert!(screen.contains_all(HISTORY));
        // This is the screen the test must have reached before it sends Esc.
        assert!(!screen.contains_all(READY));

        let end = restored.len() - FRAME_END.len();
        screen.process(&restored[..end]);
        assert!(!screen.contains_all(READY));
        screen.process(&restored[end..]);
        assert!(screen.contains_all(READY));
    }

    #[test]
    fn every_split_inside_the_frame_boundary_waits_for_the_complete_command() {
        let (ready, history, _) = frames();
        for split in (history.len() - FRAME_END.len())..history.len() {
            let mut screen = TerminalScreen::new(24, 80);
            screen.process(&ready);
            screen.process(&history[..split]);
            assert!(!screen.contains_all(HISTORY), "split: {split}");
            screen.process(&history[split..]);
            assert!(screen.contains_all(HISTORY), "split: {split}");
        }
    }

    #[test]
    fn byte_at_a_time_output_including_unicode_waits_for_the_frame_end() {
        let (ready, history, _) = frames();
        let mut screen = TerminalScreen::new(24, 80);
        screen.process(&ready);
        for (index, byte) in history.iter().enumerate() {
            screen.process(&[*byte]);
            assert_eq!(screen.contains_all(HISTORY), index + 1 == history.len());
        }
        assert!(screen.screen().contents().contains("原稿を書く"));
    }

    #[test]
    fn coalesced_frames_keep_the_latest_completed_frame_and_ignore_the_partial_tail() {
        let (ready, history, restored) = frames();
        let end = restored.len() - FRAME_END.len();
        let output = [ready.as_slice(), history.as_slice(), &restored[..end]].concat();
        let mut screen = TerminalScreen::new(24, 80);
        screen.process(&output);
        assert!(screen.contains_all(HISTORY));
        assert!(!screen.contains_all(READY));
        screen.process(&restored[end..]);
        assert!(screen.contains_all(READY));
    }
}
