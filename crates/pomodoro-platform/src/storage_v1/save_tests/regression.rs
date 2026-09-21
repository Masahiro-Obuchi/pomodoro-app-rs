use super::*;

#[test]
fn initial_retry_rejects_external_remnants_before_any_io() {
    for name in ["state.json.bak", "state.json.tmp-external", "unknown-file"] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut store = load(&location);
        store
            .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
                if stage == SaveStage::CreatePrimaryTemp {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
        let external = directory.path().join(name);
        fs::write(&external, VALID).unwrap();
        let error = store
            .retry_pending_with_hook(&mut |_, _| panic!("must check remnants before I/O"))
            .unwrap_err();
        assert!(matches!(error, SaveError::Conflict { path } if path == external));
        assert!(!location.state_path().exists());
        assert_eq!(fs::read(&external).unwrap(), VALID);
        assert_eq!(store.pending_save().unwrap().encoded_bytes(), candidate);
        fs::remove_file(external).unwrap();
        store.retry_pending().unwrap();
        assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
        assert!(!location.backup_path().exists());
    }
}

fn uncertain_store(location: &StorageLocation) -> WritableStorage {
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(location);
    let error = store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            if stage == SaveStage::SyncPrimaryDirectory {
                Err(io::Error::other("injected uncertain save"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert!(error.is_commit_uncertain());
    store
}

#[test]
fn every_retry_sync_failure_keeps_uncertainty_and_original_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut store = uncertain_store(&location);
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    let mut failures: Vec<_> = location
        .directory()
        .ancestors()
        .map(|path| (SaveStage::SyncStorageAncestry, path.to_owned()))
        .collect();
    failures.push((
        SaveStage::SyncPrimaryDirectory,
        location.directory().to_owned(),
    ));
    for (failed_stage, failed_path) in failures {
        let error = store
            .retry_pending_with_hook(&mut |stage, path| {
                if stage == failed_stage && path == failed_path {
                    Err(io::Error::other("injected retry sync"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(error.is_commit_uncertain());
        assert!(
            matches!(error.failure(), SaveError::Io { stage, path, source } if *stage == failed_stage && *path == failed_path && source.to_string() == "injected retry sync")
        );
        assert!(store.pending_save().unwrap().is_commit_uncertain());
        assert_eq!(store.pending_save().unwrap().encoded_bytes(), candidate);
        assert_eq!(fs::read(location.state_path()).unwrap(), candidate);
        assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
        assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
        let blocked = store.save(&domain(), Timestamp(3_000)).unwrap_err();
        assert!(blocked.is_commit_uncertain());
        assert!(matches!(blocked.failure(), SaveError::PendingSave));
    }
    store.retry_pending().unwrap();
    assert!(store.pending_save().is_none());
    assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
}

#[test]
fn failed_reconciliation_retains_uncertainty_until_the_baseline_is_confirmed() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut store = uncertain_store(&location);
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    // A non-file primary triggers a real read error before any retry sync.
    fs::remove_file(location.state_path()).unwrap();
    fs::create_dir(location.state_path()).unwrap();
    let error = store.retry_pending().unwrap_err();
    assert!(error.is_commit_uncertain());
    assert!(matches!(error.failure(), SaveError::Read(_)));
    fs::remove_dir(location.state_path()).unwrap();
    fs::write(location.state_path(), b"third-party value").unwrap();
    let error = store.retry_pending().unwrap_err();
    assert!(error.is_commit_uncertain());
    assert!(matches!(error.failure(), SaveError::Conflict { .. }));
    assert_eq!(store.pending_save().unwrap().encoded_bytes(), candidate);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);

    fs::write(location.state_path(), VALID).unwrap();
    let error = store
        .retry_pending_with_hook(&mut |_, _| Err(io::Error::other("before rewriting")))
        .unwrap_err();
    assert!(!error.is_commit_uncertain());
    assert!(!store.pending_save().unwrap().is_commit_uncertain());
    store.retry_pending().unwrap();
    assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
}
