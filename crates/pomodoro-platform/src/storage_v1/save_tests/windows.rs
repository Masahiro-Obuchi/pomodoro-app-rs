use super::*;
use std::{
    fs::OpenOptions,
    os::windows::fs::{OpenOptionsExt, symlink_file},
};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

#[test]
fn replacement_error_keeps_the_candidate_uncertain_until_primary_is_reread() {
    let directory = crate::storage_v1::test_tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(&location);
    let mut blocking_handle = None;
    let error = store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
            if stage == SaveStage::RenamePrimary {
                // The baseline has already been read. This handle prevents
                // replacing the target and exercises an actual rename error.
                blocking_handle = Some(
                    OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                        .open(path)?,
                );
            }
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(
        error.failure(),
        SaveError::Io {
            stage: SaveStage::RenamePrimary,
            ..
        }
    ));
    assert!(error.is_commit_uncertain());
    assert!(store.pending_save().unwrap().is_commit_uncertain());
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    let fixed = store.pending_save().unwrap().encoded_bytes().to_vec();
    drop(blocking_handle);
    store.retry_pending().unwrap();
    assert_eq!(fs::read(location.state_path()).unwrap(), fixed);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn replacement_never_writes_through_an_external_symlink() {
    let directory = crate::storage_v1::test_tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(&location);
    let external = directory.path().join("outside.json");
    fs::write(&external, b"external data").unwrap();
    let result = store.save_with_hook(&domain(), Timestamp(2_000), &mut |stage, path| {
        if stage == SaveStage::RenamePrimary {
            fs::remove_file(path)?;
            symlink_file(&external, path)?;
        }
        Ok(())
    });
    assert_eq!(fs::read(&external).unwrap(), b"external data");
    match result {
        Ok(saved) => assert_eq!(
            fs::read(location.state_path()).unwrap(),
            saved.original_bytes()
        ),
        Err(error) => {
            assert!(error.is_commit_uncertain());
            assert!(store.pending_save().is_some());
        }
    }
}
