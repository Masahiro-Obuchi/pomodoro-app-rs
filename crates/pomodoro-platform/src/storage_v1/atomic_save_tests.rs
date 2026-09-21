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
    let ancestor_count = location.directory().ancestors().count();
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        &stages[..ancestor_count],
        vec![SaveStage::SyncStorageAncestry; ancestor_count]
    );
    assert_eq!(&stages[ancestor_count..], SAVE_STAGES);
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
fn storage_ancestry_is_synced_deepest_first_once_per_handle() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("new/nested/store"));
    let mut store = load(&location);
    let mut synced = Vec::new();
    let mut stages = Vec::new();
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
            stages.push(stage);
            if stage == SaveStage::SyncStorageAncestry {
                synced.push(path.to_owned());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(synced.first().unwrap(), location.directory());
    assert_eq!(synced.last().unwrap(), Path::new("/"));
    assert!(synced.contains(&directory.path().to_owned()));
    for pair in synced.windows(2) {
        assert_eq!(pair[0].parent(), Some(pair[1].as_path()));
    }
    assert_eq!(
        &stages[..synced.len()],
        vec![SaveStage::SyncStorageAncestry; synced.len()]
    );
    assert_eq!(&stages[synced.len()..], &SAVE_STAGES[5..]);
    assert!(store.locked.directories_to_sync.is_empty());
    // Successfully persisted directory entries need no repeated ancestor sync.
    store
        .save_with_hook(&domain(), Timestamp(3_000), &mut |stage, _| {
            assert_ne!(stage, SaveStage::SyncStorageAncestry);
            Ok(())
        })
        .unwrap();
}

#[test]
fn each_ancestor_sync_failure_blocks_writes_even_after_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("new/store"));
    let ancestor_count = location.directory().ancestors().count();
    for failed_index in 0..ancestor_count {
        let mut store = load(&location);
        let mut index = 0;
        let error = store
            .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
                assert_eq!(stage, SaveStage::SyncStorageAncestry);
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
                stage: SaveStage::SyncStorageAncestry,
                ..
            }
        ));
        assert!(!error.is_commit_uncertain());
        assert!(store.saved_state().is_none());
        assert!(store.pending_save().is_some());
        assert_eq!(store.locked.directories_to_sync.len(), ancestor_count);
        assert_eq!(fs::read_dir(location.directory()).unwrap().count(), 1);
    }
}

#[test]
fn reopened_store_syncs_ancestors_after_an_abandoned_directory_sync_failure() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("new/nested/store"));
    let mut store = load(&location);
    let error = store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            assert_eq!(stage, SaveStage::SyncStorageAncestry);
            Err(io::Error::other("injected first-directory sync failure"))
        })
        .unwrap_err();
    assert!(matches!(error, SaveError::Io { .. }));
    assert!(store.saved_state().is_none());
    drop(store);

    let mut reopened = load(&location);
    let mut synced = Vec::new();
    reopened
        .save_with_hook(&domain(), Timestamp(3_000), &mut |stage, path| {
            if stage == SaveStage::SyncStorageAncestry {
                synced.push(path.to_owned());
            } else {
                // The directory entry for "new" belongs to this parent.
                // No write may proceed before that parent has been synced.
                assert!(synced.contains(&directory.path().to_owned()));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(reopened.saved_state().unwrap().save_generation(), 1);
}

#[test]
fn ancestor_sync_failure_preserves_loaded_primary_and_backup_across_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    for _ in 0..2 {
        let mut store = load(&location);
        let error = store
            .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
                assert_eq!(stage, SaveStage::SyncStorageAncestry);
                if path == Path::new("/") {
                    Err(io::Error::other("injected final-ancestor sync failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(matches!(
            error,
            SaveError::Io {
                stage: SaveStage::SyncStorageAncestry,
                ..
            }
        ));
        assert!(!error.is_commit_uncertain());
        assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
        assert_eq!(fs::read(location.backup_path()).unwrap(), OLD_BACKUP);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
        assert_eq!(store.saved_state().unwrap().save_generation(), 7);
        assert_eq!(store.pending_save().unwrap().save_generation(), 8);
    }
    let mut reopened = load(&location);
    reopened.save(&domain(), Timestamp(3_000)).unwrap();
    assert_eq!(reopened.saved_state().unwrap().save_generation(), 8);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn retry_confirms_an_uncertain_candidate_without_rewriting_files() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    let mut store = load(&location);
    let domain = domain();
    store
        .save_with_hook(&domain, Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::SyncPrimaryDirectory {
                Err(io::Error::other("injected uncertain commit"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let pending = store.pending_save().unwrap();
    let candidate = (
        pending.domain().clone(),
        pending.save_generation(),
        pending.saved_at(),
        pending.encoded_bytes().to_vec(),
    );
    assert_eq!(fs::read(location.state_path()).unwrap(), candidate.3);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);

    let mut stages = Vec::new();
    store
        .retry_pending_with_hook(&mut |stage, _| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
    assert!(
        stages
            .iter()
            .all(|stage| *stage == SaveStage::SyncStorageAncestry
                || *stage == SaveStage::SyncPrimaryDirectory)
    );
    assert_eq!(stages.last(), Some(&SaveStage::SyncPrimaryDirectory));
    assert!(store.pending_save().is_none());
    let committed = store.saved_state().unwrap();
    assert_eq!(committed.domain(), &candidate.0);
    assert_eq!(committed.save_generation(), candidate.1);
    assert_eq!(committed.saved_at(), candidate.2);
    assert_eq!(committed.original_bytes(), candidate.3);
    assert_eq!(fs::read(location.state_path()).unwrap(), candidate.3);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn retry_rewrites_the_fixed_candidate_only_after_confirming_the_old_primary() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    let mut store = load(&location);
    let domain = domain();
    store
        .save_with_hook(&domain, Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::CreatePrimaryTemp {
                Err(io::Error::other("injected pre-rename failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    // The first attempt already copied the known old primary to the backup.
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);

    let mut stages = Vec::new();
    store
        .retry_pending_with_hook(&mut |stage, _| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
    let ancestry_count = location.directory().ancestors().count();
    assert_eq!(
        &stages[..ancestry_count],
        vec![SaveStage::SyncStorageAncestry; ancestry_count]
    );
    assert_eq!(&stages[ancestry_count..], SAVE_STAGES);
    assert!(store.pending_save().is_none());
    assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
    assert_eq!(fs::read(location.state_path()).unwrap(), candidate);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn retry_writes_an_initial_candidate_without_creating_a_backup() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut store = load(&location);
    let domain = domain();
    store
        .save_with_hook(&domain, Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::CreatePrimaryTemp {
                Err(io::Error::other("injected initial pre-rename failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    assert!(!location.state_path().exists());
    assert!(!location.backup_path().exists());

    store.retry_pending().unwrap();
    assert!(store.pending_save().is_none());
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(2_000));
    assert_eq!(fs::read(location.state_path()).unwrap(), candidate);
    assert!(!location.backup_path().exists());
}

#[test]
fn retry_rejects_a_third_primary_without_touching_the_backup_or_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    let mut store = load(&location);
    let domain = domain();
    store
        .save_with_hook(&domain, Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::CreatePrimaryTemp {
                Err(io::Error::other("injected pre-rename failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    let third = b"third primary bytes";
    fs::write(location.state_path(), third).unwrap();
    let before_backup = fs::read(location.backup_path()).unwrap();

    let error = store
        .retry_pending_with_hook(&mut |_, _| panic!("must not write before comparison"))
        .unwrap_err();
    assert!(matches!(error, SaveError::Conflict { .. }));
    assert_eq!(fs::read(location.state_path()).unwrap(), third);
    assert_eq!(fs::read(location.backup_path()).unwrap(), before_backup);
    assert_eq!(store.pending_save().unwrap().encoded_bytes(), candidate);
    assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
}

#[test]
fn failed_retries_keep_the_same_candidate_until_a_later_success() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(&location);
    let domain = domain();
    store
        .save_with_hook(&domain, Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::SyncPrimaryDirectory {
                Err(io::Error::other("injected initial uncertainty"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();

    for _ in 0..2 {
        let error = store
            .retry_pending_with_hook(&mut |stage, _| {
                if stage == SaveStage::SyncPrimaryDirectory {
                    Err(io::Error::other("injected retry uncertainty"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(error.is_commit_uncertain());
        assert_eq!(store.pending_save().unwrap().encoded_bytes(), candidate);
        assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
        assert_eq!(fs::read(location.state_path()).unwrap(), candidate);
        assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
    }
    store.retry_pending_with_hook(&mut |_, _| Ok(())).unwrap();
    assert!(store.pending_save().is_none());
    assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
}

#[test]
fn retry_without_a_pending_candidate_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    let mut store = load(&location);
    let error = store.retry_pending().unwrap_err();
    assert!(matches!(error, SaveError::NoPendingSave));
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    assert_eq!(fs::read(location.backup_path()).unwrap(), OLD_BACKUP);
    assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
    assert!(store.pending_save().is_none());
}
