#![cfg(windows)]

mod support;

use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::windows::fs::symlink_file,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use pomodoro_core::{DomainState, TimerConfig, Timestamp};
use pomodoro_platform::{LoadOutcome, SaveError, SaveStage, StorageLocation, StorageLockError};

const CHILD_DIRECTORY: &str = "POMODORO_WINDOWS_LOCK_DIRECTORY";
const CHILD_MODE: &str = "POMODORO_WINDOWS_LOCK_MODE";
const MARKER: &str = "WINDOWS_STORAGE_LOCK:";
const VALID: &[u8] = include_bytes!("fixtures/state_v1.json");

#[test]
#[ignore = "subprocess helper"]
fn lock_child() {
    let directory = std::env::var_os(CHILD_DIRECTORY).unwrap();
    let location = StorageLocation::at(directory.into());
    let mode = std::env::var(CHILD_MODE).unwrap();
    let lock = if mode == "lock" {
        let lock = location.lock().unwrap();
        println!("{MARKER}ACQUIRED");
        Some(lock)
    } else {
        assert_eq!(mode, "wait");
        println!("{MARKER}ALIVE");
        None
    };
    io::stdout().flush().unwrap();
    let mut byte = [0];
    io::stdin().read_exact(&mut byte).unwrap();
    drop(lock);
}

struct LockChild {
    process: Child,
    messages: Receiver<String>,
}

impl LockChild {
    fn spawn(directory: &Path, mode: &str) -> Self {
        let mut process = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lock_child", "--ignored", "--nocapture"])
            .env(CHILD_DIRECTORY, directory)
            .env(CHILD_MODE, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let output = process.stdout.take().unwrap();
        let (send, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in io::BufReader::new(output).lines().map_while(Result::ok) {
                if let Some((_, message)) = line.split_once(MARKER) {
                    let _ = send.send(message.to_owned());
                    break;
                }
            }
        });
        Self { process, messages }
    }

    fn expect_message(&self, message: &str) {
        assert_eq!(
            self.messages.recv_timeout(Duration::from_secs(10)).unwrap(),
            message
        );
    }
}

impl Drop for LockChild {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn product_lock_rejects_another_process_and_releases_after_termination() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut first = LockChild::spawn(directory.path(), "lock");
    first.expect_message("ACQUIRED");
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    first.process.kill().unwrap();
    assert!(!first.process.wait().unwrap().success());
    let _reacquired = location.lock().unwrap();
}

#[test]
fn lock_handle_is_not_inherited_by_a_child_process() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    let lock = location.clone().lock().unwrap();
    let mut child = LockChild::spawn(directory.path(), "wait");
    child.expect_message("ALIVE");
    drop(lock);
    let _reacquired = location.lock().unwrap();
    assert!(child.process.try_wait().unwrap().is_none());
}

#[test]
fn reparse_points_are_not_loaded_or_used_as_a_lock() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    let outside = directory.path().join("outside.json");
    fs::write(&outside, VALID).unwrap();
    symlink_file(&outside, location.state_path())
        .expect("Windows test runner must create symlinks");
    let error = location.clone().lock().unwrap().load().unwrap_err();
    assert!(error.to_string().contains("state.json"));
    assert_eq!(fs::read(&outside).unwrap(), VALID);
    fs::remove_file(location.state_path()).unwrap();
    // The preceding load created the dedicated lock file; replace it only
    // after that handle has been dropped by the failed load.
    fs::remove_file(location.lock_path()).unwrap();

    symlink_file(&outside, location.lock_path()).expect("Windows test runner must create symlinks");
    assert!(matches!(location.lock(), Err(StorageLockError::Io { .. })));
    assert_eq!(fs::read(&outside).unwrap(), VALID);
}

#[test]
fn failed_backup_replacement_keeps_the_original_and_fixed_candidate() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), VALID).unwrap();
    let LoadOutcome::Loaded(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("valid primary");
    };
    fs::create_dir(location.backup_path()).unwrap();
    let domain = DomainState::new(TimerConfig::default()).unwrap();
    let error = store.save(&domain, Timestamp(2_000)).unwrap_err();
    assert!(matches!(
        error.failure(),
        SaveError::Io {
            stage: SaveStage::RenameBackup,
            ..
        }
    ));
    assert_eq!(fs::read(location.state_path()).unwrap(), VALID);
    assert_eq!(store.saved_state().unwrap().save_generation(), 7);
    let candidate = store.pending_save().unwrap().encoded_bytes().to_vec();
    assert_eq!(store.pending_save().unwrap().save_generation(), 8);
    fs::remove_dir(location.backup_path()).unwrap();
    store.retry_pending().unwrap();
    assert_eq!(fs::read(location.state_path()).unwrap(), candidate);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
}
