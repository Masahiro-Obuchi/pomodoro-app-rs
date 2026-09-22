use super::*;

fn closing_count(domain: &DomainState) -> usize {
    domain
        .history()
        .events
        .iter()
        .filter(|event| matches!(event.payload, EventKind::AppClosing))
        .count()
}

#[test]
fn shutdown_saves_observed_time_and_app_exit_before_reporting_saved() {
    let mut controller = running();
    controller.clock.push(100);
    controller.tick().unwrap();
    controller.clock.push(200);
    let report = controller.shutdown().unwrap();
    assert_eq!(report.command, Some(Command::CloseApp));
    assert_eq!(elapsed(controller.saved_state()), 200);
    assert_eq!(closing_count(controller.saved_state()), 1);
    assert!(matches!(
        controller.saved_state().snapshot().state,
        ProgressState::Active {
            timer: TimerState::Interrupted { ref interruption }, ..
        } if interruption.kind == InterruptionKind::AppExit
    ));
    assert_valid(controller.saved_state());
    assert!(controller.is_closed());
    assert!(!controller.is_save_pending());
    assert!(matches!(controller.tick(), Err(ControllerError::Closed)));
    assert!(matches!(
        controller.shutdown(),
        Err(ControllerError::Closed)
    ));
    assert!(matches!(
        controller.execute(Command::Resume(SessionId(1))),
        Err(ControllerError::Closed)
    ));
    assert_eq!(controller.exit(), ExitOutcome::Saved);
}

#[test]
fn shutdown_retries_one_candidate_without_observing_or_repeating_close() {
    let mut controller = running();
    controller.store.outcomes = [
        Some(Failure::BeforeCandidate),
        Some(Failure::Pending),
        Some(Failure::Uncertain),
        None,
    ]
    .into();
    controller.clock.push(100);
    assert!(matches!(
        controller.shutdown(),
        Err(ControllerError::Save(_))
    ));
    let fixed = controller.pending_state().unwrap().clone();
    assert!(!controller.is_closed());
    assert_eq!(elapsed(controller.saved_state()), 0);
    assert!(matches!(
        controller.shutdown(),
        Err(ControllerError::SavePending)
    ));
    assert!(controller.retry().is_err());
    assert!(controller.retry().is_err());
    let report = controller.retry().unwrap();
    assert_eq!(report.command, Some(Command::CloseApp));
    assert!(controller.is_closed());
    assert_eq!(controller.clock.samples, 1);
    assert_eq!(controller.store.attempts.len(), 4);
    for (domain, at) in &controller.store.attempts {
        assert_eq!(domain, &fixed);
        assert_eq!(*at, Timestamp(100));
    }
    assert_eq!(closing_count(controller.saved_state()), 1);
    assert_eq!(gap_count(controller.saved_state()), 0);
    assert_eq!(controller.store.generation, 2);
    assert_eq!(controller.exit(), ExitOutcome::Saved);
}

#[test]
fn completion_during_shutdown_notifies_only_after_the_final_save() {
    let mut domain = DomainState::new(TimerConfig::new(1, 1, 1, 2).unwrap()).unwrap();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    let mut controller = controller(domain, 0);
    controller.store.outcomes = [Some(Failure::Uncertain), None].into();
    controller.clock.push(1_000);
    assert!(controller.shutdown().is_err());
    assert_eq!(
        *controller.store.log.borrow(),
        vec![Trace::Save(Timestamp(1_000))]
    );
    let report = controller.retry().unwrap();
    assert_eq!(report.completed, Some(SessionKind::Focus));
    assert!(controller.is_closed());
    assert_eq!(
        *controller.store.log.borrow(),
        vec![
            Trace::Save(Timestamp(1_000)),
            Trace::Retry,
            Trace::Saved,
            Trace::Notify(SessionKind::Focus)
        ]
    );
    assert_eq!(controller.clock.samples, 1);
    assert_eq!(controller.exit(), ExitOutcome::Saved);
}

#[test]
fn shutdown_after_an_observation_gap_still_records_close() {
    let mut controller = running();
    controller.clock.push(10_000);
    controller.shutdown().unwrap();
    assert_eq!(closing_count(controller.saved_state()), 1);
    assert_eq!(gap_count(controller.saved_state()), 1);
    assert_eq!(elapsed(controller.saved_state()), 0);
    assert_eq!(controller.store.attempts.len(), 1);
    assert_valid(controller.saved_state());
}

#[test]
fn shutdown_preserves_distraction_and_quick_start_choice() {
    let mut distraction = ready();
    distraction
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    distraction
        .apply(Command::Distraction(SessionId(1)), Timestamp(0))
        .unwrap();
    let mut awaiting = ready();
    awaiting
        .apply(Command::Start(SessionKind::QuickStart), Timestamp(0))
        .unwrap();
    for at in (1_000..=120_000).step_by(1_000) {
        awaiting
            .observe(Observation {
                previous_at: Timestamp(at - 1_000),
                at: Timestamp(at),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
    }
    for (domain, at) in [(distraction, 0), (awaiting, 120_000)] {
        let snapshot = domain.snapshot().clone();
        let mut controller = controller(domain, at);
        controller.clock.push(at + 100);
        controller.shutdown().unwrap();
        assert_eq!(controller.saved_state().snapshot(), &snapshot);
        assert_eq!(closing_count(controller.saved_state()), 1);
        assert_valid(controller.saved_state());
    }
}

#[test]
fn ready_shutdown_needs_no_redundant_save_and_notification_failure_does_not_undo_exit() {
    let mut controller = controller(ready(), 0);
    controller.clock.push(100);
    controller.execute(Command::CloseApp).unwrap();
    assert!(controller.store.attempts.is_empty());
    assert!(controller.is_closed());
    assert_eq!(controller.exit(), ExitOutcome::Saved);

    let mut controller = running();
    controller.notifier.fail = true;
    for at in [5_000, 10_000] {
        controller.clock.push(at);
        if at == 5_000 {
            controller.tick().unwrap();
        }
    }
    assert!(controller.shutdown().unwrap().notification_error.is_some());
    assert_eq!(controller.exit(), ExitOutcome::Saved);
}

#[test]
fn failed_or_unrequested_shutdown_is_never_a_saved_exit() {
    for failure in [
        Failure::BeforeCandidate,
        Failure::Pending,
        Failure::Uncertain,
    ] {
        let mut controller = running();
        controller.store.outcomes.push_back(Some(failure));
        controller.clock.push(100);
        assert!(controller.shutdown().is_err());
        assert_eq!(controller.exit(), ExitOutcome::Unsaved);
    }
    let mut controller = running();
    controller
        .clock
        .readings
        .push_back(Err(TimeError::BeforeUnixEpoch));
    assert!(matches!(
        controller.shutdown(),
        Err(ControllerError::Time(_))
    ));
    assert_eq!(controller.exit(), ExitOutcome::Unsaved);
    assert_eq!(running().exit(), ExitOutcome::Unsaved);
}
