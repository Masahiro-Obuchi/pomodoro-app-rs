use super::*;

#[derive(Clone, Copy, Debug)]
enum ArchiveChange {
    Delete,
    Replace,
    Edit,
}

#[test]
fn archive_changes_during_the_same_attempt_block_primary_replacement() {
    for stage in [
        SaveStage::SyncQuarantine,
        SaveStage::SyncQuarantineDirectory,
        SaveStage::CreatePrimaryTemp,
        SaveStage::WritePrimaryTemp,
        SaveStage::SyncPrimaryTemp,
        SaveStage::RenamePrimary,
    ] {
        for change in [
            ArchiveChange::Delete,
            ArchiveChange::Replace,
            ArchiveChange::Edit,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let location = StorageLocation::at(directory.path().to_owned());
            let mut recovery = prepare(&location, Some(BROKEN))
                .confirm(RESTORED_AT)
                .unwrap();
            let candidate = recovery.pending_save().unwrap().encoded_bytes().to_vec();
            let mut archive = None;
            let mut changed = false;
            let result = recovery.save_with_hook(&mut |current, path| {
                if current == SaveStage::WriteQuarantine {
                    archive = Some(path.to_owned());
                }
                if current == stage {
                    let path = archive.as_ref().unwrap();
                    match change {
                        ArchiveChange::Delete => fs::remove_file(path)?,
                        ArchiveChange::Replace => {
                            fs::remove_file(path)?;
                            fs::write(path, BROKEN)?;
                        }
                        ArchiveChange::Edit => fs::write(path, b"external archive edit")?,
                    }
                    changed = true;
                }
                Ok(())
            });
            assert!(changed);
            assert!(
                result.is_err(),
                "archive {change:?} at {stage:?} must stop recovery"
            );
            let error = result.unwrap_err();
            assert!(matches!(
                error.failure(),
                SaveError::Read(_) | SaveError::Conflict { .. }
            ));
            assert!(!error.is_commit_uncertain());
            assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
            assert_eq!(
                fs::read(location.backup_path()).unwrap(),
                backup().encode().unwrap()
            );
            assert_eq!(recovery.pending_save().unwrap().encoded_bytes(), candidate);
            // A later retry cannot accept or delete the altered archive either.
            assert!(recovery.save().is_err());
            drop(recovery);
            assert_eq!(fs::read(location.state_path()).unwrap(), BROKEN);
            let actual = fs::read(archive.unwrap()).ok();
            match change {
                ArchiveChange::Delete => assert!(actual.is_none()),
                ArchiveChange::Replace => assert_eq!(actual.as_deref(), Some(BROKEN)),
                ArchiveChange::Edit => {
                    assert_eq!(actual.as_deref(), Some(b"external archive edit".as_slice()));
                }
            }
            assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-")
            }));
        }
    }
}

#[test]
fn source_changes_during_temporary_write_block_rename_and_retain_the_candidate() {
    for stage in [
        SaveStage::CreatePrimaryTemp,
        SaveStage::WritePrimaryTemp,
        SaveStage::SyncPrimaryTemp,
        SaveStage::RenamePrimary,
    ] {
        for modify_backup in [false, true] {
            for original in [None, Some(BROKEN)] {
                let directory = tempfile::tempdir().unwrap();
                let location = StorageLocation::at(directory.path().to_owned());
                let mut recovery = prepare(&location, original).confirm(RESTORED_AT).unwrap();
                let candidate = recovery.pending_save().unwrap().encoded_bytes().to_vec();
                let path = if modify_backup {
                    location.backup_path()
                } else {
                    location.state_path()
                };
                let mut changed = false;
                let result = recovery.save_with_hook(&mut |current, _| {
                    if current == stage {
                        fs::write(&path, b"external source update")?;
                        changed = true;
                    }
                    Ok(())
                });
                assert!(changed);
                assert!(
                    result.is_err(),
                    "source update at {stage:?} must stop recovery"
                );
                let error = result.unwrap_err();
                assert!(matches!(error, SaveError::Conflict { path: actual } if actual == path));
                assert_eq!(fs::read(&path).unwrap(), b"external source update");
                if modify_backup {
                    assert_eq!(fs::read(location.state_path()).ok().as_deref(), original);
                } else {
                    assert_eq!(
                        fs::read(location.backup_path()).unwrap(),
                        backup().encode().unwrap()
                    );
                }
                assert_eq!(recovery.pending_save().unwrap().encoded_bytes(), candidate);
                assert!(!recovery.pending_save().unwrap().is_commit_uncertain());
                assert!(matches!(recovery.save(), Err(SaveError::Conflict { .. })));
                assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains(".tmp-")
                }));
                // Once the external conflict is resolved, retry the original JSON.
                if modify_backup {
                    fs::write(path, backup().encode().unwrap()).unwrap();
                } else if let Some(bytes) = original {
                    fs::write(path, bytes).unwrap();
                } else {
                    fs::remove_file(path).unwrap();
                }
                let store = recovery.save().unwrap();
                assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
                if let Some(bytes) = original {
                    assert_eq!(
                        fs::read(recovery.quarantine_path().unwrap()).unwrap(),
                        bytes
                    );
                }
            }
        }
    }
}

#[test]
fn changes_during_commit_reconciliation_retain_uncertainty() {
    for stage in [
        SaveStage::SyncQuarantine,
        SaveStage::SyncQuarantineDirectory,
        SaveStage::SyncPrimaryDirectory,
    ] {
        for target in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            let location = StorageLocation::at(directory.path().to_owned());
            let mut recovery = prepare(&location, Some(BROKEN))
                .confirm(RESTORED_AT)
                .unwrap();
            recovery
                .save_with_hook(&mut fail(SaveStage::SyncPrimaryDirectory))
                .unwrap_err();
            let candidate = recovery.pending_save().unwrap().encoded_bytes().to_vec();
            let path = match target {
                0 => location.state_path(),
                1 => location.backup_path(),
                _ => recovery.quarantine_path().unwrap().to_owned(),
            };
            let original = fs::read(&path).unwrap();
            let mut changed = false;
            let error = recovery
                .save_with_hook(&mut |current, _| {
                    if current == stage {
                        fs::write(&path, b"changed during reconciliation")?;
                        changed = true;
                    }
                    Ok(())
                })
                .unwrap_err();
            assert!(changed);
            assert!(
                matches!(error.failure(), SaveError::Conflict { path: actual } if actual == &path)
            );
            assert!(error.is_commit_uncertain());
            assert!(recovery.pending_save().unwrap().is_commit_uncertain());
            assert_eq!(recovery.pending_save().unwrap().encoded_bytes(), candidate);
            assert_eq!(fs::read(&path).unwrap(), b"changed during reconciliation");
            fs::write(path, original).unwrap();
            let store = recovery.save().unwrap();
            assert_eq!(store.saved_state().unwrap().original_bytes(), candidate);
        }
    }
}
