#![cfg(target_os = "linux")]

use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use pomodoro_core::{
    Command as DomainCommand, DomainState, EventKind, GapReason, InterruptionKind, Observation,
    ProgressState, SessionKind, TimerConfig, TimerState, Timestamp,
};
use pomodoro_platform::{LoadOutcome, StorageLocation, WritableStorage};
use rustix::{
    pty::{OpenptFlags, ioctl_tiocgptpeer, openpt, unlockpt},
    termios::{Winsize, tcsetwinsize},
};

const TIMEOUT: Duration = Duration::from_secs(10);
const VALID: &[u8] =
    include_bytes!("../../../crates/pomodoro-platform/tests/fixtures/state_v1.json");

#[test]
#[ignore = "PTY subprocess helper invoked by terminal integration tests"]
fn terminal_probe() {
    // Detach from the test runner's controlling terminal. Crossterm must use the
    // supplied PTY for both input and size, even when cargo runs in a terminal.
    rustix::process::setsid().unwrap();
    let error = Command::new(env!("CARGO_BIN_EXE_pomodoro-tui")).exec();
    panic!("could not start TUI: {error}");
}

struct Tui {
    child: Child,
    input: File,
    output: Receiver<Vec<u8>>,
    received: Vec<u8>,
}

impl Tui {
    fn spawn(directory: &Path) -> Self {
        Self::spawn_sized(directory, 100, 30)
    }

    fn spawn_sized(directory: &Path, width: u16, height: u16) -> Self {
        let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC;
        let master = openpt(flags).unwrap();
        unlockpt(&master).unwrap();
        let slave = File::from(ioctl_tiocgptpeer(&master, flags).unwrap());
        tcsetwinsize(
            &slave,
            Winsize {
                ws_row: height,
                ws_col: width,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let input = File::from(master);
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "terminal_probe", "--ignored", "--nocapture"])
            .env("XDG_STATE_HOME", directory)
            .env("TERM", "xterm-256color")
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave)
            .spawn()
            .unwrap();
        let mut reader = input.try_clone().unwrap();
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
            input,
            output,
            received: Vec::new(),
        }
    }

    fn expect(&mut self, text: &str) {
        let deadline = Instant::now() + TIMEOUT;
        let expected: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        loop {
            // Ratatui may position spaces with cursor commands rather than
            // emitting them. Compare the visible words without whitespace.
            let emitted = without_csi(&self.received);
            let compact: String = emitted.chars().filter(|ch| !ch.is_whitespace()).collect();
            if compact.contains(&expected) {
                self.received.clear();
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.received.extend(bytes),
                Err(error) => panic!("waiting for {text:?}: {error}; output: {emitted}"),
            }
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.input.write_all(keys).unwrap();
    }

    fn finish(mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "TUI did not exit");
            // This polls process termination; domain timing tests inject clocks.
            std::thread::sleep(Duration::from_millis(5));
        }
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
        } else {
            result.push(ch);
        }
    }
    result
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn location(directory: &Path) -> StorageLocation {
    StorageLocation::at(directory.join("pomodoro-app-rs"))
}

fn loaded(location: &StorageLocation) -> WritableStorage {
    let LoadOutcome::Loaded(store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected saved V1 state");
    };
    store
}

fn paste(tui: &mut Tui, text: &str) {
    tui.send(b"\x1b[200~");
    tui.send(text.as_bytes());
    tui.send(b"\x1b[201~");
}

#[test]
fn executable_edits_task_without_saving_until_enter_and_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("t: Edit task");
    let initial = fs::read(location.state_path()).unwrap();
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, "  原稿を書く  ");
    tui.expect("原稿を書く");
    assert_eq!(fs::read(location.state_path()).unwrap(), initial);
    tui.send(b"\x1b");
    tui.expect("Task edit canceled");
    assert_eq!(fs::read(location.state_path()).unwrap(), initial);
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, "  原稿を書く  ");
    tui.expect("原稿を書く");
    tui.send(b"\r");
    tui.expect("Task saved");
    assert_ne!(fs::read(location.state_path()).unwrap(), initial);
    tui.send(b" ");
    tui.expect("Running");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Active { session, .. } =
        &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("expected active Focus")
    };
    assert_eq!(
        session.current_task.as_ref().unwrap().as_str(),
        "原稿を書く"
    );
    drop(store);
    let mut tui = Tui::spawn(dir.path());
    tui.expect("原稿を書く");
    tui.send(b"q");
    assert!(tui.finish().success());
}

#[test]
fn bracketed_multiline_paste_stays_in_task_editor_and_plain_key_stream_can_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("Focus");
    let initial = fs::read(location.state_path()).unwrap();
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, "first\nq n");
    tui.expect("paste was rejected");
    assert_eq!(fs::read(location.state_path()).unwrap(), initial);
    tui.send(b"a");
    tui.send(b"\r");
    tui.expect("Task saved");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Ready {
        current_task_draft, ..
    } = &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("expected ready")
    };
    assert_eq!(current_task_draft.as_ref().unwrap().as_str(), "a");
    drop(store);

    let mut tui = Tui::spawn(dir.path());
    tui.expect("Task: a");
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    // A terminal without bracketed paste sends ordinary keys. Its first Enter
    // confirms the edit, so following keys can reach normal controls.
    tui.send(b"b\r");
    tui.expect("Task saved");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Ready {
        current_task_draft, ..
    } = &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("expected ready")
    };
    assert_eq!(current_task_draft.as_ref().unwrap().as_str(), "ab");
}

#[test]
fn executable_can_start_focus_without_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("Focus");
    tui.send(b" ");
    tui.expect("Session started and saved");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Active { session, .. } =
        &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("expected active Focus")
    };
    assert!(session.current_task.is_none());
}

#[test]
fn executable_saves_restarts_and_rejects_second_launch() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("Focus");
    tui.send(b" ");
    tui.expect("Session started and saved");

    let second = Command::new(env!("CARGO_BIN_EXE_pomodoro-tui"))
        .env("XDG_STATE_HOME", dir.path())
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already in use"));

    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Active {
        session,
        timer: TimerState::Interrupted { interruption },
    } = &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("shutdown must save an interrupted session");
    };
    assert_eq!(interruption.kind, InterruptionKind::AppExit);
    let original_session = session.clone();
    drop(store);

    let mut tui = Tui::spawn(dir.path());
    tui.expect("Stopped on exit · Awaiting Resume");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let domain = store.saved_state().unwrap().domain();
    let ProgressState::Active { session, .. } = &domain.snapshot().state else {
        panic!("restart must retain the session");
    };
    assert_eq!(session, &original_session);
    assert_eq!(
        domain
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::AppRestored))
            .count(),
        1
    );
}

#[test]
fn executable_restores_old_running_session_without_crediting_downtime() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected new store");
    };
    let mut domain = DomainState::new(TimerConfig::default()).unwrap();
    domain
        .apply(DomainCommand::Start(SessionKind::Focus), Timestamp(1_000))
        .unwrap();
    domain
        .observe(Observation {
            previous_at: Timestamp(1_000),
            at: Timestamp(1_500),
            monotonic_elapsed_ms: Some(500),
        })
        .unwrap();
    store.save(&domain, Timestamp(1_500)).unwrap();
    drop(store);

    // The executable's real startup clock is decades later than this saved run.
    let mut tui = Tui::spawn(dir.path());
    tui.expect("Timing gap · Awaiting Resume");
    tui.send(b"q");
    assert!(tui.finish().success());

    let store = loaded(&location);
    let domain = store.saved_state().unwrap().domain();
    let ProgressState::Active {
        session,
        timer: TimerState::Interrupted { interruption },
    } = &domain.snapshot().state
    else {
        panic!("restart must interrupt the running session");
    };
    assert_eq!(session.kind, SessionKind::Focus);
    assert_eq!(session.elapsed_ms, 500);
    assert_eq!(interruption.kind, InterruptionKind::ObservationGap);
    assert_eq!(interruption.started_at, Timestamp(1_500));
    assert_eq!(
        domain
            .history()
            .events
            .iter()
            .filter_map(|event| match event.payload {
                EventKind::RunIntervalRecorded { credited_ms, .. } => Some(credited_ms),
                _ => None,
            })
            .sum::<u64>(),
        500
    );
    assert_eq!(
        domain
            .history()
            .events
            .iter()
            .filter(|event| matches!(
                event.payload,
                EventKind::ObservationGapDetected {
                    last_confirmed_at: Timestamp(1_500),
                    reason: GapReason::Restart,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn executable_offers_recovery_and_waits_for_explicit_consent() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    fs::create_dir_all(location.directory()).unwrap();
    fs::write(location.backup_path(), VALID).unwrap();
    fs::write(location.state_path(), b"broken primary").unwrap();

    let mut tui = Tui::spawn_sized(dir.path(), 40, 8);
    tui.expect("Enlarge the terminal");
    tui.send(b"yq");
    assert!(tui.finish().success());
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);

    let mut tui = Tui::spawn(dir.path());
    tui.expect("may be lost");
    tui.send(b"n");
    assert!(tui.finish().success());
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);

    let mut tui = Tui::spawn(dir.path());
    tui.expect("may be lost");
    tui.send(b"y");
    tui.expect("Focus");
    tui.send(b"q");
    assert!(tui.finish().success());
    assert_eq!(
        loaded(&location).saved_state().unwrap().save_generation(),
        8
    );
    assert_eq!(fs::read(location.backup_path()).unwrap(), VALID);
    assert!(fs::read_dir(location.directory()).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry.file_name().to_string_lossy().contains("quarantine")
            && fs::read(entry.path()).unwrap() == b"broken primary"
    }));
}

#[test]
fn executable_handles_startup_save_failure_with_retry_or_unsaved_exit() {
    for retry in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let location = location(dir.path());
        let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
            panic!("expected new store");
        };
        let mut domain = DomainState::new(TimerConfig::default()).unwrap();
        domain
            .apply(DomainCommand::Start(SessionKind::Focus), Timestamp(1_000))
            .unwrap();
        store.save(&domain, Timestamp(1_000)).unwrap();
        drop(store);
        let original = fs::read(location.state_path()).unwrap();
        fs::create_dir(location.backup_path()).unwrap();

        let mut tui = Tui::spawn(dir.path());
        tui.expect("Startup save unconfirmed");
        if retry {
            fs::remove_dir(location.backup_path()).unwrap();
            tui.send(b"r");
            tui.expect("Timing gap · Awaiting Resume");
            tui.send(b"q");
            assert!(tui.finish().success());
            assert!(loaded(&location).saved_state().unwrap().save_generation() > 1);
        } else {
            tui.send(b"Qy");
            assert!(!tui.finish().success());
            assert_eq!(fs::read(location.state_path()).unwrap(), original);
            assert!(location.lock().is_ok());
        }
    }
}

#[test]
fn executable_reports_unsaved_shutdown_as_failure() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("Focus");
    tui.send(b" ");
    tui.expect("Session started and saved");
    let original = fs::read(location.state_path()).unwrap();
    fs::remove_file(location.backup_path()).unwrap();
    fs::create_dir(location.backup_path()).unwrap();
    tui.send(b"q");
    tui.expect("RenameBackup");
    tui.send(b"Qy");
    assert!(!tui.finish().success());
    assert_eq!(fs::read(location.state_path()).unwrap(), original);
    assert!(location.lock().is_ok());
}
