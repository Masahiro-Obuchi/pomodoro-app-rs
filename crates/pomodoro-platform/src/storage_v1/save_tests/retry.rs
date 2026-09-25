use super::*;

#[test]
fn retry_confirms_an_uncertain_candidate_without_rewriting_files() {
    let directory = crate::storage_v1::test_tempdir();
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
    let directory = crate::storage_v1::test_tempdir();
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
    let ancestry_count = store.locked.directories_to_sync.len();
    store
        .retry_pending_with_hook(&mut |stage, _| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
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
    let directory = crate::storage_v1::test_tempdir();
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
    let directory = crate::storage_v1::test_tempdir();
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
    let directory = crate::storage_v1::test_tempdir();
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
    let directory = crate::storage_v1::test_tempdir();
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
