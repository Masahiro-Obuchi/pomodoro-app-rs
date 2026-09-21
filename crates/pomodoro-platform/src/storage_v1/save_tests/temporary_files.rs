use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

// Cause both a write failure and a real unlink failure. Restoring permissions
// happens before assertions so a failing test still permits TempDir cleanup.
fn leave_owned_temporary(location: &StorageLocation) -> Option<(WritableStorage, PathBuf)> {
    let mut store = load(location);
    let mut temporary_path = None;
    let result = store.save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
        if stage == SaveStage::WritePrimaryTemp {
            temporary_path = Some(path.to_owned());
            fs::write(path, b"partial candidate")?;
            fs::set_permissions(location.directory(), fs::Permissions::from_mode(0o500))?;
            return Err(io::Error::other("injected write failure"));
        }
        Ok(())
    });
    fs::set_permissions(location.directory(), fs::Permissions::from_mode(0o700)).unwrap();
    let error = result.unwrap_err();
    assert!(matches!(
        error.failure(),
        SaveError::Io {
            stage: SaveStage::WritePrimaryTemp,
            ..
        }
    ));
    let path = temporary_path.unwrap();
    if !path.exists() {
        eprintln!("unlink-failure case requires an unprivileged process");
        return None;
    }
    assert!(
        store
            .pending_save()
            .unwrap()
            .temporary_files
            .owns(&path)
            .unwrap()
    );
    Some((store, path))
}

#[test]
fn initial_retry_accepts_only_its_owned_temporary_and_cleans_it() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let Some((mut store, path)) = leave_owned_temporary(&location) else {
        return;
    };
    let bytes = store.pending_save().unwrap().encoded_bytes().to_vec();
    store.retry_pending().unwrap();
    assert_eq!(store.saved_state().unwrap().original_bytes(), bytes);
    assert!(!path.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    assert!(!location.backup_path().exists());
}

#[test]
fn an_owned_temporary_does_not_hide_external_backups() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let Some((mut store, path)) = leave_owned_temporary(&location) else {
        return;
    };
    fs::write(location.backup_path(), VALID).unwrap();
    let error = store
        .retry_pending_with_hook(&mut |_, _| panic!("conflict must precede I/O"))
        .unwrap_err();
    assert!(matches!(error, SaveError::Conflict { path } if path == location.backup_path()));
    assert!(!location.state_path().exists());
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
    assert_eq!(fs::read(&path).unwrap(), b"partial candidate");
    drop(store);
    assert!(!path.exists());
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn replaced_temporary_paths_are_neither_accepted_nor_deleted() {
    for use_symlink in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let Some((mut store, path)) = leave_owned_temporary(&location) else {
            return;
        };
        // The old file remains open, so its inode cannot be reused by replacement.
        fs::remove_file(&path).unwrap();
        let target_directory = tempfile::tempdir().unwrap();
        let target = target_directory.path().join("external");
        if use_symlink {
            fs::write(&target, VALID).unwrap();
            symlink(&target, &path).unwrap();
        } else {
            fs::write(&path, VALID).unwrap();
        }
        let error = store
            .retry_pending_with_hook(&mut |_, _| panic!("must not modify an external replacement"))
            .unwrap_err();
        assert!(matches!(error, SaveError::Conflict { path: conflict } if conflict == path));
        drop(store);
        assert_eq!(fs::read(&path).unwrap(), VALID);
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().is_symlink(),
            use_symlink
        );
    }
}
