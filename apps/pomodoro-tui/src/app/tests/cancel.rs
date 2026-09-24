use pomodoro_core::{InterruptionEnd, InterruptionOutcome};

use super::*;

fn ended_with(app: &TestApp, outcome: SessionOutcome) {
    let state = app.state();
    assert_eq!(state.history().sessions.len(), 1);
    assert_eq!(state.history().sessions[0].end.unwrap().outcome, outcome);
}

fn focus_ready(app: &TestApp) {
    assert!(matches!(
        app.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::Focus,
            current_task_draft: None,
        }
    ));
}

#[test]
fn cancel_ends_every_running_kind_and_keeps_credited_work() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut h = harness(started(kind), 0);
        let display = render(&h.app, 100, 30);
        assert!(display.contains("x: Cancel"), "{kind:?}: {display}");
        press(&mut h.app, '?');
        assert!(render(&h.app, 80, 25).contains("Cancel goes to Focus"));
        h.at.set(100);
        press(&mut h.app, 'x');
        ended_with(&h.app, SessionOutcome::Cancelled);
        focus_ready(&h.app);
        assert_eq!(h.app.state().history().sessions[0].elapsed_ms, 100);
        assert_eq!(
            h.app.state().reflection().unwrap().work_ms,
            if kind.is_work() { 100 } else { 0 }
        );
        assert!(h.app.message().contains("cancelled and saved"));
        assert!(!render(&h.app, 100, 30).contains("x: Cancel"));
        assert_eq!(h.app.state(), h.app.controller.saved_state());
    }
}

#[test]
fn cancel_ends_every_interruption_without_recording_a_return() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        for interruption_kind in [
            InterruptionKind::Pause,
            InterruptionKind::Distraction,
            InterruptionKind::AppExit,
            InterruptionKind::ObservationGap,
        ] {
            if interruption_kind == InterruptionKind::Distraction && !kind.is_work() {
                continue;
            }
            let mut domain = started(kind);
            match interruption_kind {
                InterruptionKind::Pause => domain.apply(Command::Pause(SessionId(1)), Timestamp(0)),
                InterruptionKind::Distraction => {
                    domain.apply(Command::Distraction(SessionId(1)), Timestamp(0))
                }
                InterruptionKind::AppExit => domain.apply(Command::CloseApp, Timestamp(0)),
                InterruptionKind::ObservationGap => domain.observe(Observation {
                    previous_at: Timestamp(0),
                    at: Timestamp(10),
                    monotonic_elapsed_ms: None,
                }),
            }
            .unwrap();
            let initial_at = if interruption_kind == InterruptionKind::ObservationGap {
                10
            } else {
                0
            };
            let mut h = harness(domain, initial_at);
            let display = render(&h.app, 100, 30);
            assert!(
                display.contains("x: Cancel"),
                "{kind:?} {interruption_kind:?}: {display}"
            );
            h.at.set(100);
            press(&mut h.app, 'x');
            ended_with(&h.app, SessionOutcome::Cancelled);
            focus_ready(&h.app);
            assert_eq!(h.app.state().reflection().unwrap().returns, 0);
            let ended = h
                .app
                .state()
                .history()
                .events
                .iter()
                .filter_map(|event| match event.payload {
                    EventKind::InterruptionEnded { end, .. } => Some(end),
                    _ => None,
                })
                .collect::<Vec<InterruptionEnd>>();
            assert_eq!(ended.len(), 1);
            assert_eq!(
                ended[0].outcome,
                InterruptionOutcome::SessionEnded {
                    session_outcome: SessionOutcome::Cancelled
                }
            );
        }
    }
}

#[test]
fn cancel_is_unavailable_in_ready_and_quick_start_decision() {
    for domain in [ready(), awaiting()] {
        let mut h = harness(domain, 120_000);
        let before = h.app.state().clone();
        assert!(!render(&h.app, 100, 30).contains("x: Cancel"));
        press(&mut h.app, 'x');
        assert_eq!(h.app.state(), &before);
        assert!(h.log.borrow().is_empty());
    }
    let mut h = harness(ready(), 0);
    press(&mut h.app, 't');
    press(&mut h.app, 'x');
    assert_eq!(h.app.task_edit(), Some("x"));
    assert!(h.app.state().history().sessions.is_empty());
}

#[test]
fn reset_skip_and_cancel_keep_distinct_results_and_next_states() {
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        for (key, outcome) in [
            ('r', SessionOutcome::Reset),
            ('n', SessionOutcome::Skipped),
            ('x', SessionOutcome::Cancelled),
        ] {
            let mut h = harness(started(kind), 0);
            h.at.set(100);
            press(&mut h.app, key);
            ended_with(&h.app, outcome);
            let ProgressState::Ready {
                next_kind,
                current_task_draft,
            } = &h.app.state().snapshot().state
            else {
                panic!("expected ready")
            };
            assert_eq!(
                *next_kind,
                match outcome {
                    SessionOutcome::Reset => kind,
                    SessionOutcome::Skipped if kind == SessionKind::Focus => {
                        SessionKind::ShortBreak
                    }
                    SessionOutcome::Skipped | SessionOutcome::Cancelled => SessionKind::Focus,
                    SessionOutcome::Completed => unreachable!(),
                }
            );
            assert_eq!(
                current_task_draft.is_some(),
                outcome == SessionOutcome::Reset
            );
            let message = h.app.message();
            assert!(match outcome {
                SessionOutcome::Reset => message.contains("Reset to ready"),
                SessionOutcome::Skipped => message.contains("Session skipped and saved"),
                SessionOutcome::Cancelled => message.contains("Session cancelled and saved"),
                SessionOutcome::Completed => unreachable!(),
            });
            assert_eq!(h.app.state().history().sessions[0].elapsed_ms, 100);
        }
    }
}

#[test]
fn cancelling_long_break_resets_the_round_while_reset_keeps_it() {
    let mut domain = ready();
    for at in [0, 1_000] {
        domain
            .apply(Command::Start(SessionKind::Focus), Timestamp(at))
            .unwrap();
        domain
            .observe(Observation {
                previous_at: Timestamp(at),
                at: Timestamp(at + 1_000),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
        if at == 0 {
            domain.apply(Command::SkipReady, Timestamp(1_000)).unwrap();
        }
    }
    assert_eq!(
        domain.snapshot().round_progress.completed_focuses_in_round,
        2
    );
    for (key, expected_kind, expected_round) in [
        ('x', SessionKind::Focus, 0),
        ('r', SessionKind::LongBreak, 2),
        ('n', SessionKind::Focus, 0),
    ] {
        let mut h = harness(domain.clone(), 2_000);
        press(&mut h.app, ' ');
        h.at.set(2_100);
        press(&mut h.app, key);
        let ProgressState::Ready { next_kind, .. } = h.app.state().snapshot().state else {
            panic!("expected ready")
        };
        assert_eq!(next_kind, expected_kind);
        assert_eq!(
            h.app
                .state()
                .snapshot()
                .round_progress
                .completed_focuses_in_round,
            expected_round
        );
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 2_000);
    }
}

#[test]
fn cancel_at_natural_completion_keeps_the_completed_result() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.log.borrow_mut().clear();
    h.at.set(1_000);
    press(&mut h.app, 'x');
    ended_with(&h.app, SessionOutcome::Completed);
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::ShortBreak,
            ..
        }
    ));
    assert_eq!(*h.log.borrow(), ["saved", "notify"]);
    assert!(!h.app.message().contains("cancelled"));
}

#[test]
fn failed_cancel_keeps_last_saved_state_and_retries_one_ended_session() {
    for interrupted in [false, true] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, '2');
        h.at.set(100);
        if interrupted {
            press(&mut h.app, 'd');
        }
        h.log.borrow_mut().clear();
        h.failures.set(1);
        press(&mut h.app, 'x');
        assert!(h.app.pending_state().is_some());
        assert!(matches!(
            h.app.pending_state().unwrap().snapshot().state,
            ProgressState::Ready {
                next_kind: SessionKind::Focus,
                ..
            }
        ));
        assert!(matches!(
            h.app.state().snapshot().state,
            ProgressState::Active { .. }
        ));
        assert!(!h.app.message().contains("cancelled and saved"));
        let pending = h.app.pending_state().unwrap().clone();
        for key in ['x', ' ', 'n'] {
            press(&mut h.app, key);
        }
        assert_eq!(h.app.pending_state(), Some(&pending));
        assert_eq!(*h.log.borrow(), ["failed"]);
        h.at.set(200);
        press(&mut h.app, 'r');
        ended_with(&h.app, SessionOutcome::Cancelled);
        focus_ready(&h.app);
        assert!(h.app.pending_state().is_none());
        assert!(h.app.message().contains("cancelled and saved"));
        assert_eq!(*h.log.borrow(), ["failed", "saved"]);
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
        assert_eq!(h.app.state().reflection().unwrap().returns, 0);
        let ended = h
            .app
            .state()
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::InterruptionEnded { .. }))
            .count();
        assert_eq!(ended, usize::from(interrupted));
    }
}
