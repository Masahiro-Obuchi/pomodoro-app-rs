use super::*;

#[test]
fn repeated_failures_preserve_candidate_and_resume_normal_saves() {
    for initial_store in [false, true] {
        let mut stages = vec![SaveStage::SyncStorageAncestry];
        stages.extend_from_slice(if initial_store {
            &SAVE_STAGES[5..]
        } else {
            &SAVE_STAGES
        });
        for initial_failure in &stages {
            let retry_stages = if *initial_failure == SaveStage::SyncPrimaryDirectory {
                vec![
                    SaveStage::SyncStorageAncestry,
                    SaveStage::SyncPrimaryDirectory,
                ]
            } else {
                stages.clone()
            };
            for retry_failure in retry_stages {
                check_attempts(initial_store, *initial_failure, retry_failure);
            }
        }
    }
}

fn check_attempts(initial_store: bool, initial_failure: SaveStage, retry_failure: SaveStage) {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    if !initial_store {
        fs::write(location.state_path(), VALID).unwrap();
        fs::write(location.backup_path(), OLD_BACKUP).unwrap();
    }
    let mut store = load(&location);
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            if stage == initial_failure {
                Err(io::Error::other("first failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let pending = store.pending_save().unwrap();
    let bytes = pending.encoded_bytes().to_vec();
    let generation = pending.save_generation();
    let uncertain = initial_failure == SaveStage::SyncPrimaryDirectory
        || retry_failure == SaveStage::SyncPrimaryDirectory;
    for _ in 0..2 {
        let error = store
            .retry_pending_with_hook(&mut |stage, _| {
                if stage == retry_failure {
                    Err(io::Error::other("retry failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(matches!(error.failure(), SaveError::Io { stage, .. } if *stage == retry_failure));
        assert_eq!(error.is_commit_uncertain(), uncertain);
        assert_eq!(
            store.pending_save().unwrap().is_commit_uncertain(),
            uncertain
        );
        assert_eq!(store.pending_save().unwrap().encoded_bytes(), bytes);
        assert_eq!(store.pending_save().unwrap().saved_at(), Timestamp(2_000));
        assert_eq!(store.pending_save().unwrap().save_generation(), generation);
        assert_eq!(
            store.saved_state().map(SavedState::original_bytes),
            if initial_store { None } else { Some(VALID) }
        );
    }
    let committed = store.retry_pending().unwrap();
    assert_eq!(committed.original_bytes(), bytes);
    assert_eq!(committed.save_generation(), generation);
    assert_eq!(committed.saved_at(), Timestamp(2_000));
    assert!(store.pending_save().is_none());
    assert_eq!(fs::read(location.state_path()).unwrap(), bytes);
    assert_eq!(
        fs::read(location.backup_path()).ok().as_deref(),
        if initial_store { None } else { Some(VALID) }
    );
    store.save(&domain(), Timestamp(3_000)).unwrap();
    assert_eq!(
        store.saved_state().unwrap().save_generation(),
        generation + 1
    );
    assert_eq!(fs::read(location.backup_path()).unwrap(), bytes);
}
