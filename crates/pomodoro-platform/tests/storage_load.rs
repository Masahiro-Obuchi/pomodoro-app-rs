#![cfg(target_os = "linux")]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::Path,
};

use pomodoro_core::{DomainState, ProgressState, TimerConfig, TimerState, Timestamp};
use pomodoro_platform::{LoadOutcome, LoadProblem, StorageLocation, StorageLockError};
use serde_json::{Value, json};

const VALID: &[u8] = include_bytes!("fixtures/state_v1.json");

fn location(directory: &tempfile::TempDir) -> StorageLocation {
    StorageLocation::at(directory.path().to_owned())
}

fn files(directory: &Path) -> BTreeMap<OsString, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name() != "state.lock")
        .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
        .collect()
}

fn assert_locked(location: &StorageLocation) {
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
}

#[test]
fn empty_directory_is_new_and_does_not_create_default_state() {
    let directory = tempfile::tempdir().unwrap();
    let location = location(&directory);
    let LoadOutcome::New(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected new store");
    };
    assert!(store.saved_state().is_none());
    assert_eq!(store.location(), &location);
    assert!(files(directory.path()).is_empty());
    assert_locked(&location);
    drop(store);
    let LoadOutcome::New(store) = location.lock().unwrap().load().unwrap() else {
        panic!("a leftover state.lock must not prevent initialization");
    };
    assert!(store.saved_state().is_none());
}

#[test]
fn valid_primary_keeps_exact_bytes_and_metadata_under_the_lock() {
    let directory = tempfile::tempdir().unwrap();
    let location = location(&directory);
    fs::write(location.state_path(), VALID).unwrap();
    let before = files(directory.path());
    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected loaded V1");
    };
    let saved = store.saved_state().unwrap();
    assert_eq!(
        saved.domain(),
        &DomainState::new(TimerConfig::default()).unwrap()
    );
    assert_eq!(saved.save_generation(), 7);
    assert_eq!(saved.saved_at(), Timestamp(1_000));
    assert_eq!(saved.original_bytes(), VALID);
    assert_eq!(files(directory.path()), before);
    assert_locked(&location);
    drop(store);
    let _lock = location.lock().unwrap();
}

#[test]
fn loading_running_snapshot_does_not_apply_restore_or_generate_events() {
    let directory = tempfile::tempdir().unwrap();
    let location = location(&directory);
    let mut value: Value = serde_json::from_slice(VALID).unwrap();
    value["id_allocators"]["next_session_id"] = json!(2);
    value["snapshot"]["state"] = json!({
        "status": "active",
        "session": {
            "id": 1, "kind": "focus", "current_task": "read",
            "planned_duration_ms": 1_500_000, "elapsed_ms": 500,
            "started_at": "1970-01-01T00:00:00.000Z",
            "continued_from_quick_start": null, "end": null
        },
        "timer": {"status":"running", "run": {
            "started_at":"1970-01-01T00:00:00.000Z", "elapsed_ms_at_start":0,
            "last_confirmed_at":"1970-01-01T00:00:00.500Z", "time_uncertainty":null
        }}
    });
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(location.state_path(), &bytes).unwrap();
    let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
        panic!("expected loaded V1");
    };
    let saved = store.saved_state().unwrap();
    assert!(matches!(
        saved.domain().snapshot().state,
        ProgressState::Active {
            timer: TimerState::Running { .. },
            ..
        }
    ));
    assert!(saved.domain().history().events.is_empty());
    assert_eq!(saved.original_bytes(), bytes);
}

#[test]
fn valid_primary_wins_over_backups_and_newer_or_broken_temps() {
    for backup in [VALID, b"broken backup"] {
        let directory = tempfile::tempdir().unwrap();
        let location = location(&directory);
        fs::write(location.state_path(), VALID).unwrap();
        fs::write(location.backup_path(), backup).unwrap();
        let mut newer: Value = serde_json::from_slice(VALID).unwrap();
        newer["save_generation"] = json!(999);
        fs::write(
            directory.path().join("state.json.tmp-newer"),
            serde_json::to_vec(&newer).unwrap(),
        )
        .unwrap();
        fs::write(directory.path().join("state.json.tmp-broken"), b"{").unwrap();
        let before = files(directory.path());
        let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
            panic!("expected primary");
        };
        assert_eq!(store.saved_state().unwrap().save_generation(), 7);
        assert_eq!(files(directory.path()), before);
    }
}

#[test]
fn missing_or_corrupt_primary_offers_only_the_valid_backup() {
    for primary in [
        None,
        Some(b"{ broken data".as_slice()),
        Some(b"[1,]".as_slice()),
        Some(b"\"unterminated".as_slice()),
        Some(b"[]{}".as_slice()),
        Some(b"true false".as_slice()),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = location(&directory);
        if let Some(bytes) = primary {
            fs::write(location.state_path(), bytes).unwrap();
        }
        fs::write(location.backup_path(), VALID).unwrap();
        fs::write(directory.path().join("state.json.tmp-other"), b"candidate").unwrap();
        let before = files(directory.path());
        let LoadOutcome::RecoveryRequired(candidate) =
            location.clone().lock().unwrap().load().unwrap()
        else {
            panic!("expected explicit recovery candidate");
        };
        assert_eq!(candidate.location(), &location);
        assert_eq!(candidate.backup().original_bytes(), VALID);
        assert_eq!(candidate.backup().save_generation(), 7);
        assert_eq!(candidate.backup().saved_at(), Timestamp(1_000));
        assert_eq!(candidate.original_primary_bytes(), primary);
        assert!(match candidate.primary_problem() {
            LoadProblem::Missing { .. } => primary.is_none(),
            LoadProblem::Invalid { .. } => primary.is_some(),
            _ => false,
        });
        assert_locked(&location);
        // Rejecting recovery is just dropping the candidate, with no disk writes.
        drop(candidate);
        assert_eq!(files(directory.path()), before);
        let _lock = location.lock().unwrap();
    }
}

#[test]
fn old_or_future_primary_is_not_downgraded_to_a_valid_backup() {
    for (primary, version) in [
        (br#"{"timer":{},"history":{}}"#.as_slice(), None),
        (b"[]".as_slice(), None),
        (br#""legacy""#.as_slice(), None),
        (b"null".as_slice(), None),
        (b"true".as_slice(), None),
        (b"false".as_slice(), None),
        (b"123".as_slice(), None),
        (b"-4.5".as_slice(), None),
        (br#"[{"schema_version":1}]"#.as_slice(), None),
        (br#"{"schema_version":2}"#.as_slice(), Some(2)),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = location(&directory);
        fs::write(location.state_path(), primary).unwrap();
        fs::write(location.backup_path(), VALID).unwrap();
        let before = files(directory.path());
        let error = location.clone().lock().unwrap().load().unwrap_err();
        assert!(
            matches!(error.primary_problem(), LoadProblem::Unsupported { schema_version, .. } if *schema_version == version)
        );
        assert!(error.backup_problem().is_none());
        assert_eq!(files(directory.path()), before);
        let _lock = location.lock().unwrap();
    }
}

#[test]
fn malformed_schema_and_domain_data_never_become_new_state() {
    let valid: Value = serde_json::from_slice(VALID).unwrap();
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("snapshot");
    let mut reference = valid.clone();
    reference["snapshot"]["state"] = json!({"status":"awaiting_quick_start_decision", "quick_start_session_id":99, "current_task":null});
    let mut allocator = valid;
    allocator["id_allocators"]["next_event_sequence"] = json!(2);
    for bytes in [
        b"{".to_vec(),
        serde_json::to_vec(&missing).unwrap(),
        serde_json::to_vec(&reference).unwrap(),
        serde_json::to_vec(&allocator).unwrap(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = location(&directory);
        fs::write(location.state_path(), &bytes).unwrap();
        let before = files(directory.path());
        let error = location.lock().unwrap().load().unwrap_err();
        assert!(matches!(
            error.primary_problem(),
            LoadProblem::Invalid { .. }
        ));
        assert_eq!(files(directory.path()), before);
    }
}

#[test]
fn bad_backups_preserve_both_diagnostics_without_initializing() {
    for primary in [None, Some(b"{ broken primary".as_slice())] {
        for backup in [
            b"{ broken backup".as_slice(),
            br#"{"schema_version":2}"#,
            b"{}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let location = location(&directory);
            if let Some(bytes) = primary {
                fs::write(location.state_path(), bytes).unwrap();
            }
            fs::write(location.backup_path(), backup).unwrap();
            let before = files(directory.path());
            let error = location.lock().unwrap().load().unwrap_err();
            assert!(matches!(
                error.primary_problem(),
                LoadProblem::Missing { .. } | LoadProblem::Invalid { .. }
            ));
            assert!(matches!(
                error.backup_problem(),
                Some(LoadProblem::Invalid { .. } | LoadProblem::Unsupported { .. })
            ));
            assert!(error.to_string().contains("state.json.bak"));
            assert_eq!(files(directory.path()), before);
        }
    }
}

#[test]
fn temporary_quarantined_and_unrecognized_entries_prevent_initialization() {
    for name in [
        OsString::from("state.json.tmp-1"),
        "state.json.bak.tmp-1".into(),
        "state.json.quarantine-1".into(),
        "unknown-file".into(),
        OsString::from_vec(vec![0xff]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = location(&directory);
        let path = directory.path().join(name);
        // Even a completely valid temp is not an authorized recovery source.
        fs::write(&path, VALID).unwrap();
        let before = files(directory.path());
        let error = location.lock().unwrap().load().unwrap_err();
        assert!(matches!(
            error.primary_problem(),
            LoadProblem::Missing { .. }
        ));
        assert_eq!(error.remnants(), &[path]);
        assert_eq!(files(directory.path()), before);
    }
}

#[test]
fn io_errors_and_dangling_symlinks_are_not_missing_files() {
    for backup in [false, true] {
        for is_link in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let location = location(&directory);
            let path = if backup {
                location.backup_path()
            } else {
                location.state_path()
            };
            if is_link {
                symlink(directory.path().join("missing-target"), &path).unwrap();
            } else {
                fs::create_dir(&path).unwrap();
            }
            if !backup {
                fs::write(location.backup_path(), VALID).unwrap();
            }
            let error = location.lock().unwrap().load().unwrap_err();
            let problem = if backup {
                error.backup_problem().unwrap()
            } else {
                error.primary_problem()
            };
            assert!(matches!(problem, LoadProblem::Io { path: reported, .. } if reported == &path));
            assert!(fs::symlink_metadata(path).is_ok());
        }
    }
}

#[test]
fn permission_denied_is_not_a_new_store_or_recovery_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let location = location(&directory);
    fs::write(location.state_path(), VALID).unwrap();
    fs::write(location.backup_path(), VALID).unwrap();
    fs::set_permissions(location.state_path(), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(location.state_path()).is_ok() {
        // Root/CAP_DAC_OVERRIDE bypasses the permission we are testing.
        eprintln!("permission-denied case requires an unprivileged process");
        return;
    }
    let error = location.clone().lock().unwrap().load().unwrap_err();
    assert!(
        matches!(error.primary_problem(), LoadProblem::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(error.backup_problem().is_none());
    fs::set_permissions(location.state_path(), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}

#[test]
fn unreadable_directory_prevents_the_new_store_check() {
    let directory = tempfile::tempdir().unwrap();
    let location = location(&directory);
    let locked = location.clone().lock().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o300)).unwrap();
    let enforces_permissions = fs::read_dir(directory.path()).is_err();
    let result = locked.load();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if enforces_permissions {
        assert!(
            matches!(result.unwrap_err().primary_problem(), LoadProblem::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied)
        );
    }
    assert!(!location.state_path().exists());
}
