#![cfg(target_os = "linux")]

use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use pomodoro_platform::{NativeStorage, StorageLocation, StorageLockError};

const MODE: &str = "POMODORO_LOCK_TEST_MODE";
const DIRECTORY: &str = "POMODORO_LOCK_TEST_DIRECTORY";
const MESSAGE: &str = "LOCK_PROBE:";
const TIMEOUT: Duration = Duration::from_secs(10);

// Run in a fresh process so contention and exec inheritance exercise the kernel,
// without mutating environment variables or the working directory of test peers.
#[test]
#[ignore = "subprocess helper invoked by the integration tests"]
fn lock_probe() {
    let mode = std::env::var(MODE).unwrap();
    let directory = std::env::var_os(DIRECTORY).unwrap();
    let location = StorageLocation::at(directory.into());
    let guard = match mode.as_str() {
        "lock" | "relative" => {
            let selected = if mode == "relative" {
                StorageLocation::at(".".into())
            } else {
                location.clone()
            };
            match selected.lock() {
                Ok(guard) => {
                    assert_eq!(
                        guard.location().directory(),
                        fs::canonicalize(location.directory()).unwrap()
                    );
                    println!("{MESSAGE}ACQUIRED");
                    Some(guard)
                }
                Err(StorageLockError::InUse { path }) => {
                    assert_eq!(
                        path,
                        fs::canonicalize(location.directory())
                            .unwrap()
                            .join("state.lock")
                    );
                    println!("{MESSAGE}BUSY");
                    None
                }
                Err(error) => panic!("unexpected lock error: {error}"),
            }
        }
        "discover" => {
            let discovered = StorageLocation::discover().unwrap();
            assert_eq!(
                discovered.directory(),
                location.directory().join("pomodoro-app-rs")
            );
            assert_eq!(
                discovered.state_path(),
                NativeStorage::discover().unwrap().state_path()
            );
            println!("{MESSAGE}DISCOVERED");
            None
        }
        "wait" => {
            println!("{MESSAGE}ALIVE");
            None
        }
        "inheritance" => {
            // Isolate ownership from the parallel test runner. Otherwise an
            // unrelated test's fork can temporarily inherit this descriptor
            // before exec closes it, making immediate reacquisition flaky.
            let lock = location.clone().lock().unwrap();
            let mut child = Probe::spawn("wait", location.directory());
            child.expect("ALIVE");
            drop(lock);
            // Do not retry: missing CLOEXEC must fail while the child is alive.
            let reacquired = location.lock().unwrap();
            assert!(child.child.try_wait().unwrap().is_none());
            child.finish();
            println!("{MESSAGE}VERIFIED");
            Some(reacquired)
        }
        _ => panic!("unknown helper mode"),
    };
    io::stdout().flush().unwrap();
    // IPC handshake keeps the handle/process alive until the parent is finished.
    let mut input = [0];
    io::stdin().read_exact(&mut input).unwrap();
    drop(guard);
}

struct Probe {
    child: Child,
    messages: Receiver<String>,
}

impl Probe {
    fn spawn(mode: &str, directory: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lock_probe", "--ignored", "--nocapture"])
            .env(MODE, mode)
            .env(DIRECTORY, directory)
            .env("XDG_STATE_HOME", directory)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some((_, message)) = line.split_once(MESSAGE) {
                    if sender.send(message.to_owned()).is_err() {
                        break;
                    }
                }
            }
        });
        Self { child, messages }
    }

    fn expect(&self, message: &str) {
        // A mistaken blocking flock fails promptly instead of hanging the suite.
        assert_eq!(
            self.messages
                .recv_timeout(TIMEOUT)
                .expect("helper timed out or exited"),
            message
        );
    }

    fn finish(mut self) {
        self.child.stdin.as_mut().unwrap().write_all(b"x").unwrap();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "helper failed: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "helper failed to exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn kill(&mut self) {
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        // Also reap a helper when an assertion fails, before TempDir cleanup.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn second_process_is_rejected_and_normal_exit_releases_the_lock() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.lock_path(), b"existing lock file").unwrap();
    let inode = fs::metadata(location.lock_path()).unwrap().ino();
    let first = Probe::spawn("lock", directory.path());
    first.expect("ACQUIRED");
    let second = Probe::spawn("lock", directory.path());
    second.expect("BUSY");
    second.finish();
    first.finish();
    assert_eq!(
        fs::read(location.lock_path()).unwrap(),
        b"existing lock file"
    );
    assert_eq!(fs::metadata(location.lock_path()).unwrap().ino(), inode);
    let _lock = location.lock().unwrap();
}

#[test]
fn abrupt_process_exit_releases_the_lock_without_deleting_its_file() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    let mut first = Probe::spawn("lock", directory.path());
    first.expect("ACQUIRED");
    let inode = fs::metadata(location.lock_path()).unwrap().ino();
    first.kill();
    let _lock = location.clone().lock().unwrap();
    assert_eq!(fs::metadata(location.lock_path()).unwrap().ino(), inode);
}

#[test]
fn exec_child_does_not_keep_its_parents_lock_alive() {
    let directory = tempfile::tempdir().unwrap();
    let probe = Probe::spawn("inheritance", directory.path());
    probe.expect("VERIFIED");
    probe.finish();
}

#[test]
fn separate_directories_can_be_locked_concurrently() {
    let first_directory = tempfile::tempdir().unwrap();
    let second_directory = tempfile::tempdir().unwrap();
    let first = Probe::spawn("lock", first_directory.path());
    first.expect("ACQUIRED");
    let second = Probe::spawn("lock", second_directory.path());
    second.expect("ACQUIRED");
    second.finish();
    first.finish();
}

#[test]
fn state_rename_does_not_release_the_lock_or_touch_the_backup() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().to_owned());
    fs::write(location.state_path(), b"broken existing JSON").unwrap();
    fs::write(location.backup_path(), b"backup bytes").unwrap();
    let lock = location.clone().lock().unwrap();
    assert_eq!(
        fs::read(location.state_path()).unwrap(),
        b"broken existing JSON"
    );
    let replacement = directory.path().join("replacement");
    fs::write(&replacement, b"new state bytes").unwrap();
    fs::rename(replacement, location.state_path()).unwrap();
    let second = Probe::spawn("lock", directory.path());
    second.expect("BUSY");
    second.finish();
    assert_eq!(fs::read(location.backup_path()).unwrap(), b"backup bytes");
    drop(lock);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}

#[test]
fn relative_paths_and_directory_aliases_share_the_same_lock() {
    let directory = tempfile::tempdir().unwrap();
    let first = Probe::spawn("relative", directory.path());
    first.expect("ACQUIRED");
    let alias_parent = tempfile::tempdir().unwrap();
    let alias = alias_parent.path().join("alias");
    symlink(directory.path(), &alias).unwrap();
    let second = Probe::spawn("lock", &alias);
    second.expect("BUSY");
    second.finish();
    first.finish();
}

#[test]
fn creates_only_the_directory_and_private_lock_file() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("nested/store"));
    assert!(!location.directory().exists());
    let lock = location.clone().lock().unwrap();
    assert_eq!(fs::read_dir(location.directory()).unwrap().count(), 1);
    assert!(!location.state_path().exists());
    assert!(!location.backup_path().exists());
    let permissions = fs::metadata(location.lock_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(permissions & 0o077, 0);
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    drop(lock);
    assert!(location.lock_path().exists());
}

#[test]
fn discovery_matches_legacy_path_without_creating_files() {
    let directory = tempfile::tempdir().unwrap();
    let probe = Probe::spawn("discover", directory.path());
    probe.expect("DISCOVERED");
    probe.finish();
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn io_errors_are_distinct_from_contention_and_do_not_replace_files() {
    let directory = tempfile::tempdir().unwrap();
    let not_a_directory = directory.path().join("ordinary-file");
    fs::write(&not_a_directory, b"keep").unwrap();
    let error = StorageLocation::at(not_a_directory.clone())
        .lock()
        .unwrap_err();
    assert!(matches!(error, StorageLockError::Io { .. }));
    assert_eq!(fs::read(&not_a_directory).unwrap(), b"keep");
    let location = StorageLocation::at(directory.path().to_owned());
    symlink(&not_a_directory, location.lock_path()).unwrap();
    let error = location.clone().lock().unwrap_err();
    assert!(matches!(error, StorageLockError::Io { .. }));
    assert!(error.to_string().contains("state.lock"));
    assert_eq!(fs::read(&not_a_directory).unwrap(), b"keep");
    assert!(!location.state_path().exists());
}
