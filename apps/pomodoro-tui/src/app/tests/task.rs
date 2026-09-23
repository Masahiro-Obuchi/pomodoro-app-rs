use crossterm::event::{Event, KeyEvent, KeyModifiers};

use super::*;

fn draft(app: &TestApp) -> Option<&CurrentTask> {
    let ProgressState::Ready {
        current_task_draft, ..
    } = &app.state().snapshot().state
    else {
        panic!("expected ready")
    };
    current_task_draft.as_ref()
}

#[test]
fn task_editor_keeps_draft_unsaved_until_enter_and_restores_it_for_reedit() {
    let mut h = harness(ready(), 0);
    assert!(render(&h.app, 100, 30).contains("t: Edit task"));
    press(&mut h.app, 't');
    assert_eq!(h.app.task_edit(), Some(""));
    for ch in "  原稿をq?2書く  ".chars() {
        press(&mut h.app, ch);
    }
    h.app.handle_key(KeyCode::Backspace);
    assert_eq!(h.app.task_edit(), Some("  原稿をq?2書く "));
    press(&mut h.app, ' ');
    press(&mut h.app, 'く');
    assert!(draft(&h.app).is_none());
    assert!(h.log.borrow().is_empty());
    assert!(render(&h.app, 100, 30).contains("Enter: Save"));
    h.app.tick();
    assert!(h.log.borrow().is_empty());
    h.app.handle_key(KeyCode::Esc);
    assert!(h.app.task_edit().is_none());
    assert!(draft(&h.app).is_none());

    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("  原稿を書く  ".into()));
    h.app.handle_key(KeyCode::Enter);
    assert!(h.app.task_edit().is_none());
    assert_eq!(draft(&h.app).unwrap().as_str(), "原稿を書く");
    assert_eq!(*h.log.borrow(), ["saved"]);
    assert_eq!(h.app.state(), h.app.controller.saved_state());
    press(&mut h.app, 't');
    assert_eq!(h.app.task_edit(), Some("原稿を書く"));
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(*h.log.borrow(), ["saved"]);
    assert_eq!(h.app.message(), "Task unchanged.");
    press(&mut h.app, ' ');
    let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
        panic!("expected active")
    };
    assert_eq!(
        session.current_task.as_ref().unwrap().as_str(),
        "原稿を書く"
    );
    press(&mut h.app, 'r');
    assert_eq!(draft(&h.app).unwrap().as_str(), "原稿を書く");
}

#[test]
fn task_editor_accepts_blank_and_blocks_commands_and_modified_characters() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    for key in ['q', '?', '2', 'n', 's', ' '] {
        press(&mut h.app, key);
    }
    assert!(!h.app.should_quit());
    assert!(!h.app.show_help());
    assert!(h.app.settings().is_none());
    assert!(h.log.borrow().is_empty());
    h.app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::CONTROL,
    )));
    h.app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::ALT,
    )));
    assert_eq!(h.app.task_edit(), Some("q?2ns "));
    h.app.handle_key(KeyCode::Esc);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("   ".into()));
    h.app.handle_key(KeyCode::Enter);
    assert!(draft(&h.app).is_none());
    assert!(h.log.borrow().is_empty());
}

#[test]
fn pasted_control_characters_are_rejected_without_changing_the_draft() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("safe".into()));
    for text in ["a\tb", "a\u{001b}[31mb", "a\u{0008}b", "a\u{007f}b"] {
        h.app.handle_event(Event::Paste(text.into()));
        assert_eq!(h.app.task_edit(), Some("safe"));
        assert!(h.app.message().contains("control characters"));
        assert!(draft(&h.app).is_none());
        assert!(h.log.borrow().is_empty());
    }
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(draft(&h.app).unwrap().as_str(), "safe");
}

#[test]
fn backspace_removes_one_whole_grapheme() {
    for grapheme in ["e\u{301}", "👩‍👩‍👧‍👦", "🇯🇵"] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, 't');
        h.app.handle_event(Event::Paste(format!("task{grapheme}")));
        h.app.handle_key(KeyCode::Backspace);
        assert_eq!(h.app.task_edit(), Some("task"));
    }
}

#[test]
fn multiline_paste_is_rejected_as_one_event_and_never_acts_as_commands() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("safe\nq n".into()));
    assert_eq!(h.app.task_edit(), Some(""));
    assert!(h.app.message().contains("single line"));
    h.app.handle_event(Event::Paste("\n".into()));
    assert_eq!(h.app.task_edit(), Some(""));
    h.app.handle_event(Event::Paste("safe\u{2028}q".into()));
    assert_eq!(h.app.task_edit(), Some(""));
    assert!(!h.app.should_quit());
    assert!(h.log.borrow().is_empty());
    h.app.handle_event(Event::Paste("新しい作業".into()));
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(draft(&h.app).unwrap().as_str(), "新しい作業");
    h.app.handle_event(Event::Paste("qn".into()));
    assert!(!h.app.should_quit());
    assert_eq!(draft(&h.app).unwrap().as_str(), "新しい作業");
}

#[test]
fn task_editor_is_only_available_before_focus_and_not_inside_settings() {
    for domain in [
        started(SessionKind::Focus),
        started(SessionKind::QuickStart),
        started(SessionKind::ShortBreak),
        awaiting(),
    ] {
        let mut h = harness(domain, 0);
        assert!(!render(&h.app, 100, 30).contains("t: Edit task"));
        press(&mut h.app, 't');
        assert!(h.app.task_edit().is_none());
    }
    let mut h = harness(ready(), 0);
    press(&mut h.app, 's');
    press(&mut h.app, 't');
    assert!(h.app.task_edit().is_none());
    h.app.handle_key(KeyCode::Esc);
    press(&mut h.app, 't');
    press(&mut h.app, 's');
    assert!(h.app.settings().is_none());
    h.app.handle_key(KeyCode::Esc);
    press(&mut h.app, 'n');
    assert!(!render(&h.app, 100, 30).contains("t: Edit task"));
    press(&mut h.app, 't');
    assert!(h.app.task_edit().is_none());
}

#[test]
fn task_save_failure_closes_editor_and_retry_confirms_only_one_candidate() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("review".into()));
    h.failures.set(1);
    h.app.handle_key(KeyCode::Enter);
    assert!(h.app.task_edit().is_none());
    assert!(draft(&h.app).is_none());
    assert!(h.app.pending_state().is_some());
    assert!(!h.app.message().contains("Task saved"));
    press(&mut h.app, 't');
    assert!(h.app.task_edit().is_none());
    press(&mut h.app, 'r');
    assert_eq!(draft(&h.app).unwrap().as_str(), "review");
    assert!(h.app.message().contains("Task saved"));
    assert_eq!(*h.log.borrow(), ["failed", "saved"]);
    assert!(h.app.state().history().events.is_empty());
}

#[test]
fn clearing_an_existing_task_saves_none_and_unsaved_exit_never_reports_success() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("review".into()));
    h.app.handle_key(KeyCode::Enter);
    press(&mut h.app, 't');
    for _ in 0..6 {
        h.app.handle_key(KeyCode::Backspace);
    }
    assert_eq!(h.app.task_edit(), Some(""));
    h.app.handle_key(KeyCode::Enter);
    assert!(draft(&h.app).is_none());
    assert_eq!(*h.log.borrow(), ["saved", "saved"]);

    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("unconfirmed".into()));
    h.failures.set(1);
    h.app.handle_key(KeyCode::Enter);
    press(&mut h.app, 'Q');
    press(&mut h.app, 'y');
    assert!(h.app.should_quit());
    assert!(draft(&h.app).is_none());
    assert!(!h.app.message().contains("Task saved"));
    assert_eq!(h.app.exit(), ExitOutcome::Unsaved);
}

#[test]
fn clock_failure_keeps_task_text_for_retry_and_long_input_survives_narrow_view() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    let long = "日本語の長い作業名".repeat(20);
    h.app.handle_event(Event::Paste(long.clone()));
    let display = render(&h.app, 30, 25);
    assert!(display.contains("Enter: Save"));
    assert!(display.contains("Esc: Cancel"));
    assert_eq!(h.app.task_edit(), Some(long.as_str()));
    h.clock_failure.set(true);
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(h.app.task_edit(), Some(long.as_str()));
    assert!(h.app.message().contains("Action failed"));
    assert!(h.log.borrow().is_empty());
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(draft(&h.app).unwrap().as_str(), long);
}
