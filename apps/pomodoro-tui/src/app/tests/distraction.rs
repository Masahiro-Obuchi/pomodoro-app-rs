use pomodoro_core::{InterruptionEnd, InterruptionOutcome, MeasuredDuration};

use super::*;

fn interruption(app: &TestApp) -> pomodoro_core::InterruptionId {
    let ProgressState::Active {
        timer: TimerState::Interrupted { interruption },
        ..
    } = &app.state().snapshot().state
    else {
        panic!("expected interrupted session")
    };
    assert_eq!(interruption.kind, InterruptionKind::Distraction);
    interruption.id
}

fn count_distractions(app: &TestApp) -> usize {
    app.state()
        .history()
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.payload,
                EventKind::InterruptionStarted {
                    interruption_kind: InterruptionKind::Distraction,
                    ..
                }
            )
        })
        .count()
}

fn returns(app: &TestApp) -> Vec<InterruptionEnd> {
    app.state()
        .history()
        .events
        .iter()
        .filter_map(|event| match event.payload {
            EventKind::InterruptionEnded { end, .. }
                if end.outcome == InterruptionOutcome::Returned =>
            {
                Some(end)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn work_sessions_report_distraction_and_return_without_crediting_interrupted_time() {
    for (start_key, kind) in [(' ', SessionKind::Focus), ('2', SessionKind::QuickStart)] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, start_key);
        assert_eq!(active_id(&h.app), SessionId(1));
        let running = render(&h.app, 100, 30);
        assert!(
            running.contains("d: Report distraction"),
            "{kind:?}: {running}"
        );

        h.at.set(100);
        press(&mut h.app, 'd');
        assert_eq!(interruption(&h.app), pomodoro_core::InterruptionId(1));
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
        assert_eq!(h.app.state().reflection().unwrap().distractions, 1);
        assert_eq!(h.app.state().reflection().unwrap().returns, 0);
        assert_eq!(count_distractions(&h.app), 1);
        assert!(h.app.message().contains("Distraction saved"));
        let distracted = render(&h.app, 100, 30);
        assert!(distracted.contains("Awaiting Return"), "{distracted}");
        assert!(distracted.contains("Space: Return"), "{distracted}");
        assert!(!distracted.contains("d: Report distraction"));

        h.at.set(90_000);
        h.app.tick();
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
        h.at.set(90_100);
        press(&mut h.app, ' ');
        assert_running(&h.app);
        assert_eq!(active_id(&h.app), SessionId(1));
        assert!(h.app.message().contains("Returned to work and saved"));
        assert_eq!(returns(&h.app).len(), 1);
        assert_eq!(
            returns(&h.app)[0].duration,
            MeasuredDuration::Known { elapsed_ms: 90_000 }
        );
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
        assert_eq!(h.app.state().reflection().unwrap().returns, 1);
        h.at.set(90_200);
        h.app.tick();
        assert_eq!(h.app.state().reflection().unwrap().work_ms, 200);
    }
}

#[test]
fn distraction_key_is_ignored_outside_running_work() {
    let mut domains = vec![ready(), awaiting()];
    for kind in [SessionKind::ShortBreak, SessionKind::LongBreak] {
        domains.push(started(kind));
    }
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        for interruption_kind in [
            InterruptionKind::Pause,
            InterruptionKind::Distraction,
            InterruptionKind::AppExit,
            InterruptionKind::ObservationGap,
        ] {
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
            domains.push(domain);
        }
    }
    for domain in domains {
        let mut h = harness(domain, 10);
        let before = h.app.state().clone();
        assert!(!render(&h.app, 100, 30).contains("d: Report distraction"));
        press(&mut h.app, 'd');
        assert_eq!(h.app.state(), &before);
        assert!(h.log.borrow().is_empty());
    }
}

#[test]
fn distraction_input_at_completion_does_not_target_the_next_session() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.log.borrow_mut().clear();
    h.at.set(1_000);
    press(&mut h.app, 'd');
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::ShortBreak,
            ..
        }
    ));
    assert_eq!(count_distractions(&h.app), 0);
    assert_eq!(*h.log.borrow(), ["saved", "notify"]);
}

#[test]
fn restart_keeps_distraction_and_counts_offline_recovery_until_return() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.at.set(100);
    press(&mut h.app, 'd');
    let id = interruption(&h.app);
    h.at.set(200);
    press(&mut h.app, 'q');
    assert!(h.app.should_quit());
    let mut restored = h.app.state().clone();
    restored
        .apply(Command::RestoreApp, Timestamp(90_000))
        .unwrap();
    let mut h = harness(restored, 90_000);
    assert_eq!(interruption(&h.app), id);
    assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
    h.at.set(90_100);
    press(&mut h.app, ' ');
    assert_running(&h.app);
    assert_eq!(returns(&h.app).len(), 1);
    assert_eq!(
        returns(&h.app)[0].duration,
        MeasuredDuration::Known { elapsed_ms: 90_000 }
    );
    assert_eq!(h.app.state().reflection().unwrap().work_ms, 100);
}

#[test]
fn clock_anomaly_keeps_return_but_marks_recovery_unknown() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.at.set(100);
    press(&mut h.app, 'd');
    h.at.set(50);
    h.app.tick();
    assert_eq!(interruption(&h.app), pomodoro_core::InterruptionId(1));
    h.at.set(200);
    press(&mut h.app, ' ');
    assert_running(&h.app);
    assert!(matches!(
        returns(&h.app)[0].duration,
        MeasuredDuration::Unknown { .. }
    ));
    assert_eq!(h.app.state().reflection().unwrap().returns, 1);
}

#[test]
fn failed_report_and_return_retry_the_same_saved_candidate() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.at.set(100);
    h.failures.set(1);
    press(&mut h.app, 'd');
    assert!(h.app.pending_state().is_some());
    assert_running(&h.app);
    assert!(!h.app.message().contains("Distraction saved"));
    assert!(!render(&h.app, 100, 30).contains("Space: Return"));
    let pending = h.app.pending_state().unwrap().clone();
    for key in ['d', ' ', 'n'] {
        press(&mut h.app, key);
    }
    assert_eq!(h.app.pending_state(), Some(&pending));
    h.at.set(150);
    press(&mut h.app, 'r');
    assert_eq!(interruption(&h.app), pomodoro_core::InterruptionId(1));
    assert_eq!(count_distractions(&h.app), 1);
    assert!(h.app.pending_state().is_none());

    h.at.set(200);
    h.failures.set(1);
    press(&mut h.app, ' ');
    assert!(h.app.pending_state().is_some());
    assert_eq!(returns(&h.app).len(), 0);
    assert!(!h.app.message().contains("Returned to work"));
    let pending = h.app.pending_state().unwrap().clone();
    for key in ['d', ' ', 'n'] {
        press(&mut h.app, key);
    }
    assert_eq!(h.app.pending_state(), Some(&pending));
    h.at.set(250);
    press(&mut h.app, 'r');
    assert_eq!(count_distractions(&h.app), 1);
    assert_eq!(returns(&h.app).len(), 1);
    assert_eq!(h.app.state().reflection().unwrap().returns, 1);
    assert!(h.app.pending_state().is_none());
}
