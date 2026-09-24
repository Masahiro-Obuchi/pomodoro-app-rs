#![cfg(target_os = "linux")]

use std::{fs, process::Command};

const VALID: &[u8] =
    include_bytes!("../../../crates/pomodoro-platform/tests/fixtures/state_v1.json");

#[test]
fn unsupported_or_broken_load_exits_before_terminal_setup_without_overwriting() {
    for (original, backup, reason) in [
        (
            b"{ broken saved data, must remain unchanged".as_slice(),
            false,
            "invalid saved state",
        ),
        (
            br#"{"legacy":true}"#,
            true,
            "unversioned saved state is unsupported",
        ),
        (
            br#"{"schema_version":999}"#,
            true,
            "unsupported schema version 999",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let state_directory = directory.path().join("pomodoro-app-rs");
        fs::create_dir_all(&state_directory).unwrap();
        let path = state_directory.join("state.json");
        fs::write(&path, original).unwrap();
        if backup {
            // An unsupported primary must not silently downgrade to a valid V1
            // backup, or even offer that recovery as if the primary were corrupt.
            fs::write(state_directory.join("state.json.bak"), VALID).unwrap();
        }

        let output = Command::new(env!("CARGO_BIN_EXE_pomodoro-tui"))
            .env("XDG_STATE_HOME", directory.path())
            .output()
            .unwrap();

        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("startup load failed"), "{error}");
        assert!(error.contains(reason), "{error}");
        assert!(output.stdout.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        if backup {
            assert_eq!(
                fs::read(state_directory.join("state.json.bak")).unwrap(),
                VALID
            );
        }
        let entries: Vec<_> = fs::read_dir(&state_directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(entries.iter().all(|name| name == "state.json"
            || name == "state.lock"
            || (backup && name == "state.json.bak")));
        assert!(entries.iter().any(|name| name == "state.json"));
    }
}
