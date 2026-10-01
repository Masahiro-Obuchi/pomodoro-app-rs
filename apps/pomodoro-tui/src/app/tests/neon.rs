use ratatui::{buffer::Buffer, style::Color};

use super::*;
use crate::ui_theme;

fn buffer(app: &TestApp, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn assert_clock_color(app: &TestApp, color: Color) {
    let rendered = buffer(app, 80, 24);
    assert_eq!(rendered[(0, 0)].bg, ui_theme::BACKGROUND);
    let clock = rendered
        .content
        .iter()
        .find(|cell| cell.symbol() == "█")
        .unwrap();
    assert_eq!(clock.fg, color);
    assert_eq!(clock.bg, ui_theme::BACKGROUND);
}

#[test]
fn clock_and_status_distinguish_work_break_interruption_and_save_recovery() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut h = harness(started(kind), 0);
        assert_clock_color(
            &h.app,
            if kind.is_work() {
                ui_theme::FOCUS
            } else {
                ui_theme::BREAK
            },
        );
        press(&mut h.app, ' ');
        assert_clock_color(&h.app, ui_theme::WAITING);
        assert!(render(&h.app, 80, 24).contains("Paused"));
    }
    assert_clock_color(&harness(awaiting(), 120_000).app, ui_theme::WAITING);

    let mut h = harness(started(SessionKind::Focus), 0);
    h.failures.set(1);
    press(&mut h.app, 'd');
    assert_clock_color(&h.app, ui_theme::ERROR);
    let screen = render(&h.app, 80, 24);
    assert!(screen.contains("Last saved: Focus · Running"), "{screen}");
    assert!(
        screen.contains("Unconfirmed save: Focus · Distracted"),
        "{screen}"
    );
    assert_metric(&screen, "Distractions", "0");
    press(&mut h.app, 'Q');
    assert_clock_color(&h.app, ui_theme::ERROR);
}

#[test]
fn resizing_restores_the_clock_without_leaving_old_glyphs() {
    let h = harness(DomainState::new(TimerConfig::default()).unwrap(), 0);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    for (width, height, large) in [
        (80, 24, true),
        (60, 24, true),
        (24, 17, false),
        (80, 24, true),
    ] {
        terminal.backend_mut().resize(width, height);
        terminal
            .draw(|frame| crate::ui::draw(frame, &h.app))
            .unwrap();
        assert_eq!(terminal.backend().buffer(), &buffer(&h.app, width, height));
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "█"),
            large
        );
        let screen = render(&h.app, width, height);
        assert!(screen.contains("25:00"), "{screen}");
        assert!(screen.contains("Space: Start"), "{screen}");
    }
}

#[test]
fn maximum_duration_and_unicode_task_fit_without_losing_digits_or_graphemes() {
    let mut domain =
        DomainState::new(TimerConfig::new(86_400, 86_400, 86_400, u32::MAX).unwrap()).unwrap();
    let task = format!("{}e\u{301}👩‍👩‍👧‍👦", "原稿を書く".repeat(8));
    domain
        .apply(
            Command::SetCurrentTask(CurrentTask::parse(&task).unwrap()),
            Timestamp(0),
        )
        .unwrap();
    let h = harness(domain, 0);
    for (width, height) in [(100, 30), (80, 24), (60, 24), (30, 25), (24, 17)] {
        let screen = render(&h.app, width, height);
        assert!(screen.contains("1440:00"), "{screen}");
        assert!(screen.contains("Space: Start"), "{screen}");
        assert!(!screen.contains('�'), "{screen}");
    }
    let screen = render(&h.app, 80, 24);
    assert!(screen.contains("Round 0/4294967295"), "{screen}");
    assert!(screen.contains('…'), "{screen}");
}

#[test]
fn repeated_drawing_never_observes_time_changes_state_saves_or_notifies() {
    let mut h = harness(started(SessionKind::Focus), 0);
    let state = h.app.state().clone();
    let saved = h.app.controller.saved_state().clone();
    let log = h.log.borrow().clone();
    h.at.set(1_000);
    h.clock_failure.set(true);
    for key in [None, Some('h'), Some('h'), Some('?')] {
        if let Some(key) = key {
            press(&mut h.app, key);
        }
        for (width, height) in [(100, 30), (80, 24), (24, 17)] {
            render(&h.app, width, height);
        }
    }
    assert_eq!(h.app.state(), &state);
    assert_eq!(h.app.controller.saved_state(), &saved);
    assert_eq!(*h.log.borrow(), log);
    assert!(
        h.clock_failure.get(),
        "render must not consume a clock observation"
    );
}

#[test]
fn narrow_settings_keeps_selected_value_and_every_control_visible() {
    let mut h = harness(
        DomainState::new(TimerConfig::new(86_400, 86_400, 86_400, u32::MAX).unwrap()).unwrap(),
        0,
    );
    press(&mut h.app, 's');
    for _ in 0..4 {
        let screen = render(&h.app, 24, 17);
        let compact: String = screen
            .chars()
            .filter(|ch| !ch.is_whitespace() && !matches!(ch, '│' | '─'))
            .collect();
        for control in ["↑/↓:Select", "←/→:Adjust", "Enter:Save", "Escors:Cancel"] {
            assert!(compact.contains(control), "missing {control}: {screen}");
        }
        h.app.handle_key(KeyCode::Down);
    }
    let screen = render(&h.app, 24, 6);
    assert!(screen.contains("Enlarge terminal"), "{screen}");
}
