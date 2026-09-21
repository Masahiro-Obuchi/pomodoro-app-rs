use super::*;
use crate::{LoadOutcome, StorageLockError};
use pomodoro_core::{DomainState, EventKind, SessionKind, TimerConfig};
use std::fs;

#[path = "recovery_race_tests.rs"]
mod races;

const BROKEN: &[u8] = b"{ broken primary with information to preserve";
const RESTORED_AT: Timestamp = Timestamp(5_000);
const STAGES: &[SaveStage] = &[
    SaveStage::VerifyRecoverySources,
    SaveStage::SyncStorageAncestry,
    SaveStage::CreateQuarantine,
    SaveStage::WriteQuarantine,
    SaveStage::SyncQuarantine,
    SaveStage::SyncQuarantineDirectory,
    SaveStage::CreatePrimaryTemp,
    SaveStage::WritePrimaryTemp,
    SaveStage::SyncPrimaryTemp,
    SaveStage::RenamePrimary,
    SaveStage::SyncPrimaryDirectory,
];

fn backup() -> PersistedStateV1 {
    let mut domain = DomainState::new(TimerConfig::default()).unwrap();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(1_000))
        .unwrap();
    PersistedStateV1 {
        domain,
        save_generation: 7,
        saved_at: Timestamp(1_000),
    }
}

fn prepare(location: &StorageLocation, original: Option<&[u8]>) -> RecoveryCandidate {
    if let Some(bytes) = original {
        fs::write(location.state_path(), bytes).unwrap();
    }
    fs::write(location.backup_path(), backup().encode().unwrap()).unwrap();
    let LoadOutcome::RecoveryRequired(candidate) = location.clone().lock().unwrap().load().unwrap()
    else {
        panic!("expected recovery");
    };
    candidate
}

fn fail(stage: SaveStage) -> impl FnMut(SaveStage, &Path) -> io::Result<()> {
    move |current, _| {
        if current == stage {
            Err(io::Error::other("injected recovery failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn every_failure_keeps_one_restored_candidate_and_protects_backup() {
    for &stage in STAGES {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, Some(BROKEN))
            .confirm(RESTORED_AT)
            .unwrap();
        let encoded = recovery.pending_save().unwrap().encoded_bytes().to_vec();
        let mut expected = backup().domain;
        expected.apply(Command::RestoreApp, RESTORED_AT).unwrap();
        assert_eq!(recovery.pending_save().unwrap().domain(), &expected);
        for _ in 0..2 {
            let error = recovery.save_with_hook(&mut fail(stage)).unwrap_err();
            assert!(
                matches!(error.failure(), SaveError::Io { stage: actual, .. } if *actual == stage)
            );
            assert_eq!(
                error.is_commit_uncertain(),
                stage == SaveStage::SyncPrimaryDirectory
            );
            let pending = recovery.pending_save().unwrap();
            assert_eq!(pending.encoded_bytes(), encoded);
            assert_eq!(pending.domain(), &expected);
            assert_eq!(pending.save_generation(), 8);
            assert_eq!(pending.saved_at(), RESTORED_AT);
            assert_eq!(
                fs::read(location.backup_path()).unwrap(),
                backup().encode().unwrap()
            );
            assert_eq!(
                fs::read(location.state_path()).unwrap(),
                if stage == SaveStage::SyncPrimaryDirectory {
                    encoded.as_slice()
                } else {
                    BROKEN
                }
            );
            assert!(matches!(
                location.clone().lock(),
                Err(StorageLockError::InUse { .. })
            ));
        }
        let store = recovery.save().unwrap();
        assert_eq!(store.saved_state().unwrap().domain(), &expected);
        assert_eq!(
            fs::read(recovery.quarantine_path().unwrap()).unwrap(),
            BROKEN
        );
        assert_eq!(fs::read(location.state_path()).unwrap(), encoded);
        assert_eq!(
            fs::read(location.backup_path()).unwrap(),
            backup().encode().unwrap()
        );
        assert!(recovery.pending_save().is_none());
        assert!(matches!(recovery.save(), Err(SaveError::NoPendingSave)));
    }
}

#[test]
fn missing_primary_retries_without_creating_an_archive() {
    for stage in [SaveStage::RenamePrimary, SaveStage::SyncPrimaryDirectory] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, None).confirm(RESTORED_AT).unwrap();
        let bytes = recovery.pending_save().unwrap().encoded_bytes().to_vec();
        recovery.save_with_hook(&mut fail(stage)).unwrap_err();
        let store = recovery.save().unwrap();
        assert_eq!(store.saved_state().unwrap().original_bytes(), bytes);
        assert!(recovery.quarantine_path().is_none());
        assert_eq!(
            fs::read(location.backup_path()).unwrap(),
            backup().encode().unwrap()
        );
    }
}

#[test]
fn third_primary_or_changed_backup_stops_before_writes() {
    for modify_backup in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, Some(BROKEN))
            .confirm(RESTORED_AT)
            .unwrap();
        let path = if modify_backup {
            location.backup_path()
        } else {
            location.state_path()
        };
        fs::write(&path, b"externally changed").unwrap();
        let error = recovery.save().unwrap_err();
        assert!(matches!(error, SaveError::Conflict { path: actual } if actual == path));
        assert!(recovery.quarantine_path().is_none());
        assert_eq!(fs::read(path).unwrap(), b"externally changed");
    }
}

#[test]
fn changes_after_quarantine_are_rechecked_before_replacement() {
    for modify_backup in [false, true] {
        for missing_primary in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let location = StorageLocation::at(directory.path().to_owned());
            let mut recovery = prepare(&location, (!missing_primary).then_some(BROKEN))
                .confirm(RESTORED_AT)
                .unwrap();
            let path = if modify_backup {
                location.backup_path()
            } else {
                location.state_path()
            };
            let mut checks = 0;
            let error = recovery
                .save_with_hook(&mut |stage, _| {
                    if stage == SaveStage::VerifyRecoverySources {
                        checks += 1;
                        if checks == 2 {
                            fs::write(&path, b"changed after archive")?;
                        }
                    }
                    Ok(())
                })
                .unwrap_err();
            assert!(matches!(error, SaveError::Conflict { path: actual } if actual == path));
            assert_eq!(fs::read(path).unwrap(), b"changed after archive");
            if !missing_primary {
                assert_eq!(
                    fs::read(recovery.quarantine_path().unwrap()).unwrap(),
                    BROKEN
                );
            }
        }
    }
}

#[test]
fn unreadable_primary_cannot_be_replaced_even_after_archive() {
    for archived in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, Some(BROKEN))
            .confirm(RESTORED_AT)
            .unwrap();
        if archived {
            recovery
                .save_with_hook(&mut fail(SaveStage::RenamePrimary))
                .unwrap_err();
        }
        fs::remove_file(location.state_path()).unwrap();
        fs::create_dir(location.state_path()).unwrap();
        assert!(matches!(recovery.save(), Err(SaveError::Read(_))));
        assert!(location.state_path().is_dir());
        assert_eq!(
            fs::read(location.backup_path()).unwrap(),
            backup().encode().unwrap()
        );
    }
}

#[test]
fn uncertain_commit_survives_read_conflict_and_sync_errors_until_reconciled() {
    for stage in [
        SaveStage::VerifyRecoverySources,
        SaveStage::SyncStorageAncestry,
        SaveStage::SyncQuarantine,
        SaveStage::SyncQuarantineDirectory,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, Some(BROKEN))
            .confirm(RESTORED_AT)
            .unwrap();
        let fixed = recovery.pending_save().unwrap().encoded_bytes().to_vec();
        recovery
            .save_with_hook(&mut fail(SaveStage::SyncPrimaryDirectory))
            .unwrap_err();
        assert!(
            recovery
                .save_with_hook(&mut fail(stage))
                .unwrap_err()
                .is_commit_uncertain()
        );
        fs::remove_file(location.state_path()).unwrap();
        fs::create_dir(location.state_path()).unwrap();
        let error = recovery.save().unwrap_err();
        assert!(matches!(error.failure(), SaveError::Read(_)));
        assert!(error.is_commit_uncertain());
        fs::remove_dir(location.state_path()).unwrap();
        fs::write(location.state_path(), b"third contents").unwrap();
        let error = recovery.save().unwrap_err();
        assert!(matches!(error.failure(), SaveError::Conflict { .. }));
        assert!(error.is_commit_uncertain());
        fs::write(location.state_path(), &fixed).unwrap();
        let store = recovery.save().unwrap();
        assert_eq!(store.saved_state().unwrap().original_bytes(), fixed);
    }
}

#[test]
fn old_primary_after_uncertainty_rewrites_the_same_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut recovery = prepare(&location, Some(BROKEN))
        .confirm(RESTORED_AT)
        .unwrap();
    let bytes = recovery.pending_save().unwrap().encoded_bytes().to_vec();
    recovery
        .save_with_hook(&mut fail(SaveStage::SyncPrimaryDirectory))
        .unwrap_err();
    fs::write(location.state_path(), BROKEN).unwrap();
    let error = recovery
        .save_with_hook(&mut fail(SaveStage::RenamePrimary))
        .unwrap_err();
    assert!(!error.is_commit_uncertain());
    assert_eq!(recovery.pending_save().unwrap().encoded_bytes(), bytes);
    let _store = recovery.save().unwrap();
    assert_eq!(fs::read(location.state_path()).unwrap(), bytes);
    assert_eq!(
        fs::read(location.backup_path()).unwrap(),
        backup().encode().unwrap()
    );
}

#[test]
fn changed_archive_is_preserved_and_never_authorizes_replacement() {
    for replace_inode in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut recovery = prepare(&location, Some(BROKEN))
            .confirm(RESTORED_AT)
            .unwrap();
        recovery
            .save_with_hook(&mut fail(SaveStage::SyncQuarantineDirectory))
            .unwrap_err();
        let path = recovery.quarantine_path().unwrap().to_owned();
        let bytes = if replace_inode {
            fs::remove_file(&path).unwrap();
            BROKEN
        } else {
            b"changed archive"
        };
        fs::write(&path, bytes).unwrap();
        assert!(matches!(recovery.save(), Err(SaveError::Conflict { .. })));
        drop(recovery);
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
    }
}

#[test]
fn preparation_overflow_never_changes_files() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let candidate = prepare(&location, Some(BROKEN));
    drop(candidate);
    let mut state = backup();
    state.save_generation = u64::MAX;
    let bytes = state.encode().unwrap();
    fs::write(location.backup_path(), &bytes).unwrap();
    let LoadOutcome::RecoveryRequired(candidate) = location.clone().lock().unwrap().load().unwrap()
    else {
        panic!("expected recovery");
    };
    assert!(matches!(
        candidate.confirm(RESTORED_AT),
        Err(SaveError::GenerationExhausted)
    ));
    assert_eq!(fs::read(location.backup_path()).unwrap(), bytes);
    assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}

#[test]
fn recovery_records_one_restore_even_after_many_failures() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut recovery = prepare(&location, Some(BROKEN))
        .confirm(RESTORED_AT)
        .unwrap();
    for &stage in STAGES {
        // Once archived, the creation stages no longer run.
        if matches!(
            stage,
            SaveStage::CreateQuarantine | SaveStage::WriteQuarantine
        ) {
            continue;
        }
        recovery.save_with_hook(&mut fail(stage)).unwrap_err();
    }
    let mut store = recovery.save().unwrap();
    let recovered = store.saved_state().unwrap().original_bytes().to_vec();
    let domain = store.saved_state().unwrap().domain().clone();
    assert_eq!(
        domain
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::AppRestored))
            .count(),
        1
    );
    store.save(&domain, Timestamp(6_000)).unwrap();
    assert_eq!(store.saved_state().unwrap().save_generation(), 9);
    assert_eq!(fs::read(location.backup_path()).unwrap(), recovered);
}
