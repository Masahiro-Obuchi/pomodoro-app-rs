use crossterm::event::Event;

use super::*;

fn with_task() -> Harness {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    h.app.handle_event(Event::Paste("原稿を書く".into()));
    h.app.handle_key(KeyCode::Enter);
    h
}

fn complete_quick_start(h: &mut Harness) {
    press(&mut h.app, '2');
    for at in (1_000..120_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    assert_running(&h.app);
    h.log.borrow_mut().clear();
    h.at.set(120_000);
    h.app.tick();
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
}

#[test]
fn quick_start_key_works_only_while_waiting_for_focus_and_inherits_the_task() {
    let mut h = with_task();
    assert!(render(&h.app, 100, 30).contains("2: Quick Start (2 min)"));
    press(&mut h.app, '2');
    let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
        panic!("expected Quick Start")
    };
    assert_eq!(session.kind, SessionKind::QuickStart);
    assert_eq!(session.planned_duration_ms, 120_000);
    assert_eq!(
        session.current_task.as_ref().unwrap().as_str(),
        "原稿を書く"
    );
    assert!(h.app.message().contains("Quick Start started and saved"));
    assert!(render(&h.app, 100, 30).contains("Quick Start · Running"));
    assert!(!render(&h.app, 100, 30).contains("2: Quick Start"));
    let active = h.app.state().clone();
    press(&mut h.app, '2');
    assert_eq!(h.app.state(), &active);

    let mut normal = harness(ready(), 0);
    press(&mut normal.app, ' ');
    let ProgressState::Active { session, .. } = &normal.app.state().snapshot().state else {
        panic!("expected Focus")
    };
    assert_eq!(session.kind, SessionKind::Focus);

    let mut break_ready = started(SessionKind::ShortBreak);
    break_ready
        .apply(
            Command::End {
                session_id: SessionId(1),
                outcome: SessionOutcome::Reset,
            },
            Timestamp(0),
        )
        .unwrap();
    let mut h = harness(break_ready, 0);
    assert!(!render(&h.app, 100, 30).contains("2: Quick Start"));
    let before = h.app.state().clone();
    press(&mut h.app, '2');
    assert_eq!(h.app.state(), &before);
    press(&mut h.app, 'n');
    assert!(render(&h.app, 100, 30).contains("2: Quick Start"));
    press(&mut h.app, '2');
    let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
        panic!("expected Quick Start")
    };
    assert_eq!(session.kind, SessionKind::QuickStart);
}

#[test]
fn two_minute_completion_waits_for_finish_or_a_new_full_focus() {
    for choice in ['f', 'c'] {
        let mut h = with_task();
        complete_quick_start(&mut h);
        assert_eq!(*h.log.borrow(), ["saved", "notify"]);
        assert_eq!(h.app.state().history().sessions.len(), 1);
        assert_eq!(
            h.app.state().history().sessions[0].end.unwrap().outcome,
            SessionOutcome::Completed
        );
        assert_eq!(h.app.reflection().unwrap().completed_focus_sessions, 0);
        assert_eq!(
            h.app
                .state()
                .snapshot()
                .round_progress
                .completed_focuses_in_round,
            0
        );
        let display = render(&h.app, 100, 30);
        assert!(display.contains("f: Finish Quick Start"));
        assert!(display.contains("c: Continue to Focus"));
        assert!(display.contains("Choice time is not counted"));
        h.at.set(180_000);
        h.app.tick();
        assert!(matches!(
            h.app.state().snapshot().state,
            ProgressState::AwaitingQuickStartDecision { .. }
        ));
        let waiting = h.app.state().clone();
        press(&mut h.app, '2');
        assert_eq!(h.app.state(), &waiting);
        press(&mut h.app, choice);
        assert_eq!(
            h.app
                .state()
                .history()
                .events
                .iter()
                .filter(|event| matches!(event.payload, EventKind::QuickStartDecisionMade { .. }))
                .count(),
            1
        );
        assert_eq!(h.app.reflection().unwrap().completed_focus_sessions, 0);
        if choice == 'f' {
            assert!(matches!(
                h.app.state().snapshot().state,
                ProgressState::Ready {
                    next_kind: SessionKind::Focus,
                    ..
                }
            ));
            assert_eq!(h.app.state().history().sessions.len(), 1);
        } else {
            let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
                panic!("expected Focus")
            };
            assert_eq!(session.kind, SessionKind::Focus);
            assert_eq!(session.id, SessionId(2));
            assert_eq!(session.planned_duration_ms, 1_000);
            assert_eq!(session.elapsed_ms, 0);
            assert_eq!(
                session.current_task.as_ref().unwrap().as_str(),
                "原稿を書く"
            );
            assert_eq!(session.continued_from_quick_start, Some(SessionId(1)));
        }
    }
}

#[test]
fn start_completion_and_choice_failures_retry_each_fixed_candidate_once() {
    let mut h = harness(ready(), 0);
    h.failures.set(1);
    press(&mut h.app, '2');
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Ready { .. }
    ));
    assert!(!h.app.message().contains("Quick Start started"));
    let candidate = h.app.pending_state().unwrap().clone();
    press(&mut h.app, ' ');
    assert_eq!(h.app.pending_state(), Some(&candidate));
    h.at.set(100);
    press(&mut h.app, 'r');
    let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
        panic!("expected saved Quick Start")
    };
    assert_eq!(session.kind, SessionKind::QuickStart);
    assert_eq!(session.id, SessionId(1));
    assert_eq!(h.app.state().id_allocators().next_session_id, 2);
    assert!(h.app.message().contains("Quick Start started and saved"));
    assert_eq!(*h.log.borrow(), ["failed", "saved", "saved"]);

    let mut h = with_task();
    press(&mut h.app, '2');
    for at in (1_000..120_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    h.failures.set(1);
    h.log.borrow_mut().clear();
    h.at.set(120_000);
    h.app.tick();
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Active { .. }
    ));
    assert!(!h.app.message().contains("Quick Start complete"));
    assert_eq!(*h.log.borrow(), ["failed"]);
    h.at.set(120_100);
    press(&mut h.app, 'r');
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    assert_eq!(
        h.log
            .borrow()
            .iter()
            .filter(|entry| **entry == "notify")
            .count(),
        1
    );

    for choice in ['f', 'c'] {
        let mut h = with_task();
        complete_quick_start(&mut h);
        h.log.borrow_mut().clear();
        h.failures.set(1);
        h.at.set(180_000);
        press(&mut h.app, choice);
        let candidate = h.app.pending_state().unwrap().clone();
        assert!(matches!(
            h.app.state().snapshot().state,
            ProgressState::AwaitingQuickStartDecision { .. }
        ));
        assert!(!h.app.message().contains("choice saved"));
        press(&mut h.app, if choice == 'f' { 'c' } else { 'f' });
        assert_eq!(h.app.pending_state(), Some(&candidate));
        h.at.set(180_100);
        press(&mut h.app, 'r');
        assert!(h.app.message().contains("choice saved"));
        assert_eq!(
            h.app
                .state()
                .history()
                .events
                .iter()
                .filter(|event| matches!(event.payload, EventKind::QuickStartDecisionMade { .. }))
                .count(),
            1
        );
        assert_eq!(
            h.app.state().id_allocators().next_session_id,
            if choice == 'f' { 2 } else { 3 }
        );
        assert!(!h.log.borrow().contains(&"notify"));
    }
}
