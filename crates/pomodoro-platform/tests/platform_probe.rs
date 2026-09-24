//! W0 native-filesystem probes. CI output records observed OS behavior; these
//! probes do not claim that any platform satisfies the V1 durability contract.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use fs4::FileExt;

#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(windows)]
use std::os::windows::fs::{OpenOptionsExt, symlink_file};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

const LOCK_PATH: &str = "POMODORO_PROBE_LOCK_PATH";
const LOCK_HOLD: &str = "POMODORO_PROBE_LOCK_HOLD";
const MARKER: &str = "PLATFORM_PROBE:";

#[test]
#[ignore = "subprocess helper"]
fn lock_child() {
    let path = std::env::var_os(LOCK_PATH).expect("lock path");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let result = FileExt::try_lock(&file);
    println!("{MARKER}{result:?}");
    io::stdout().flush().unwrap();
    if result.is_ok() && std::env::var_os(LOCK_HOLD).is_some() {
        let mut input = [0];
        io::stdin().read_exact(&mut input).unwrap();
    }
}

#[test]
fn probe_nonblocking_file_lock_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.lock");
    let first = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    FileExt::try_lock(&first).expect("first process takes the lock");
    let busy = child_lock_result(&path);
    assert!(busy.contains("WouldBlock"), "unexpected contention: {busy}");
    drop(first);
    let released = child_lock_result(&path);
    assert_eq!(released, "Ok(())", "lock did not release: {released}");
}

fn child_lock_result(path: &Path) -> String {
    let (mut child, receive) = spawn_lock_child(path, false);
    let result = receive.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "child failed: {status}");
    result.expect("child lock result timed out")
}

fn spawn_lock_child(path: &Path, hold: bool) -> (std::process::Child, mpsc::Receiver<String>) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_child", "--ignored", "--nocapture"])
        .env(LOCK_PATH, path)
        .stdin(if hold { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .envs(hold.then_some((LOCK_HOLD, "1")))
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reported = false;
        for line in io::BufReader::new(output).lines().map_while(Result::ok) {
            if !reported {
                if let Some((_, result)) = line.split_once(MARKER) {
                    let _ = send.send(result.to_owned());
                    reported = true;
                }
            }
        }
    });
    (child, receive)
}

#[test]
fn probe_abrupt_process_exit_releases_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.lock");
    let _file = File::create(&path).unwrap();
    let (mut child, receive) = spawn_lock_child(&path, true);
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(10)).unwrap(),
        "Ok(())"
    );
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let reopened = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    FileExt::try_lock(&reopened).expect("lock remained held after abrupt exit");
}

#[test]
fn probe_replace_existing_file_and_directory_sync() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json");
    let replacement = dir.path().join("state.json.tmp");
    fs::write(&target, b"old").unwrap();
    fs::write(&replacement, b"new").unwrap();
    OpenOptions::new()
        .write(true)
        .open(&replacement)
        .unwrap()
        .sync_all()
        .unwrap();

    let old_handle = File::open(&target).unwrap();
    let open_target_result = fs::rename(&replacement, &target);
    println!("{MARKER}replace_with_open_target={open_target_result:?}");
    if open_target_result.is_err() {
        assert_eq!(fs::read(&target).unwrap(), b"old");
        drop(old_handle);
        fs::rename(&replacement, &target).unwrap();
    }
    assert_eq!(fs::read(&target).unwrap(), b"new");

    let directory_sync = File::open(dir.path()).and_then(|file| file.sync_all());
    println!("{MARKER}std_directory_sync={directory_sync:?}");
}

#[test]
fn probe_storage_base_directories() {
    let base = directories::BaseDirs::new().unwrap();
    println!("{MARKER}state_dir={:?}", base.state_dir());
    println!("{MARKER}data_local_dir={:?}", base.data_local_dir());
}

#[cfg(windows)]
#[test]
fn probe_windows_directory_handle_sync() {
    let dir = tempfile::tempdir().unwrap();
    for (name, read, write) in [("read", true, false), ("write", false, true)] {
        let result = OpenOptions::new()
            .read(read)
            .write(write)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir.path())
            .and_then(|file| file.sync_all());
        println!("{MARKER}backup_semantics_directory_sync_{name}={result:?}");
    }
}

#[test]
fn probe_symlink_read_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("outside.json");
    let link = dir.path().join("state.json");
    fs::write(&target, b"outside").unwrap();
    #[cfg(unix)]
    let created = symlink(&target, &link);
    #[cfg(windows)]
    let created = symlink_file(&target, &link);
    if let Err(error) = created {
        println!("{MARKER}symlink_creation_unavailable={error}");
        return;
    }
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&link).unwrap(), b"outside");

    #[cfg(windows)]
    {
        let no_follow = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&link)
            .and_then(|file| file.metadata());
        println!("{MARKER}open_reparse_point={no_follow:?}");
    }
}

#[test]
fn probe_open_handle_detects_replaced_path() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json.tmp");
    let replacement = dir.path().join("replacement.tmp");
    fs::write(&target, b"original").unwrap();
    fs::write(&replacement, b"other").unwrap();
    let owned = same_file::Handle::from_file(File::open(&target).unwrap()).unwrap();
    assert_eq!(owned, same_file::Handle::from_path(&target).unwrap());
    fs::rename(&replacement, &target).unwrap();
    assert_ne!(owned, same_file::Handle::from_path(&target).unwrap());
    println!("{MARKER}open_handle_identity_detects_replacement=true");
}
