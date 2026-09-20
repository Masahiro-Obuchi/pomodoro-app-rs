use super::*;
use crate::{LoadOutcome, StorageLocation, StorageLockError};
use pomodoro_core::{Command, SessionKind, TimerConfig};

const VALID: &[u8] = include_bytes!("../../tests/fixtures/state_v1.json");
const OLD_BACKUP: &[u8] = b"previous backup bytes";
const SAVE_STAGES: [SaveStage; 10] = [
    SaveStage::CreateBackupTemp,
    SaveStage::WriteBackupTemp,
    SaveStage::SyncBackupTemp,
    SaveStage::RenameBackup,
    SaveStage::SyncBackupDirectory,
    SaveStage::CreatePrimaryTemp,
    SaveStage::WritePrimaryTemp,
    SaveStage::SyncPrimaryTemp,
    SaveStage::RenamePrimary,
    SaveStage::SyncPrimaryDirectory,
];

fn domain() -> DomainState {
    let mut domain = DomainState::new(TimerConfig::default()).unwrap();
    domain
        .apply(Command::Start(SessionKind::QuickStart), Timestamp(1_000))
        .unwrap();
    domain
}

fn load(location: &StorageLocation) -> WritableStorage {
    match location.clone().lock().unwrap().load().unwrap() {
        LoadOutcome::New(store) | LoadOutcome::Loaded(store) => store,
        LoadOutcome::RecoveryRequired(_) => panic!("unexpected recovery candidate"),
    }
}

#[test]
fn backup_and_primary_follow_the_durable_save_order() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(&location);
    let mut stages = Vec::new();
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
    assert_eq!(stages, SAVE_STAGES);
    assert_eq!(store.saved_state().unwrap().save_generation(), 8);
    assert!(store.pending_save().is_none());
}

#[test]
fn every_io_failure_preserves_the_baseline_and_fixed_candidate() {
    for failing_stage in SAVE_STAGES {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        fs::write(location.state_path(), VALID).unwrap();
        fs::write(location.backup_path(), OLD_BACKUP).unwrap();
        let mut store = load(&location);
        let domain = domain();
        let error = store
            .save_with_hook(&domain, Timestamp(2_000), &mut |stage, path| {
                if stage == failing_stage {
                    if matches!(
                        stage,
                        SaveStage::WriteBackupTemp | SaveStage::WritePrimaryTemp
                    ) {
                        // Model a partial write too: the incomplete temp must not
                        // replace the target and normal error cleanup removes it.
                        fs::write(path, b"partial JSON")?;
                    }
                    return Err(io::Error::other("injected save failure"));
                }
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, SaveError::Io { stage, .. } if stage == failing_stage));
        let uncertain = failing_stage == SaveStage::SyncPrimaryDirectory;
        assert_eq!(error.is_commit_uncertain(), uncertain);
        assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
        assert_eq!(store.saved_state().unwrap().save_generation(), 7);
        let candidate = store.pending_save().unwrap();
        assert_eq!(candidate.domain(), &domain);
        assert_eq!(candidate.save_generation(), 8);
        assert_eq!(candidate.saved_at(), Timestamp(2_000));
        let frozen_bytes = candidate.encoded_bytes().to_vec();
        assert_eq!(
            PersistedStateV1::decode(&frozen_bytes).unwrap().domain,
            domain
        );
        assert_eq!(
            fs::read(location.state_path()).unwrap(),
            if uncertain {
                frozen_bytes.as_slice()
            } else {
                VALID
            }
        );
        let backup_was_renamed = SAVE_STAGES
            .iter()
            .position(|stage| *stage == failing_stage)
            .unwrap()
            >= 4;
        assert_eq!(
            fs::read(location.backup_path()).unwrap(),
            if backup_was_renamed {
                VALID
            } else {
                OLD_BACKUP
            }
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
        assert!(matches!(
            location.clone().lock(),
            Err(StorageLockError::InUse { .. })
        ));
        assert!(matches!(
            store.save(&domain, Timestamp(9_000)),
            Err(SaveError::PendingSave)
        ));
        assert_eq!(store.pending_save().unwrap().encoded_bytes(), frozen_bytes);
        assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
    }
}

#[test]
fn initial_save_failures_do_not_manufacture_a_committed_generation_or_backup() {
    for failing_stage in &SAVE_STAGES[5..] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut store = load(&location);
        let error = store
            .save_with_hook(&domain(), Timestamp(1_000), &mut |stage, _| {
                if stage == *failing_stage {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(matches!(error, SaveError::Io { stage, .. } if stage == *failing_stage));
        assert!(store.saved_state().is_none());
        assert_eq!(store.pending_save().unwrap().save_generation(), 1);
        assert!(!location.backup_path().exists());
        if error.is_commit_uncertain() {
            assert_eq!(
                fs::read(location.state_path()).unwrap(),
                store.pending_save().unwrap().encoded_bytes()
            );
        } else {
            assert!(!location.state_path().exists());
        }
    }
}

#[test]
fn nested_new_directories_are_synced_deepest_first_before_the_first_commit() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("new/nested/store"));
    let mut store = load(&location);
    let mut synced = Vec::new();
    let mut stages = Vec::new();
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
            stages.push(stage);
            if stage == SaveStage::SyncCreatedDirectory {
                synced.push(path.to_owned());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        synced,
        vec![
            location.directory().to_owned(),
            directory.path().join("new/nested"),
            directory.path().join("new"),
            directory.path().to_owned()
        ]
    );
    assert_eq!(&stages[..4], &[SaveStage::SyncCreatedDirectory; 4]);
    assert_eq!(&stages[4..], &SAVE_STAGES[5..]);
    assert!(store.locked.directories_to_sync.is_empty());
    // Successfully persisted directory entries need no repeated ancestor sync.
    store
        .save_with_hook(&domain(), Timestamp(3_000), &mut |stage, _| {
            assert_ne!(stage, SaveStage::SyncCreatedDirectory);
            Ok(())
        })
        .unwrap();
}

#[test]
fn each_new_directory_sync_failure_prevents_first_commit() {
    for failed_index in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().join("new/store"));
        let mut store = load(&location);
        let mut index = 0;
        let error = store
            .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
                assert_eq!(stage, SaveStage::SyncCreatedDirectory);
                if index == failed_index {
                    return Err(io::Error::other("injected"));
                }
                index += 1;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(
            error,
            SaveError::Io {
                stage: SaveStage::SyncCreatedDirectory,
                ..
            }
        ));
        assert!(!error.is_commit_uncertain());
        assert!(store.saved_state().is_none());
        assert!(store.pending_save().is_some());
        assert_eq!(store.locked.directories_to_sync.len(), 3);
        assert_eq!(fs::read_dir(location.directory()).unwrap().count(), 1);
    }
}
