#![cfg(any(target_os = "linux", target_os = "macos"))]

mod support;

use std::{fs, os::unix::fs::PermissionsExt};

use pomodoro_core::{
    Command, CurrentTask, DomainState, Observation, ProgressState, SessionId, SessionKind,
    TimerConfig, Timestamp,
};
use pomodoro_platform::{LoadOutcome, SaveError, SaveStage, StorageLocation, StorageLockError};

const BROKEN: &[u8] = b"{ damaged primary, retained verbatim";

fn restore_states() -> Vec<DomainState> {
    let mut ready = DomainState::new(TimerConfig::default()).unwrap();
    ready
        .apply(
            Command::SetCurrentTask(CurrentTask::parse("read chapter 2").unwrap()),
            Timestamp(0),
        )
        .unwrap();
    let mut states = vec![ready.clone()];
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut snapshot = ready.snapshot().clone();
        if !kind.is_work() {
            snapshot.state = ProgressState::Ready {
                next_kind: kind,
                current_task_draft: None,
            };
        }
        let mut domain = DomainState::from_parts(
            snapshot,
            ready.history().clone(),
            ready.id_allocators().clone(),
        )
        .unwrap();
        domain
            .apply(Command::Start(kind), Timestamp(1_000))
            .unwrap();
        states.push(domain);
    }
    let running = states[1].clone();
    for command in [
        Command::Pause(SessionId(1)),
        Command::Distraction(SessionId(1)),
        Command::CloseApp,
    ] {
        let mut domain = running.clone();
        domain.apply(command, Timestamp(1_000)).unwrap();
        states.push(domain);
    }
    let mut gap = running;
    gap.observe(Observation {
        previous_at: Timestamp(1_000),
        at: Timestamp(2_000),
        monotonic_elapsed_ms: None,
    })
    .unwrap();
    states.push(gap);
    let mut awaiting = states[2].clone();
    for second in 1..=120 {
        awaiting
            .observe(Observation {
                previous_at: Timestamp(second * 1_000),
                at: Timestamp((second + 1) * 1_000),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
    }
    assert!(matches!(
        awaiting.snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    states.push(awaiting);
    states
}

#[test]
fn recovery_restores_saved_states_and_returns_to_normal_saves_under_the_same_lock() {
    for domain in restore_states() {
        for original in [None, Some(BROKEN)] {
            let directory = support::tempdir();
            let location = StorageLocation::at(directory.path().to_owned());
            let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap()
            else {
                panic!("new store");
            };
            store.save(&domain, Timestamp(200_000)).unwrap();
            let backup = fs::read(location.state_path()).unwrap();
            store.save(&domain, Timestamp(201_000)).unwrap();
            drop(store);
            if let Some(bytes) = original {
                fs::write(location.state_path(), bytes).unwrap();
            } else {
                fs::remove_file(location.state_path()).unwrap();
            }
            let LoadOutcome::RecoveryRequired(candidate) =
                location.clone().lock().unwrap().load().unwrap()
            else {
                panic!("recovery candidate");
            };
            assert_eq!(candidate.backup().saved_at(), Timestamp(200_000));
            let mut expected = domain.clone();
            expected
                .apply(Command::RestoreApp, Timestamp(300_000))
                .unwrap();
            let mut recovery = candidate.confirm(Timestamp(300_000)).unwrap();
            // Confirmation alone does not modify files or grant normal writes.
            assert_eq!(fs::read(location.state_path()).ok().as_deref(), original);
            let mut store = recovery.save().unwrap();
            assert_eq!(store.saved_state().unwrap().domain(), &expected);
            assert_eq!(store.saved_state().unwrap().save_generation(), 2);
            assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
            assert!(matches!(
                location.clone().lock(),
                Err(StorageLockError::InUse { .. })
            ));
            if let Some(bytes) = original {
                let archive = recovery.quarantine_path().unwrap();
                assert_eq!(fs::read(archive).unwrap(), bytes);
                assert_eq!(
                    fs::metadata(archive).unwrap().permissions().mode() & 0o077,
                    0
                );
            } else {
                assert!(recovery.quarantine_path().is_none());
            }
            let recovered = store.saved_state().unwrap().original_bytes().to_vec();
            store.save(&expected, Timestamp(301_000)).unwrap();
            assert_eq!(store.saved_state().unwrap().save_generation(), 3);
            assert_eq!(fs::read(location.backup_path()).unwrap(), recovered);
            drop(store);
            let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
                panic!("restored primary");
            };
            assert_eq!(store.saved_state().unwrap().domain(), &expected);
        }
    }
}

#[test]
fn cancelling_before_or_after_confirmation_changes_no_saved_files() {
    for confirm in [false, true] {
        let directory = support::tempdir();
        let location = StorageLocation::at(directory.path().to_owned());
        let backup = include_bytes!("fixtures/state_v1.json");
        fs::write(location.state_path(), BROKEN).unwrap();
        fs::write(location.backup_path(), backup).unwrap();
        let LoadOutcome::RecoveryRequired(candidate) =
            location.clone().lock().unwrap().load().unwrap()
        else {
            panic!("recovery candidate");
        };
        if confirm {
            drop(candidate.confirm(Timestamp(2_000)).unwrap());
        } else {
            drop(candidate);
        }
        assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
        assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
        let _lock = location.lock().unwrap();
    }
}

#[test]
fn real_quarantine_creation_failure_preserves_sources_and_can_retry() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    let backup = include_bytes!("fixtures/state_v1.json");
    fs::write(location.state_path(), BROKEN).unwrap();
    fs::write(location.backup_path(), backup).unwrap();
    let LoadOutcome::RecoveryRequired(candidate) = location.clone().lock().unwrap().load().unwrap()
    else {
        panic!("recovery candidate");
    };
    let mut recovery = candidate.confirm(Timestamp(2_000)).unwrap();
    let bytes = recovery.pending_save().unwrap().encoded_bytes().to_vec();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = recovery.save();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if result.is_ok() {
        eprintln!("permission-denied case requires an unprivileged process");
        return;
    }
    assert!(matches!(
        result.unwrap_err(),
        SaveError::Io {
            stage: SaveStage::CreateQuarantine,
            ..
        }
    ));
    assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
    assert_eq!(recovery.pending_save().unwrap().encoded_bytes(), bytes);
    let store = recovery.save().unwrap();
    assert_eq!(store.saved_state().unwrap().original_bytes(), bytes);
    assert_eq!(
        fs::read(recovery.quarantine_path().unwrap()).unwrap(),
        BROKEN
    );
}
