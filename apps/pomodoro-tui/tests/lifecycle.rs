#![cfg(target_os = "linux")]

use std::{collections::VecDeque, fs};

use pomodoro_core::{
    Command, CurrentTask, DomainState, EventKind, InterruptionKind, Observation, ProgressState,
    SessionId, SessionKind, TimerConfig, TimerState, Timestamp,
};
use pomodoro_platform::{
    LoadOutcome, SaveError, StorageLocation, StorageLockError, TimeError, WritableStorage,
};
use pomodoro_tui::controller::{
    Clock, Controller, ControllerError, ExitOutcome, Startup, StartupError, StartupSave,
};

fn settings() -> TimerConfig {
    TimerConfig::new(10, 5, 20, 2).unwrap()
}

fn ready() -> DomainState {
    DomainState::new(settings()).unwrap()
}

fn observe(domain: &mut DomainState, previous: u64, at: u64) {
    domain
        .observe(Observation {
            previous_at: Timestamp(previous),
            at: Timestamp(at),
            monotonic_elapsed_ms: Some(at - previous),
        })
        .unwrap();
}

fn running(kind: SessionKind) -> DomainState {
    let mut domain = ready();
    if !kind.is_work() {
        domain.apply(Command::SkipReady, Timestamp(0)).unwrap();
        if kind == SessionKind::LongBreak {
            // One focus per round makes the next break long without fabricating state.
            domain = DomainState::new(TimerConfig::new(1, 5, 20, 1).unwrap()).unwrap();
            domain
                .apply(Command::Start(SessionKind::Focus), Timestamp(0))
                .unwrap();
            observe(&mut domain, 0, 1_000);
        }
    }
    domain
        .apply(Command::Start(kind), Timestamp(1_000))
        .unwrap();
    observe(&mut domain, 1_000, 1_100);
    domain
}

fn seed(location: &StorageLocation, domain: &DomainState) {
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("new store expected");
    };
    store.save(domain, Timestamp(1_100)).unwrap();
}

fn prepare(location: &StorageLocation, at: u64) -> StartupSave {
    let Startup::Saving(save) = Startup::open(location.clone(), settings(), Timestamp(at)).unwrap()
    else {
        panic!("normal startup expected");
    };
    save
}

fn reload(location: &StorageLocation) -> WritableStorage {
    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("valid saved state expected");
    };
    store
}

fn restored_count(domain: &DomainState) -> usize {
    domain
        .history()
        .events
        .iter()
        .filter(|event| matches!(event.payload, EventKind::AppRestored))
        .count()
}

fn assert_locked(location: &StorageLocation) {
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
}

struct TestClock {
    at: u64,
    times: VecDeque<u64>,
    broken: bool,
}

impl Clock for TestClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        let at = self.times.pop_front().expect("unexpected clock sample");
        let observation = Observation {
            previous_at: Timestamp(self.at),
            at: Timestamp(at),
            monotonic_elapsed_ms: if self.broken {
                None
            } else {
                at.checked_sub(self.at)
            },
        };
        self.at = at;
        self.broken = false;
        Ok(observation)
    }
    fn break_continuity(&mut self) {
        self.broken = true;
    }
}

#[test]
fn new_startup_saves_before_controller_adoption_and_holds_the_lock_through_exit() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let startup = prepare(&location, 100);
    assert_eq!(startup.pending_state(), &ready());
    assert!(!location.state_path().exists());
    assert_locked(&location);
    let store = startup.save().unwrap();
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(100));
    let clock = TestClock {
        at: 100,
        times: [200].into(),
        broken: false,
    };
    let mut controller = Controller::from_saved(store, clock, |_| Ok(())).unwrap();
    controller.shutdown().unwrap();
    assert_locked(&location);
    assert_eq!(controller.exit(), ExitOutcome::Saved);
    let store = reload(&location);
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
}

#[test]
fn every_running_kind_restores_as_gap_without_crediting_downtime() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        let original = running(kind);
        seed(&location, &original);
        let startup = prepare(&location, 1_000_000);
        let store = startup.save().unwrap();
        let domain = store.saved_state().unwrap().domain();
        let ProgressState::Active {
            session,
            timer: TimerState::Interrupted { interruption },
        } = &domain.snapshot().state
        else {
            panic!("restored running session must wait for manual resume");
        };
        assert_eq!(session.kind, kind);
        assert_eq!(session.elapsed_ms, 100);
        assert_eq!(interruption.kind, InterruptionKind::ObservationGap);
        assert_eq!(interruption.started_at, Timestamp(1_100));
        assert_eq!(restored_count(domain), 1);
        let clock = TestClock {
            at: 1_000_000,
            times: [1_000_100].into(),
            broken: false,
        };
        let mut controller =
            Controller::from_saved(store, clock, |_| panic!("restore is not completion")).unwrap();
        assert!(controller.tick().unwrap().is_none());
        assert_eq!(restored_count(controller.saved_state()), 1);
    }
}

#[test]
fn every_interruption_kind_survives_restore_with_the_same_interruption_id() {
    for kind in [
        InterruptionKind::Pause,
        InterruptionKind::Distraction,
        InterruptionKind::AppExit,
        InterruptionKind::ObservationGap,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        let mut original = running(SessionKind::Focus);
        match kind {
            InterruptionKind::Pause => original
                .apply(Command::Pause(SessionId(1)), Timestamp(1_100))
                .unwrap(),
            InterruptionKind::Distraction => original
                .apply(Command::Distraction(SessionId(1)), Timestamp(1_100))
                .unwrap(),
            InterruptionKind::AppExit => {
                original.apply(Command::CloseApp, Timestamp(1_100)).unwrap();
            }
            InterruptionKind::ObservationGap => observe(&mut original, 1_100, 10_000),
        }
        seed(&location, &original);
        let store = prepare(&location, 1_000_000).save().unwrap();
        let restored = store.saved_state().unwrap().domain();
        assert_eq!(restored.snapshot(), original.snapshot());
        assert_eq!(restored_count(restored), 1);
        assert_eq!(
            restored.history().events.len(),
            original.history().events.len() + 1
        );
    }
}

#[test]
fn ready_draft_round_progress_and_quick_start_decision_are_preserved() {
    let mut draft = ready();
    draft
        .apply(
            Command::SetCurrentTask(CurrentTask::parse("read chapter 1").unwrap()),
            Timestamp(0),
        )
        .unwrap();
    let mut round = running(SessionKind::Focus);
    observe(&mut round, 1_100, 6_100);
    observe(&mut round, 6_100, 11_000);
    assert_eq!(
        round.snapshot().round_progress.completed_focuses_in_round,
        1
    );
    let mut awaiting = running(SessionKind::QuickStart);
    for at in (2_100..=121_100).step_by(1_000) {
        observe(&mut awaiting, at - 1_000, at);
    }
    assert!(matches!(
        awaiting.snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    for original in [draft, round, awaiting] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        seed(&location, &original);
        let store = prepare(&location, 1_000_000).save().unwrap();
        let restored = store.saved_state().unwrap().domain();
        assert_eq!(restored.snapshot(), original.snapshot());
        let expected_count = usize::from(matches!(
            original.snapshot().state,
            ProgressState::AwaitingQuickStartDecision { .. }
        ));
        assert_eq!(restored_count(restored), expected_count);
    }
}

#[test]
fn startup_save_failure_retries_one_restore_and_timestamp_with_no_input_permit() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    seed(&location, &running(SessionKind::Focus));
    let original = fs::read(location.state_path()).unwrap();
    let mut startup = prepare(&location, 2_000);
    let fixed = startup.pending_state().clone();
    fs::create_dir(location.backup_path()).unwrap();
    for _ in 0..2 {
        let failure = startup.save().unwrap_err();
        assert_locked(&location);
        startup = failure.startup;
        assert_eq!(startup.pending_state(), &fixed);
        assert_eq!(fs::read(location.state_path()).unwrap(), original);
        assert_locked(&location);
    }
    fs::remove_dir(location.backup_path()).unwrap();
    let store = startup.save().unwrap();
    let saved = store.saved_state().unwrap();
    assert_eq!(saved.domain(), &fixed);
    assert_eq!(restored_count(saved.domain()), 1);
    assert_eq!(saved.save_generation(), 2);
    assert_eq!(saved.saved_at(), Timestamp(2_000));
}

#[test]
fn initial_save_failure_before_native_candidate_creation_can_retry_or_exit_unsaved() {
    for retry in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        let startup = prepare(&location, 100);
        fs::create_dir(location.state_path()).unwrap();
        let failure = startup.save().unwrap_err();
        let startup = failure.startup;
        assert_eq!(startup.pending_state(), &ready());
        assert_locked(&location);
        fs::remove_dir(location.state_path()).unwrap();
        if retry {
            let store = startup.save().unwrap();
            assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(100));
            assert_eq!(store.saved_state().unwrap().save_generation(), 1);
        } else {
            assert_eq!(startup.exit_without_saving(), ExitOutcome::Unsaved);
            assert!(!location.state_path().exists());
            assert!(location.lock().is_ok());
        }
    }
}

fn recovery_files(location: &StorageLocation) -> Vec<u8> {
    seed(location, &running(SessionKind::Focus));
    let backup = fs::read(location.state_path()).unwrap();
    fs::write(location.backup_path(), &backup).unwrap();
    fs::write(location.state_path(), b"broken primary").unwrap();
    backup
}

#[test]
fn recovery_requires_explicit_acceptance_and_declining_preserves_original_files() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let backup = recovery_files(&location);
    let startup = Startup::open(location.clone(), settings(), Timestamp(2_000)).unwrap();
    let Startup::RecoveryRequired(ref candidate) = startup else {
        panic!("recovery offer expected");
    };
    assert_eq!(candidate.backup().saved_at(), Timestamp(1_100));
    assert_eq!(restored_count(candidate.backup().domain()), 0);
    assert_locked(&location);
    assert_eq!(startup.cancel(), ExitOutcome::Cancelled);
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
    assert_eq!(fs::read_dir(location.directory()).unwrap().count(), 3);
    assert!(location.lock().is_ok());
}

#[test]
fn confirmed_recovery_retries_once_then_adopts_without_a_second_restore() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let backup = recovery_files(&location);
    let Startup::RecoveryRequired(candidate) =
        Startup::open(location.clone(), settings(), Timestamp(2_000)).unwrap()
    else {
        panic!("recovery offer expected");
    };
    let mut startup = StartupSave::confirm_recovery(*candidate, Timestamp(3_000)).unwrap();
    let fixed = startup.pending_state().clone();
    assert_eq!(restored_count(&fixed), 1);
    fs::write(location.backup_path(), b"external modification").unwrap();
    for _ in 0..2 {
        let failure = startup.save().unwrap_err();
        assert!(matches!(
            failure.error.failure(),
            SaveError::Conflict { .. }
        ));
        assert_locked(&location);
        startup = failure.startup;
        assert_eq!(startup.pending_state(), &fixed);
        assert_locked(&location);
        assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    }
    fs::write(location.backup_path(), &backup).unwrap();
    let store = startup.save().unwrap();
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(3_000));
    assert_eq!(store.saved_state().unwrap().save_generation(), 2);
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
    let archived = fs::read_dir(location.directory())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| {
            entry.file_name().to_string_lossy().contains("quarantine")
                && fs::read(entry.path()).unwrap() == b"broken primary"
        });
    assert!(archived);
    let clock = TestClock {
        at: 3_000,
        times: [3_100].into(),
        broken: false,
    };
    let mut controller = Controller::from_saved(store, clock, |_| {
        panic!("recovery must not notify completion")
    })
    .unwrap();
    assert!(controller.tick().unwrap().is_none());
    assert_eq!(controller.saved_state(), &fixed);
    assert_eq!(restored_count(controller.state()), 1);
}

#[test]
fn failed_recovery_returns_ownership_for_an_explicit_unsaved_exit() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let backup = recovery_files(&location);
    let Startup::RecoveryRequired(candidate) =
        Startup::open(location.clone(), settings(), Timestamp(2_000)).unwrap()
    else {
        panic!("recovery offer expected");
    };
    let startup = StartupSave::confirm_recovery(*candidate, Timestamp(3_000)).unwrap();
    let fixed = startup.pending_state().clone();
    fs::write(location.backup_path(), b"external modification").unwrap();
    let failure = startup.save().unwrap_err();
    assert!(matches!(
        failure.error.failure(),
        SaveError::Conflict { .. }
    ));
    assert_eq!(failure.startup.pending_state(), &fixed);
    assert_locked(&location);
    assert_eq!(
        Startup::Saving(failure.startup).cancel(),
        ExitOutcome::Unsaved
    );
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    assert_eq!(
        fs::read(location.backup_path()).unwrap(),
        b"external modification"
    );
    let lock = location.clone().lock().unwrap();
    fs::write(location.backup_path(), backup).unwrap();
    drop(lock);
}

#[test]
fn invalid_or_unsupported_load_never_initializes_or_overwrites_data() {
    for bytes in [
        b"broken primary".as_slice(),
        br#"{"old_state":true}"#,
        br#"{"schema_version":999}"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        fs::write(location.state_path(), bytes).unwrap();
        assert!(matches!(
            Startup::open(location.clone(), settings(), Timestamp(100)),
            Err(StartupError::Load(_))
        ));
        assert_eq!(fs::read(location.state_path()).unwrap(), bytes);
        assert!(!location.backup_path().exists());
        assert!(location.lock().is_ok());
    }
}

#[test]
fn real_shutdown_failure_can_retry_or_exit_unsaved_and_releases_lock_only_on_exit() {
    for retry in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(dir.path().to_owned());
        seed(&location, &running(SessionKind::Focus));
        // from_saved here isolates final-save behavior from startup's restore.
        let clock = TestClock {
            at: 1_100,
            times: [1_200].into(),
            broken: false,
        };
        let mut controller = Controller::from_saved(reload(&location), clock, |_| Ok(())).unwrap();
        let original = fs::read(location.state_path()).unwrap();
        fs::create_dir(location.backup_path()).unwrap();
        assert!(matches!(
            controller.shutdown(),
            Err(ControllerError::Save(_))
        ));
        assert_locked(&location);
        assert_eq!(fs::read(location.state_path()).unwrap(), original);
        fs::remove_dir(location.backup_path()).unwrap();
        if retry {
            controller.retry().unwrap();
            assert!(controller.is_closed());
            assert_locked(&location);
            assert_eq!(controller.exit(), ExitOutcome::Saved);
            let store = reload(&location);
            assert_eq!(store.saved_state().unwrap().save_generation(), 2);
            assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(1_200));
            assert!(
                matches!(store.saved_state().unwrap().domain().snapshot().state,
                ProgressState::Active { timer: TimerState::Interrupted { ref interruption }, .. }
                if interruption.kind == InterruptionKind::AppExit)
            );
        } else {
            assert_eq!(controller.exit(), ExitOutcome::Unsaved);
            assert_eq!(fs::read(location.state_path()).unwrap(), original);
            assert!(location.lock().is_ok());
        }
    }
}

#[test]
fn distraction_survives_saved_shutdown_reopen_and_explicit_return() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    seed(&location, &running(SessionKind::Focus));
    let clock = TestClock {
        at: 1_100,
        times: [1_200, 1_300].into(),
        broken: false,
    };
    let mut controller = Controller::from_saved(reload(&location), clock, |_| Ok(())).unwrap();
    controller
        .execute(Command::Distraction(SessionId(1)))
        .unwrap();
    controller.shutdown().unwrap();
    assert_eq!(controller.exit(), ExitOutcome::Saved);

    let store = prepare(&location, 1_000_000).save().unwrap();
    let ProgressState::Active {
        session,
        timer: TimerState::Interrupted { interruption },
    } = &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("restored distraction expected");
    };
    assert_eq!(session.elapsed_ms, 200);
    assert_eq!(interruption.kind, InterruptionKind::Distraction);
    assert_eq!(interruption.started_at, Timestamp(1_200));
    let clock = TestClock {
        at: 1_000_000,
        times: [1_000_100, 1_000_200].into(),
        broken: false,
    };
    let mut controller = Controller::from_saved(store, clock, |_| Ok(())).unwrap();
    controller.execute(Command::Return(SessionId(1))).unwrap();
    assert_eq!(restored_count(controller.saved_state()), 1);
    let end = controller
        .saved_state()
        .history()
        .events
        .iter()
        .find_map(|event| {
            if let EventKind::InterruptionEnded { end, .. } = &event.payload {
                Some(end)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(end.outcome, pomodoro_core::InterruptionOutcome::Returned);
    assert_eq!(
        end.duration,
        pomodoro_core::MeasuredDuration::Known {
            elapsed_ms: 998_900
        }
    );
    assert!(controller.tick().unwrap().is_none());
    let ProgressState::Active {
        session,
        timer: TimerState::Running { .. },
    } = &controller.state().snapshot().state
    else {
        panic!("explicit Return resumes the session");
    };
    assert_eq!(session.elapsed_ms, 300);
}
