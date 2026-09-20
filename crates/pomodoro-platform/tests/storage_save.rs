#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use pomodoro_core::{Command, DomainState, SessionKind, TimerConfig, Timestamp};
use pomodoro_platform::{
    LoadOutcome, SaveError, SaveStage, StorageLocation, StorageLockError, WritableStorage,
};
use serde_json::{Value, json};

const VALID: &[u8] = include_bytes!("fixtures/state_v1.json");

fn load(location: &StorageLocation) -> WritableStorage {
    match location.clone().lock().unwrap().load().unwrap() {
        LoadOutcome::New(store) | LoadOutcome::Loaded(store) => store,
        LoadOutcome::RecoveryRequired(_) => panic!("unexpected recovery"),
    }
}

fn domain() -> DomainState {
    DomainState::new(TimerConfig::default()).unwrap()
}

#[test]
fn first_and_consecutive_saves_preserve_domain_and_previous_exact_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut store = load(&location);
    let mut domain = domain();
    store.save(&domain, Timestamp(1_000)).unwrap();
    let first_bytes = fs::read(location.state_path()).unwrap();
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
    assert_eq!(store.saved_state().unwrap().original_bytes(), first_bytes);
    assert!(!location.backup_path().exists());
    assert!(store.pending_save().is_none());

    domain
        .apply(Command::Start(SessionKind::QuickStart), Timestamp(2_000))
        .unwrap();
    store.save(&domain, Timestamp(2_000)).unwrap();
    assert_eq!(store.saved_state().unwrap().save_generation(), 2);
    assert_eq!(fs::read(location.backup_path()).unwrap(), first_bytes);
    let second_bytes = fs::read(location.state_path()).unwrap();
    domain.apply(Command::CloseApp, Timestamp(2_000)).unwrap();
    store.save(&domain, Timestamp(3_000)).unwrap();
    assert_eq!(store.saved_state().unwrap().save_generation(), 3);
    assert_eq!(fs::read(location.backup_path()).unwrap(), second_bytes);
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
    for path in [location.state_path(), location.backup_path()] {
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }
    drop(store);
    let restored = load(&location);
    let saved = restored.saved_state().unwrap();
    assert_eq!(saved.domain(), &domain);
    assert_eq!(saved.save_generation(), 3);
    assert_eq!(saved.saved_at(), Timestamp(3_000));
}

#[test]
fn backup_copies_the_loaded_bytes_without_reencoding_or_adopting_other_temps() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    // Leading/trailing whitespace must be retained in the backup baseline.
    let mut bytes = b" \n".to_vec();
    bytes.extend_from_slice(VALID);
    bytes.extend_from_slice(b"\n ");
    fs::write(location.state_path(), &bytes).unwrap();
    fs::write(location.backup_path(), b"stale backup").unwrap();
    let orphan = directory.path().join("state.json.tmp-orphan");
    fs::write(&orphan, b"do not adopt or remove").unwrap();
    let mut store = load(&location);
    store.save(&domain(), Timestamp(2_000)).unwrap();
    assert_eq!(fs::read(location.backup_path()).unwrap(), bytes);
    assert_eq!(fs::read(orphan).unwrap(), b"do not adopt or remove");
    assert_eq!(store.saved_state().unwrap().save_generation(), 8);
}

#[test]
fn changed_or_deleted_primary_is_rejected_before_backup_or_candidate_creation() {
    let mut same_generation: Value = serde_json::from_slice(VALID).unwrap();
    same_generation["saved_at"] = json!("1970-01-01T00:00:02.000Z");
    for changed in [
        Some(serde_json::to_vec(&same_generation).unwrap()),
        Some(b"corrupt".to_vec()),
        None,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        fs::write(location.state_path(), VALID).unwrap();
        fs::write(location.backup_path(), b"keep backup").unwrap();
        let mut store = load(&location);
        if let Some(bytes) = &changed {
            fs::write(location.state_path(), bytes).unwrap();
        } else {
            fs::remove_file(location.state_path()).unwrap();
        }
        assert!(matches!(
            store.save(&domain(), Timestamp(2_000)),
            Err(SaveError::Conflict { .. })
        ));
        assert_eq!(fs::read(location.state_path()).ok(), changed);
        assert_eq!(fs::read(location.backup_path()).unwrap(), b"keep backup");
        assert_eq!(store.saved_state().unwrap().original_bytes(), VALID);
        assert!(store.pending_save().is_none());
    }
}

#[test]
fn new_store_does_not_overwrite_files_that_appear_after_load() {
    for name in ["state.json", "state.json.bak", "state.json.tmp-external"] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut store = load(&location);
        let path = directory.path().join(name);
        fs::write(&path, VALID).unwrap();
        assert!(matches!(
            store.save(&domain(), Timestamp(2_000)),
            Err(SaveError::Conflict { .. })
        ));
        assert_eq!(fs::read(path).unwrap(), VALID);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        assert!(store.pending_save().is_none());
        assert!(store.saved_state().is_none());
    }
}

#[test]
fn generation_overflow_and_invalid_timestamp_write_nothing() {
    for overflow in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let mut value: Value = serde_json::from_slice(VALID).unwrap();
        if overflow {
            value["save_generation"] = json!(u64::MAX);
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(location.state_path(), &bytes).unwrap();
        fs::write(location.backup_path(), b"keep backup").unwrap();
        let mut store = load(&location);
        let error = store
            .save(
                &domain(),
                if overflow {
                    Timestamp(2_000)
                } else {
                    Timestamp(u64::MAX)
                },
            )
            .unwrap_err();
        if overflow {
            assert!(matches!(error, SaveError::GenerationExhausted));
        } else {
            assert!(matches!(error, SaveError::InvalidCandidate(_)));
        }
        assert_eq!(fs::read(location.state_path()).unwrap(), bytes);
        assert_eq!(fs::read(location.backup_path()).unwrap(), b"keep backup");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
        assert!(store.pending_save().is_none());
    }
}

#[test]
fn primary_symlink_is_not_followed_and_backup_rename_failure_retains_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let mut store = load(&location);
    let target = directory.path().join("target");
    fs::rename(location.state_path(), &target).unwrap();
    symlink(&target, location.state_path()).unwrap();
    assert!(matches!(
        store.save(&domain(), Timestamp(2_000)),
        Err(SaveError::Read(_))
    ));
    assert_eq!(fs::read(&target).unwrap(), VALID);
    assert!(store.pending_save().is_none());
    fs::remove_file(location.state_path()).unwrap();
    fs::rename(target, location.state_path()).unwrap();

    fs::create_dir(location.backup_path()).unwrap();
    let error = store.save(&domain(), Timestamp(2_000)).unwrap_err();
    assert!(matches!(
        error,
        SaveError::Io {
            stage: SaveStage::RenameBackup,
            ..
        }
    ));
    assert!(!error.is_commit_uncertain());
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    assert!(location.backup_path().is_dir());
    assert_eq!(store.saved_state().unwrap().save_generation(), 7);
    assert_eq!(store.pending_save().unwrap().save_generation(), 8);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}
