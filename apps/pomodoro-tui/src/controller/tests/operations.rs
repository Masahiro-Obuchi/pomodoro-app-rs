use super::*;

#[test]
fn ticks_update_live_state_without_saving_every_tick() {
    let mut controller = running();
    for at in [1_000, 2_000, 3_000, 4_000] {
        controller.clock.push(at);
        assert!(controller.tick().unwrap().is_none());
        assert_eq!(elapsed(controller.state()), at);
        assert_eq!(elapsed(controller.saved_state()), 0);
        assert!(controller.store.attempts.is_empty());
    }
    controller.clock.push(5_000);
    let commit = controller.tick().unwrap().unwrap();
    assert!(commit.command.is_none() && commit.completed.is_none());
    assert_eq!(controller.store.attempts.len(), 1);
    assert_eq!(elapsed(controller.saved_state()), 5_000);
    controller.clock.push(5_100);
    assert!(controller.checkpoint().unwrap().is_some());
    assert_eq!(elapsed(controller.saved_state()), 5_100);
    assert_eq!(controller.store.attempts.len(), 2);
}

#[test]
fn unchanged_ready_ticks_do_not_save_even_at_checkpoint_time() {
    let mut controller = controller(ready(), 0);
    controller.clock.push(6_000);
    assert!(controller.tick().unwrap().is_none());
    controller.clock.push(6_100);
    assert!(controller.checkpoint().unwrap().is_none());
    assert!(controller.store.attempts.is_empty());
}

#[test]
fn successful_command_is_reported_only_after_its_save() {
    let mut controller = controller(ready(), 0);
    controller.clock.push(100);
    let command = Command::Start(SessionKind::Focus);
    let commit = controller.execute(command.clone()).unwrap();
    assert_eq!(commit.command, Some(command));
    assert!(commit.completed.is_none());
    assert_eq!(controller.state(), controller.saved_state());
    assert_eq!(
        *controller.store.log.borrow(),
        vec![Trace::Save(Timestamp(100)), Trace::Saved]
    );
}

#[test]
fn completion_saves_and_notifies_before_a_late_input_can_retarget_the_next_session() {
    let mut domain = DomainState::new(TimerConfig::new(1, 1, 1, 1).unwrap()).unwrap();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    let mut controller = controller(domain, 0);
    controller.clock.push(1_000);
    // This would start the newly ready long break if applied after completion.
    let commit = controller
        .execute(Command::Start(SessionKind::LongBreak))
        .unwrap();
    assert!(commit.command.is_none());
    assert_eq!(commit.completed, Some(SessionKind::Focus));
    assert!(matches!(
        controller.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::LongBreak,
            ..
        }
    ));
    assert_eq!(controller.state().history().sessions.len(), 1);
    assert_eq!(
        *controller.store.log.borrow(),
        vec![
            Trace::Save(Timestamp(1_000)),
            Trace::Saved,
            Trace::Notify(SessionKind::Focus)
        ]
    );
}

#[test]
fn notification_failure_never_rolls_back_or_retries_the_saved_completion() {
    let mut domain = DomainState::new(TimerConfig::new(1, 1, 1, 1).unwrap()).unwrap();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    let mut controller = controller(domain, 0);
    controller.notifier.fail = true;
    controller.clock.push(1_000);
    let commit = controller.tick().unwrap().unwrap();
    assert!(commit.notification_error.is_some());
    assert_eq!(
        controller.saved_state().history().sessions[0]
            .end
            .unwrap()
            .outcome,
        SessionOutcome::Completed
    );
    assert!(!controller.is_save_pending());
    assert!(matches!(
        controller.retry(),
        Err(ControllerError::NoPendingSave)
    ));
    controller.clock.push(1_100);
    assert!(controller.tick().unwrap().is_none());
    assert_eq!(controller.store.generation, 2);
    assert_eq!(
        controller
            .store
            .log
            .borrow()
            .iter()
            .filter(|entry| matches!(entry, Trace::Notify(_)))
            .count(),
        1
    );
}

#[test]
fn snapshot_only_clock_uncertainty_is_saved_immediately() {
    let mut domain = ready();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(1_000))
        .unwrap();
    domain
        .apply(Command::Distraction(SessionId(1)), Timestamp(1_000))
        .unwrap();
    // A healthy pair can still predate the stored interruption's boundary.
    let mut controller = controller(domain, 500);
    let events = controller.saved_state().history().events.len();
    controller.clock.push(600);
    assert!(controller.tick().unwrap().is_some());
    assert_eq!(controller.saved_state().history().events.len(), events);
    assert_eq!(elapsed(controller.saved_state()), 0);
    let ProgressState::Active {
        timer: TimerState::Interrupted { interruption },
        ..
    } = &controller.saved_state().snapshot().state
    else {
        panic!("expected distraction");
    };
    assert_eq!(interruption.kind, InterruptionKind::Distraction);
    assert_eq!(
        interruption.time_uncertainty,
        Some(TimeUncertainty::ClockMovedBackward)
    );
    assert_eq!(controller.store.attempts.len(), 1);
    assert_valid(controller.saved_state());
}

#[test]
fn a_rejected_command_keeps_observed_time_but_never_reports_success() {
    let mut controller = running();
    controller.clock.push(100);
    assert!(matches!(
        controller.execute(Command::Pause(SessionId(99))),
        Err(ControllerError::Domain(DomainError::WrongSession))
    ));
    assert_eq!(elapsed(controller.state()), 100);
    assert_eq!(elapsed(controller.saved_state()), 0);
    assert!(controller.store.attempts.is_empty());
    controller.clock.push(200);
    controller.checkpoint().unwrap().unwrap();
    assert_eq!(elapsed(controller.saved_state()), 200);
}

#[test]
fn failed_clock_read_cannot_turn_into_elapsed_credit() {
    let mut controller = running();
    controller
        .clock
        .readings
        .push_back(Err(TimeError::BeforeUnixEpoch));
    assert!(matches!(controller.tick(), Err(ControllerError::Time(_))));
    assert!(controller.store.attempts.is_empty());
    controller.clock.push(100);
    controller.tick().unwrap().unwrap();
    assert_eq!(elapsed(controller.saved_state()), 0);
    assert_eq!(gap_count(controller.saved_state()), 1);
}

#[test]
fn constructor_requires_a_saved_state_and_rejects_unresolved_storage() {
    let Controller {
        mut store,
        clock,
        notifier,
        ..
    } = running();
    store.saved = None;
    assert!(matches!(
        Controller::from_saved(store, clock, notifier),
        Err(ControllerError::MissingSavedState)
    ));
    let Controller {
        mut store,
        clock,
        notifier,
        ..
    } = running();
    store.pending = Some((ready(), Timestamp(0)));
    assert!(matches!(
        Controller::from_saved(store, clock, notifier),
        Err(ControllerError::StorageAlreadyPending)
    ));
}
