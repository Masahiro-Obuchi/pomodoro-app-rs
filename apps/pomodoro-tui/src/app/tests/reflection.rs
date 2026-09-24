use super::*;

fn summary(app: &TestApp) -> ReflectionSummary {
    let summary = app.reflection().unwrap();
    assert_eq!(summary, app.state().reflection().unwrap());
    summary
}

#[test]
fn ready_reset_reports_no_change_and_active_reset_waits_for_saved_success() {
    let mut h = harness(ready(), 0);
    let initial = h.app.state().clone();
    press(&mut h.app, 'r');
    assert_eq!(h.app.state(), &initial);
    assert!(h.log.borrow().is_empty());
    assert!(h.app.message().contains("nothing changed"));
    assert!(!h.app.message().contains("Reset to ready"));
    press(&mut h.app, ' ');
    h.at.set(100);
    h.failures.set(1);
    press(&mut h.app, 'r');
    assert!(h.app.pending_state().is_some());
    assert!(!h.app.message().contains("Reset to ready"));
    press(&mut h.app, 'r');
    assert!(h.app.message().contains("Reset to ready"));
    assert_eq!(h.app.state().history().sessions.len(), 1);
    assert_eq!(summary(&h.app).work_ms, 100);
    press(&mut h.app, 'r');
    assert!(h.app.message().contains("nothing changed"));
    assert_eq!(h.app.state().history().sessions.len(), 1);
}

#[test]
fn repeated_frames_and_live_ticks_reuse_history_but_completion_refreshes_it() {
    let mut h = harness(ready(), 0);
    for _ in 0..10 {
        render(&h.app, 80, 24);
    }
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 1);
    press(&mut h.app, ' ');
    for at in (100..1_000).step_by(100) {
        h.at.set(at);
        h.app.tick();
        render(&h.app, 80, 24);
        assert_eq!(summary(&h.app).work_ms, at);
        assert_eq!(summary(&h.app).completed_focus_sessions, 0);
        assert_eq!(h.app.history_reflection.borrow().rebuilds, 1);
    }
    h.at.set(1_000);
    h.app.tick();
    let completed = summary(&h.app);
    assert_eq!(completed.work_ms, 1_000);
    assert_eq!(completed.completed_focus_sessions, 1);
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 2);
    press(&mut h.app, ' '); // Break time does not add work.
    h.at.set(1_500);
    h.app.tick();
    assert_eq!(summary(&h.app), completed);
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 2);
}

#[test]
fn checkpoint_failure_uses_durable_elapsed_and_retry_never_double_counts_work() {
    let mut h = harness(
        DomainState::new(TimerConfig::new(10, 1, 1, 2).unwrap()).unwrap(),
        0,
    );
    press(&mut h.app, ' ');
    h.at.set(1_000);
    h.app.tick();
    assert_eq!(summary(&h.app).work_ms, 1_000);
    h.failures.set(1);
    h.at.set(5_000);
    h.app.tick();
    assert!(h.app.pending_state().is_some());
    assert_eq!(summary(&h.app).work_ms, 0);
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 1);
    h.fail_after.set(Some(1));
    h.at.set(5_100);
    press(&mut h.app, 'r'); // Checkpoint is now saved, recovery gap is still pending.
    assert!(h.app.pending_state().is_some());
    assert_eq!(summary(&h.app).work_ms, 5_000);
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 1);
    press(&mut h.app, 'r');
    assert!(h.app.pending_state().is_none());
    assert_eq!(summary(&h.app).work_ms, 5_000);
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 2);
    h.at.set(5_200);
    // The failed gap save also broke clock continuity. Let the controller
    // persist that observation before asking to resume the interruption.
    h.app.tick();
    press(&mut h.app, ' ');
    assert_running(&h.app);
    h.at.set(5_300);
    h.app.tick();
    assert_eq!(summary(&h.app).work_ms, 5_100);
}

#[test]
fn reflection_shows_distractions_and_returns_including_the_active_session() {
    let mut domain = started(SessionKind::QuickStart);
    domain
        .apply(Command::Distraction(SessionId(1)), Timestamp(0))
        .unwrap();
    domain
        .apply(Command::Return(SessionId(1)), Timestamp(1_000))
        .unwrap();
    domain
        .observe(Observation {
            previous_at: Timestamp(1_000),
            at: Timestamp(1_100),
            monotonic_elapsed_ms: Some(100),
        })
        .unwrap();
    domain
        .apply(Command::Pause(SessionId(1)), Timestamp(1_100))
        .unwrap();
    domain
        .apply(Command::Resume(SessionId(1)), Timestamp(1_200))
        .unwrap();
    domain
        .observe(Observation {
            previous_at: Timestamp(1_200),
            at: Timestamp(1_300),
            monotonic_elapsed_ms: None,
        })
        .unwrap();
    domain
        .apply(Command::Resume(SessionId(1)), Timestamp(1_400))
        .unwrap();
    domain
        .apply(Command::Distraction(SessionId(1)), Timestamp(1_400))
        .unwrap();
    domain.apply(Command::CloseApp, Timestamp(1_400)).unwrap();
    domain.apply(Command::RestoreApp, Timestamp(1_500)).unwrap();
    let mut h = harness(domain, 1_500);
    assert_eq!(
        summary(&h.app),
        ReflectionSummary {
            work_ms: 100,
            completed_focus_sessions: 0,
            distractions: 2,
            returns: 1
        }
    );
    let display = render(&h.app, 80, 24);
    for label in [
        "Total:",
        "Focus completed 0",
        "Work 0:00:00",
        "Distractions 2",
        "Returns 1",
        "Round",
        "Return",
    ] {
        assert!(display.contains(label), "missing {label}: {display}");
    }
    h.failures.set(1);
    h.at.set(1_600);
    press(&mut h.app, ' ');
    assert_eq!(summary(&h.app).returns, 1);
    let pending = render(&h.app, 80, 24);
    assert!(pending.contains("Returns 1"));
    assert!(pending.contains("r: Retry save"));
    assert!(pending.contains("Q: Confirm unsaved exit"));
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 1);
    h.at.set(1_700);
    press(&mut h.app, 'r');
    assert_eq!(
        summary(&h.app),
        ReflectionSummary {
            work_ms: 100,
            completed_focus_sessions: 0,
            distractions: 2,
            returns: 2
        }
    );
    assert_eq!(h.app.history_reflection.borrow().rebuilds, 2);
    assert!(render(&h.app, 80, 24).contains("Returns 2"));
}
