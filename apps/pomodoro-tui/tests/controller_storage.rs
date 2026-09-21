#![cfg(target_os = "linux")]

use std::{cell::Cell, collections::VecDeque, fs, rc::Rc};

use pomodoro_core::{
    Command, DomainState, EventKind, InterruptionKind, Observation, ProgressState, SessionId,
    SessionKind, TimerConfig, TimerState, Timestamp,
};
use pomodoro_platform::{
    LoadOutcome, SaveError, SaveStage, StorageLocation, StorageLockError, TimeError,
    WritableStorage,
};
use pomodoro_tui::controller::{Clock, Controller, ControllerError};

struct ScriptedClock {
    times: VecDeque<u64>,
    at: u64,
    broken: bool,
}

impl Clock for ScriptedClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        let at = self.times.pop_front().unwrap();
        let observation = Observation {
            previous_at: Timestamp(self.at),
            at: Timestamp(at),
            monotonic_elapsed_ms: if self.broken {
                None
            } else {
                at.checked_sub(self.at)
            },
        };
        self.broken = false;
        self.at = at;
        Ok(observation)
    }

    fn break_continuity(&mut self) {
        self.broken = true;
    }
}

fn bootstrap(location: &StorageLocation) -> WritableStorage {
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("new store expected");
    };
    store
        .save(
            &DomainState::new(TimerConfig::new(1, 1, 1, 1).unwrap()).unwrap(),
            Timestamp(0),
        )
        .unwrap();
    store
}

#[test]
fn real_save_failure_retries_the_operation_then_saves_gap_and_preserves_lock() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let store = bootstrap(&location);
    let original = fs::read(location.state_path()).unwrap();
    let clock = ScriptedClock {
        times: [100, 150].into(),
        at: 0,
        broken: false,
    };
    let mut controller = Controller::from_saved(store, clock, |_| Ok(())).unwrap();
    // A directory at the backup target forces a genuine rename failure after
    // WritableStorage has fixed its JSON, generation and timestamp.
    fs::create_dir(location.backup_path()).unwrap();
    let Err(ControllerError::Save(error)) = controller.execute(Command::Start(SessionKind::Focus))
    else {
        panic!("expected save failure");
    };
    assert!(matches!(
        error.failure(),
        SaveError::Io {
            stage: SaveStage::RenameBackup,
            ..
        }
    ));
    let fixed = controller.pending_state().unwrap().clone();
    assert_eq!(fs::read(location.state_path()).unwrap(), original);
    assert!(matches!(
        controller.execute(Command::Pause(SessionId(1))),
        Err(ControllerError::SavePending)
    ));
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    assert!(controller.retry().is_err());
    assert_eq!(controller.pending_state(), Some(&fixed));
    fs::remove_dir(location.backup_path()).unwrap();
    let report = controller.retry().unwrap();
    assert_eq!(report.command, Some(Command::Start(SessionKind::Focus)));
    let expected = controller.saved_state().clone();
    let ProgressState::Active {
        session,
        timer: TimerState::Interrupted { interruption },
    } = &expected.snapshot().state
    else {
        panic!("expected persisted gap");
    };
    assert_eq!(session.elapsed_ms, 0);
    assert_eq!(interruption.kind, InterruptionKind::ObservationGap);
    assert_eq!(interruption.started_at, Timestamp(100));
    assert_eq!(interruption.recorded_at, Timestamp(150));
    assert_eq!(
        expected
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::ObservationGapDetected { .. }))
            .count(),
        1
    );
    drop(controller);
    let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
        panic!("saved store expected");
    };
    assert_eq!(store.saved_state().unwrap().domain(), &expected);
    assert_eq!(store.saved_state().unwrap().save_generation(), 3);
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(150));
}

#[test]
fn native_completion_is_present_on_disk_before_the_notification_callback() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let store = bootstrap(&location);
    let clock = ScriptedClock {
        times: [100, 1_100].into(),
        at: 0,
        broken: false,
    };
    let notifications = Rc::new(Cell::new(0));
    let notified = notifications.clone();
    let mut controller = Controller::from_saved(store, clock, move |kind| {
        assert_eq!(kind, SessionKind::Focus);
        notified.set(notified.get() + 1);
        // The callback cannot acquire the controller's lock. Copy the committed
        // bytes into an independent store to verify them through the public codec.
        let copy = tempfile::tempdir().unwrap();
        let copy_location = StorageLocation::at(copy.path().to_owned());
        fs::write(
            copy_location.state_path(),
            fs::read(location.state_path()).unwrap(),
        )
        .unwrap();
        let LoadOutcome::Loaded(store) = copy_location.lock().unwrap().load().unwrap() else {
            panic!("valid saved completion expected");
        };
        assert_eq!(store.saved_state().unwrap().save_generation(), 3);
        assert_eq!(
            store
                .saved_state()
                .unwrap()
                .domain()
                .history()
                .sessions
                .len(),
            1
        );
        Ok(())
    })
    .unwrap();
    controller
        .execute(Command::Start(SessionKind::Focus))
        .unwrap();
    assert_eq!(notifications.get(), 0);
    let report = controller.tick().unwrap().unwrap();
    assert_eq!(report.completed, Some(SessionKind::Focus));
    assert_eq!(notifications.get(), 1);
}
