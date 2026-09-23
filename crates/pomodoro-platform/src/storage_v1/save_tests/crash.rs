//! Subprocess integration checks colocated with the private I/O hooks. No
//! production environment switch or public failure-injection API is needed.

use super::*;
use std::{
    io::{BufRead, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const PROBE: &str = "storage_v1::atomic_save::tests::crash::crash_probe";
const DIRECTORY: &str = "POMODORO_CRASH_DIRECTORY";
const STAGE: &str = "POMODORO_CRASH_STAGE";
const REACHED: &str = "SAVE_BOUNDARY_REACHED";

fn await_kill() {
    println!("{REACHED}");
    std::io::stdout().flush().unwrap();
    let mut input = [0];
    std::io::stdin().read_exact(&mut input).unwrap();
    panic!("parent must kill the child without running destructors");
}

#[test]
#[ignore = "subprocess helper killed at save boundaries by its parent"]
fn crash_probe() {
    let location = StorageLocation::at(std::env::var_os(DIRECTORY).unwrap().into());
    let stop_at = std::env::var(STAGE).unwrap();
    let mut store = load(&location);
    store
        .save_with_hook(&domain(), Timestamp(2_000), &mut |stage, _| {
            if format!("{stage:?}") == stop_at {
                await_kill();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(stop_at, "Committed");
    await_kill();
}

struct CrashChild(Child);

impl CrashChild {
    fn stop(location: &StorageLocation, stage: &str) {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", PROBE, "--ignored", "--nocapture"])
            .env(DIRECTORY, location.directory())
            .env(STAGE, stage)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut child = Self(child);
        let stdout = child.0.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                if line.unwrap().contains(REACHED) {
                    let _ = sender.send(());
                    break;
                }
            }
        });
        receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("did not reach {stage}: {error}"));
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
    }
}

impl Drop for CrashChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Fork can transiently inherit another test thread's flock descriptors before
// exec closes them. Run this suite alone so unrelated drop/reopen assertions
// cannot race that interval; do not weaken the production nonblocking lock.
#[test]
#[ignore = "run alone with cargo test -p pomodoro-platform --lib crash_boundaries_in_isolated_process -- --ignored"]
fn crash_boundaries_in_isolated_process() {
    process_stop_at_each_save_boundary_keeps_a_whole_old_or_new_primary();
    interrupted_first_save_never_initializes_over_a_leftover_temporary_file();
}

fn process_stop_at_each_save_boundary_keeps_a_whole_old_or_new_primary() {
    let stages = std::iter::once(SaveStage::SyncStorageAncestry)
        .chain(SAVE_STAGES)
        .map(|stage| format!("{stage:?}"))
        .chain(std::iter::once("Committed".into()));
    for stage in stages {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        fs::write(location.state_path(), VALID).unwrap();
        fs::write(location.backup_path(), VALID).unwrap();
        CrashChild::stop(&location, &stage);
        // Kernel locks are released by process death; leftover temp files cannot
        // override a valid primary. This is not a power-loss durability test.
        let store = load(&location);
        let saved = store.saved_state().unwrap();
        if stage == "SyncPrimaryDirectory" || stage == "Committed" {
            assert_eq!(saved.domain(), &domain(), "{stage}");
            assert_eq!(saved.save_generation(), 8, "{stage}");
            assert_eq!(saved.saved_at(), Timestamp(2_000), "{stage}");
        } else {
            assert_eq!(saved.original_bytes(), VALID, "{stage}");
        }
        assert_eq!(fs::read(location.backup_path()).unwrap(), VALID, "{stage}");
    }
}

fn interrupted_first_save_never_initializes_over_a_leftover_temporary_file() {
    for stage in [
        "SyncStorageAncestry",
        "CreatePrimaryTemp",
        "WritePrimaryTemp",
        "SyncPrimaryTemp",
        "RenamePrimary",
        "SyncPrimaryDirectory",
        "Committed",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        CrashChild::stop(&location, stage);
        let result = location.clone().lock().unwrap().load();
        match stage {
            "SyncStorageAncestry" | "CreatePrimaryTemp" => {
                assert!(matches!(result, Ok(LoadOutcome::New(_))), "{stage}");
            }
            "SyncPrimaryDirectory" | "Committed" => {
                let Ok(LoadOutcome::Loaded(store)) = result else {
                    panic!("expected a complete first save at {stage}");
                };
                assert_eq!(store.saved_state().unwrap().domain(), &domain());
                assert_eq!(store.saved_state().unwrap().save_generation(), 1);
            }
            _ => {
                let error = result.unwrap_err();
                assert!(!error.remnants().is_empty(), "{stage}");
                assert!(!location.state_path().exists(), "{stage}");
                for path in error.remnants() {
                    assert!(path.is_file(), "load must preserve temporary files");
                }
            }
        }
    }
}
