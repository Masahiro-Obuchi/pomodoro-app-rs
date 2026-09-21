use super::*;

fn awaiting_quick_start() -> DomainState {
    let mut domain = ready();
    domain
        .apply(Command::Start(SessionKind::QuickStart), Timestamp(0))
        .unwrap();
    for at in (1_000..=120_000).step_by(1_000) {
        domain
            .observe(Observation {
                previous_at: Timestamp(at - 1_000),
                at: Timestamp(at),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
    }
    domain
}

#[test]
fn quick_start_retry_fixes_ids_and_timestamp_and_saves_gap_before_unlocking() {
    let initial = awaiting_quick_start();
    let mut controller = controller(initial.clone(), 120_000);
    controller.store.outcomes.extend([
        Some(Failure::Pending),
        Some(Failure::Uncertain),
        None,
        Some(Failure::Pending),
        Some(Failure::Uncertain),
        None,
    ]);
    controller.clock.push(120_100);
    let command = Command::DecideQuickStart {
        session_id: SessionId(1),
        choice: QuickStartChoice::Continue,
    };
    assert!(matches!(
        controller.execute(command.clone()),
        Err(ControllerError::Save(_))
    ));
    let candidate = controller.pending_state().unwrap().clone();
    assert_eq!(controller.state(), &initial);
    assert_eq!(controller.store.generation, 1);
    let samples = controller.clock.samples;
    for action in [
        Command::Start(SessionKind::Focus),
        Command::Pause(SessionId(2)),
    ] {
        assert!(matches!(
            controller.execute(action),
            Err(ControllerError::SavePending)
        ));
    }
    assert!(matches!(
        controller.tick(),
        Err(ControllerError::SavePending)
    ));
    assert!(matches!(
        controller.checkpoint(),
        Err(ControllerError::SavePending)
    ));
    assert_eq!(controller.clock.samples, samples);
    let Err(ControllerError::Save(error)) = controller.retry() else {
        panic!("expected uncertain commit");
    };
    assert!(error.is_commit_uncertain());
    assert_eq!(controller.pending_state(), Some(&candidate));
    // First save succeeds, but the recovery-gap save fails.
    controller.clock.push(120_150);
    assert!(matches!(controller.retry(), Err(ControllerError::Save(_))));
    assert_eq!(controller.saved_state(), &candidate);
    assert!(controller.is_save_pending());
    let gap_candidate = controller.pending_state().unwrap().clone();
    assert_eq!(elapsed(&gap_candidate), 0);
    assert_eq!(gap_count(&gap_candidate), 1);
    assert!(
        !gap_candidate
            .history()
            .events
            .iter()
            .any(|event| matches!(event.payload, EventKind::AppRestored))
    );
    assert_eq!(
        gap_candidate
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::QuickStartDecisionMade { .. }))
            .count(),
        1
    );
    assert_valid(&gap_candidate);
    let samples = controller.clock.samples;
    let Err(ControllerError::Save(error)) = controller.retry() else {
        panic!("expected uncertain gap save");
    };
    assert!(error.is_commit_uncertain());
    assert_eq!(controller.pending_state(), Some(&gap_candidate));
    assert_eq!(controller.saved_state(), &candidate);
    assert!(controller.is_save_pending());
    let commit = controller.retry().unwrap();
    assert_eq!(commit.command, Some(command));
    assert!(!controller.is_save_pending());
    assert_eq!(controller.clock.samples, samples);
    assert_eq!(controller.saved_state(), &gap_candidate);
    assert_eq!(controller.store.generation, 3);
    let attempts = &controller.store.attempts;
    for attempt in &attempts[..3] {
        assert_eq!(attempt, &(candidate.clone(), Timestamp(120_100)));
    }
    for attempt in &attempts[3..] {
        assert_eq!(attempt, &(gap_candidate.clone(), Timestamp(120_150)));
    }
}

#[test]
fn completion_notifications_wait_for_successful_retry_and_are_not_repeated() {
    let mut domain = DomainState::new(TimerConfig::new(1, 1, 1, 1).unwrap()).unwrap();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    let mut controller = controller(domain, 0);
    controller
        .store
        .outcomes
        .extend([Some(Failure::Uncertain), Some(Failure::Pending), None]);
    controller.clock.push(1_000);
    assert!(matches!(controller.tick(), Err(ControllerError::Save(_))));
    assert!(matches!(controller.retry(), Err(ControllerError::Save(_))));
    assert!(
        !controller
            .store
            .log
            .borrow()
            .iter()
            .any(|entry| matches!(entry, Trace::Notify(_)))
    );
    controller.clock.push(1_050);
    let commit = controller.retry().unwrap();
    assert_eq!(commit.completed, Some(SessionKind::Focus));
    assert_eq!(
        controller.store.log.borrow().last(),
        Some(&Trace::Notify(SessionKind::Focus))
    );
    let log = controller.store.log.borrow();
    let notified = log
        .iter()
        .position(|entry| matches!(entry, Trace::Notify(_)))
        .unwrap();
    assert_eq!(log[notified - 1], Trace::Saved);
    drop(log);
    assert_eq!(controller.store.generation, 2);
    assert_eq!(controller.saved_state().history().sessions.len(), 1);
    assert!(matches!(
        controller.retry(),
        Err(ControllerError::NoPendingSave)
    ));
}

#[test]
fn checkpoint_failure_freezes_ticks_and_does_not_credit_short_save_wait() {
    let mut controller = running();
    controller.clock.push(100);
    controller.tick().unwrap();
    controller.store.outcomes.push_back(Some(Failure::Pending));
    controller.clock.push(200);
    assert!(matches!(
        controller.checkpoint(),
        Err(ControllerError::Save(_))
    ));
    assert_eq!(elapsed(controller.saved_state()), 0);
    assert_eq!(elapsed(controller.pending_state().unwrap()), 200);
    assert!(matches!(
        controller.tick(),
        Err(ControllerError::SavePending)
    ));
    controller.clock.push(250);
    let commit = controller.retry().unwrap();
    assert!(commit.command.is_none() && commit.completed.is_none());
    assert_eq!(elapsed(controller.saved_state()), 200);
    let ProgressState::Active {
        timer: TimerState::Interrupted { interruption },
        ..
    } = &controller.saved_state().snapshot().state
    else {
        panic!("expected gap");
    };
    assert_eq!(interruption.kind, InterruptionKind::ObservationGap);
    assert_eq!(interruption.started_at, Timestamp(200));
    assert_eq!(interruption.recorded_at, Timestamp(250));
    assert_eq!(
        interruption.time_uncertainty,
        Some(TimeUncertainty::InsufficientClockEvidence)
    );
}

#[test]
fn pre_candidate_failure_retries_the_same_domain_and_saved_at() {
    let mut controller = controller(ready(), 0);
    controller.store.outcomes.extend([
        Some(Failure::BeforeCandidate),
        Some(Failure::BeforeCandidate),
        None,
        None,
    ]);
    controller.clock.push(100);
    assert!(matches!(
        controller.execute(Command::Start(SessionKind::Focus)),
        Err(ControllerError::Save(_))
    ));
    let fixed = controller.pending_state().unwrap().clone();
    assert!(!controller.store.has_pending_save());
    assert!(matches!(controller.retry(), Err(ControllerError::Save(_))));
    controller.clock.push(150);
    controller.retry().unwrap();
    for attempt in &controller.store.attempts[..3] {
        assert_eq!(attempt, &(fixed.clone(), Timestamp(100)));
    }
    assert_eq!(gap_count(controller.saved_state()), 1);
    assert_valid(controller.saved_state());
}

#[test]
fn clock_failure_after_first_save_keeps_input_blocked_without_resaving_it() {
    let mut controller = controller(ready(), 0);
    controller.store.outcomes.push_back(Some(Failure::Pending));
    controller.clock.push(100);
    assert!(
        controller
            .execute(Command::Start(SessionKind::Focus))
            .is_err()
    );
    controller
        .clock
        .readings
        .push_back(Err(TimeError::BeforeUnixEpoch));
    assert!(matches!(controller.retry(), Err(ControllerError::Time(_))));
    assert!(controller.is_save_pending());
    assert_eq!(controller.store.generation, 2);
    assert!(matches!(
        controller.execute(Command::Pause(SessionId(1))),
        Err(ControllerError::SavePending)
    ));
    controller.clock.push(150);
    controller.retry().unwrap();
    assert_eq!(controller.store.generation, 3);
    assert_eq!(controller.store.attempts.len(), 3);
    assert_eq!(gap_count(controller.saved_state()), 1);
}

#[test]
fn interrupted_and_awaiting_states_are_not_automatically_resumed_on_recovery() {
    let mut distraction = ready();
    distraction
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    for (domain, command, at) in [
        (distraction, Command::Distraction(SessionId(1)), 100),
        (awaiting_quick_start(), Command::CloseApp, 120_100),
    ] {
        let mut controller = controller(domain, at - 100);
        controller.store.outcomes.push_back(Some(Failure::Pending));
        controller.clock.push(at);
        assert!(controller.execute(command).is_err());
        controller.clock.push(at + 50);
        controller.retry().unwrap();
        match &controller.saved_state().snapshot().state {
            ProgressState::Active {
                timer: TimerState::Interrupted { interruption },
                ..
            } => {
                assert_eq!(interruption.kind, InterruptionKind::Distraction);
                assert!(interruption.end.is_none());
            }
            ProgressState::AwaitingQuickStartDecision { .. } => {
                assert_eq!(gap_count(controller.saved_state()), 0);
            }
            _ => panic!("recovery must preserve the interrupted or awaiting state"),
        }
        assert_valid(controller.saved_state());
    }
}
