use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn failed_load_exits_before_terminal_setup_and_does_not_overwrite_the_file() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("pomodoro-startup-{}-{suffix}", std::process::id()));
    let state_directory = directory.join("pomodoro-app-rs");
    fs::create_dir_all(&state_directory).unwrap();
    let path = state_directory.join("state.json");
    let original = b"{ broken saved data, must remain unchanged";
    fs::write(&path, original).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_pomodoro-tui"))
        .env("XDG_STATE_HOME", &directory)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(&state_directory).unwrap().count(), 1);
    fs::remove_dir_all(&directory).unwrap();
}
