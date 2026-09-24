use super::*;

fn has(screen: &str, phrase: &str) -> bool {
    let compact = |text: &str| {
        text.chars()
            .filter(|ch| !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘'))
            .collect::<String>()
    };
    compact(screen).contains(&compact(phrase))
}

#[test]
fn history_view_tracks_live_work_and_distraction_without_dispatching_commands() {
    let domain = DomainState::new(TimerConfig::new(120, 1, 1, 2).unwrap()).unwrap();
    let mut h = harness(domain, 0);
    press(&mut h.app, ' ');
    let saved_actions = h.log.borrow().len();
    press(&mut h.app, 'h');
    assert!(h.app.history_open());
    let before = h.app.state().clone();
    press(&mut h.app, 'q');
    press(&mut h.app, 'd');
    assert!(!h.app.should_quit());
    assert_eq!(h.app.state(), &before);
    assert_eq!(h.log.borrow().len(), saved_actions);

    for at in (1_000..=61_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    assert_eq!(h.app.reflection().unwrap().work_ms, 61_000);
    let live = render(&h.app, 80, 24);
    for phrase in ["Recorded work: 0:01:01", "Focus completed: 0", "Returns: 0"] {
        assert!(live.contains(phrase), "missing {phrase}: {live}");
    }

    press(&mut h.app, 'h');
    h.at.set(62_000);
    press(&mut h.app, 'd');
    press(&mut h.app, 'h');
    let distracted = render(&h.app, 80, 24);
    assert!(distracted.contains("Distractions: 1"), "{distracted}");
    h.at.set(63_000);
    h.app.tick();
    assert_eq!(h.app.reflection().unwrap().work_ms, 62_000);
    press(&mut h.app, 'h');
    press(&mut h.app, ' ');
    press(&mut h.app, 'h');
    let returned = render(&h.app, 80, 24);
    assert!(returned.contains("Returns: 1"), "{returned}");
}

#[test]
fn task_and_settings_editors_take_priority_over_history() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    press(&mut h.app, 'h');
    assert_eq!(h.app.task_edit(), Some("h"));
    assert!(!h.app.history_open());
    h.app.handle_key(KeyCode::Esc);

    press(&mut h.app, 's');
    press(&mut h.app, 'h');
    assert!(h.app.settings().is_some());
    assert!(!h.app.history_open());
    h.app.handle_key(KeyCode::Esc);

    let saved = h.app.state().clone();
    press(&mut h.app, 'h');
    assert!(h.app.history_open());
    h.app.handle_key(KeyCode::Esc);
    assert!(!h.app.history_open());
    assert_eq!(h.app.state(), &saved);
}

#[test]
fn save_failure_hides_history_until_recovery_and_excludes_pending_work() {
    let domain = DomainState::new(TimerConfig::new(10, 1, 1, 2).unwrap()).unwrap();
    let mut h = harness(domain, 0);
    press(&mut h.app, ' ');
    press(&mut h.app, 'h');
    h.failures.set(1);
    for at in (1_000..=5_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    assert!(h.app.pending_state().is_some());
    assert_eq!(h.app.reflection().unwrap().work_ms, 0);
    let blocked = render(&h.app, 30, 25);
    for phrase in ["r: Retry save", "Q: Confirm unsaved exit"] {
        assert!(has(&blocked, phrase), "missing {phrase}: {blocked}");
    }
    assert!(!blocked.contains("Recorded work:"), "{blocked}");
    press(&mut h.app, 'Q');
    let confirming = render(&h.app, 30, 25);
    assert!(has(&confirming, "y: Exit unsaved"), "{confirming}");
    assert!(!confirming.contains("Recorded work:"), "{confirming}");
    press(&mut h.app, 'n');
    press(&mut h.app, 'r');
    assert!(h.app.pending_state().is_none());
    assert_eq!(h.app.reflection().unwrap().work_ms, 5_000);
    assert!(render(&h.app, 80, 24).contains("Recorded work: 0:00:05"));
}
