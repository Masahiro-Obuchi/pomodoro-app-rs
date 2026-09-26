#![cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use pomodoro_core::{ProgressState, TimerState};
use pomodoro_platform::{LoadOutcome, StorageLocation};
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

const TIMEOUT: Duration = Duration::from_secs(15);
const VALID: &[u8] =
    include_bytes!("../../../crates/pomodoro-platform/tests/fixtures/state_v1.json");

fn executable() -> PathBuf {
    std::env::var_os("POMODORO_TUI_TEST_BIN").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_pomodoro-tui")),
        PathBuf::from,
    )
}

#[test]
fn empty_state_directory_override_stops_before_terminal_setup() {
    let output = Command::new(executable())
        .env("POMODORO_STATE_DIR", "")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("POMODORO_STATE_DIR cannot be empty"));
}

struct Tui {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    received: Vec<u8>,
    screen: vt100::Parser,
}

impl Tui {
    fn spawn(directory: &Path, width: u16, height: u16) -> Self {
        let pty = NativePtySystem::default()
            .openpty(PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(executable());
        command.env("POMODORO_STATE_DIR", directory);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        command.env("TERM", "xterm-256color");
        let child = pty.slave.spawn_command(command).unwrap();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().unwrap();
        let writer = pty.master.take_writer().unwrap();
        #[cfg(target_os = "windows")]
        let mut writer = writer;
        #[cfg(target_os = "windows")]
        {
            // portable-pty requests cursor inheritance from ConPTY. Supply a
            // cursor-position response even when the CI host has no terminal.
            writer.write_all(b"\x1b[1;1R").unwrap();
            writer.flush().unwrap();
        }
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            master: pty.master,
            writer,
            output,
            received: Vec::new(),
            screen: vt100::Parser::new(height, width, 0),
        }
    }

    fn expect(&mut self, phrase: &str) {
        self.expect_all(&[phrase]);
    }

    fn expect_all(&mut self, phrases: &[&str]) {
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| {
                    !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘')
                })
                .collect::<String>()
        };
        let expected: Vec<_> = phrases.iter().map(|phrase| compact(phrase)).collect();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let emitted = without_csi(&self.received);
            let text = compact(&emitted);
            if expected.iter().all(|phrase| text.contains(phrase)) {
                self.received.clear();
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => {
                    self.screen.process(&bytes);
                    self.received.extend(bytes);
                }
                Err(error) => panic!(
                    "waiting for {phrases:?}: {error}; child: {:?}; output: {emitted}",
                    self.child.try_wait().unwrap()
                ),
            }
        }
    }

    fn expect_screen(&mut self, phrase: &str) {
        self.expect_screen_all(&[phrase]);
    }

    fn expect_screen_all(&mut self, phrases: &[&str]) {
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| {
                    !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘')
                })
                .collect::<String>()
        };
        let expected: Vec<_> = phrases.iter().map(|phrase| compact(phrase)).collect();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let screen = self.screen.screen().contents();
            let visible = compact(&screen);
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "waiting for current screen {phrases:?}; screen: {screen:?}"
            );
            if expected.iter().all(|phrase| visible.contains(phrase)) {
                // Apply more than one 100 ms redraw before accepting a snapshot.
                let settle_until = Instant::now() + Duration::from_millis(250);
                while let Some(wait) = settle_until.checked_duration_since(Instant::now()) {
                    match self.output.recv_timeout(wait) {
                        Ok(bytes) => self.screen.process(&bytes),
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(error) => panic!(
                            "waiting for current screen {phrases:?}: {error}; screen: {screen:?}"
                        ),
                    }
                }
                let settled = self.screen.screen().contents();
                if expected
                    .iter()
                    .all(|phrase| compact(&settled).contains(phrase))
                {
                    return;
                }
                continue;
            }
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.screen.process(&bytes),
                Err(error) => panic!(
                    "waiting for current screen {phrases:?}: {error}; child: {:?}; screen: {screen:?}",
                    self.child.try_wait().unwrap()
                ),
            }
        }
    }

    fn assert_screen_absent(&self, phrase: &str) {
        let screen = self.screen.screen().contents();
        assert!(
            !screen.contains(phrase),
            "unexpected {phrase:?} on current screen: {screen:?}"
        );
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.master
            .resize(PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        self.screen.screen_mut().set_size(height, width);
    }

    fn finish(mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "PTY helper did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn without_csi(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        } else if ch == '\u{8}' {
            result.pop();
        } else {
            result.push(ch);
        }
    }
    result
}

#[test]
fn native_pty_can_edit_start_resize_and_restore_saved_state() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("state"));
    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("t: Edit task");
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    tui.send(b"\x1b[200~");
    tui.send("原稿を書く".as_bytes());
    tui.send(b"\x1b[201~");
    tui.expect("原稿を書く");
    tui.send(b"\r");
    tui.expect("Task saved");
    tui.send(b" ");
    tui.expect("Running");
    tui.resize(24, 17);
    tui.expect("d: Report distraction");
    tui.send(b"d");
    tui.expect("Distraction saved");
    tui.send(b" ");
    tui.send(b"q");
    assert!(tui.finish().success());

    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected saved V1 state");
    };
    let state = store.saved_state().unwrap().domain();
    assert_eq!(state.reflection().unwrap().distractions, 1);
    assert_eq!(state.reflection().unwrap().returns, 1);
    let ProgressState::Active { session, timer, .. } = &state.snapshot().state else {
        panic!("expected active Focus");
    };
    assert_eq!(
        session.current_task.as_ref().unwrap().as_str(),
        "原稿を書く"
    );
    assert!(matches!(timer, TimerState::Interrupted { .. }));
    drop(store);

    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("原稿を書く");
    tui.send(b"q");
    assert!(tui.finish().success());
}

#[test]
fn native_pty_requires_consent_before_recovering_a_broken_primary() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("state"));
    fs::create_dir_all(location.directory()).unwrap();
    fs::write(location.state_path(), b"broken primary").unwrap();
    fs::write(location.backup_path(), VALID).unwrap();

    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("may be lost");
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    tui.send(b"y");
    tui.expect("Focus");
    tui.send(b"q");
    assert!(tui.finish().success());

    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected recovered V1 state");
    };
    assert_eq!(store.saved_state().unwrap().save_generation(), 8);
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
    assert!(fs::read_dir(location.directory()).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry.file_name().to_string_lossy().contains("quarantine")
            && fs::read(entry.path()).unwrap() == b"broken primary"
    }));
}

#[test]
fn native_pty_keeps_save_recovery_keys_visible_and_retries_the_same_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("state"));
    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("Space: Start");
    tui.send(b" ");
    tui.expect("Session started and saved");
    let saved_before_failure = fs::read(location.state_path()).unwrap();
    fs::remove_file(location.backup_path()).unwrap();
    fs::create_dir(location.backup_path()).unwrap();
    tui.send(b"d");
    tui.expect("Could not confirm save");
    assert_eq!(
        fs::read(location.state_path()).unwrap(),
        saved_before_failure
    );
    tui.resize(24, 9);
    tui.expect_screen("Enlarge terminal to show save recovery controls.");
    tui.assert_screen_absent("Retry save");
    tui.resize(24, 20);
    tui.expect_screen_all(&["r: Retry save", "Q: Confirm unsaved exit"]);
    tui.assert_screen_absent("Enlarge terminal");
    fs::remove_dir(location.backup_path()).unwrap();
    tui.send(b"r");
    tui.expect("Distraction saved");
    tui.send(b"q");
    assert!(tui.finish().success());

    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected saved V1 state");
    };
    assert_eq!(
        store
            .saved_state()
            .unwrap()
            .domain()
            .reflection()
            .unwrap()
            .distractions,
        1
    );
}

#[test]
fn native_pty_exits_unsaved_without_committing_candidate_and_releases_lock() {
    let directory = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(directory.path().join("state"));
    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("Space: Start");
    tui.send(b" ");
    tui.expect("Session started and saved");
    let saved_before_failure = fs::read(location.state_path()).unwrap();
    fs::remove_file(location.backup_path()).unwrap();
    fs::create_dir(location.backup_path()).unwrap();
    tui.send(b"d");
    tui.expect("Could not confirm save");
    tui.resize(24, 20);
    tui.expect_screen_all(&["r: Retry save", "Q: Confirm unsaved exit"]);
    tui.send(b"Q");
    tui.expect_screen("y: Exit unsaved");
    tui.assert_screen_absent("Retry save");
    tui.send(b"y");
    assert!(!tui.finish().success());

    assert_eq!(
        fs::read(location.state_path()).unwrap(),
        saved_before_failure
    );
    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected the last confirmed V1 state");
    };
    assert_eq!(
        store
            .saved_state()
            .unwrap()
            .domain()
            .reflection()
            .unwrap()
            .distractions,
        0
    );
    drop(store);

    fs::remove_dir(location.backup_path()).unwrap();
    let mut tui = Tui::spawn(location.directory(), 100, 30);
    tui.expect("Awaiting Resume");
    tui.send(b"q");
    assert!(tui.finish().success());
    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected saved V1 state after restart");
    };
    assert_eq!(
        store
            .saved_state()
            .unwrap()
            .domain()
            .reflection()
            .unwrap()
            .distractions,
        0
    );
}
