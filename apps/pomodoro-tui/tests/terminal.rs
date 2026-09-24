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
    Command as DomainCommand, CurrentTask, DomainState, EventKind, GapReason, InterruptionKind,
    Observation, ProgressState, SessionKind, SessionOutcome, TimerConfig, TimerState, Timestamp,
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
    paste(&mut tui, "a\tb");
    tui.expect("control characters");
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
fn executable_starts_quick_start_with_the_saved_task() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("2: Quick Start (2 min)");
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, "原稿を書く");
    tui.expect("原稿を書く");
    tui.send(b"\r");
    tui.expect("Task saved");
    let ready_bytes = fs::read(location.state_path()).unwrap();
    tui.send(b"2");
    tui.expect("Running");
    assert_ne!(fs::read(location.state_path()).unwrap(), ready_bytes);
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let ProgressState::Active { session, .. } =
        &store.saved_state().unwrap().domain().snapshot().state
    else {
        panic!("expected Quick Start")
    };
    assert_eq!(session.kind, SessionKind::QuickStart);
    assert_eq!(session.planned_duration_ms, 120_000);
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
fn executable_reports_distraction_and_returns_after_restart() {
    for (start_key, kind) in [(b' ', SessionKind::Focus), (b'2', SessionKind::QuickStart)] {
        let dir = tempfile::tempdir().unwrap();
        let location = location(dir.path());
        let mut tui = Tui::spawn(dir.path());
        tui.expect("2: Quick Start (2 min)");
        tui.send(&[start_key]);
        tui.expect("Running");
        tui.send(b"d");
        tui.expect("Awaiting Return");
        tui.send(b"q");
        assert!(tui.finish().success());

        let store = loaded(&location);
        let domain = store.saved_state().unwrap().domain();
        let ProgressState::Active {
            session,
            timer: TimerState::Interrupted { interruption },
        } = &domain.snapshot().state
        else {
            panic!("expected a saved distraction")
        };
        assert_eq!(session.kind, kind);
        assert_eq!(interruption.kind, InterruptionKind::Distraction);
        let session_id = session.id;
        let interruption_id = interruption.id;
        assert_eq!(domain.reflection().unwrap().distractions, 1);
        assert_eq!(domain.reflection().unwrap().returns, 0);
        let work_before_return = domain.reflection().unwrap().work_ms;
        drop(store);

        let mut tui = Tui::spawn(dir.path());
        tui.expect("Awaiting Return");
        tui.send(b" ");
        tui.expect("Returned to work and saved");
        tui.send(b"q");
        assert!(tui.finish().success());

        let store = loaded(&location);
        let domain = store.saved_state().unwrap().domain();
        let ProgressState::Active { session, .. } = &domain.snapshot().state else {
            panic!("expected active session")
        };
        assert_eq!(session.id, session_id);
        assert_eq!(domain.reflection().unwrap().distractions, 1);
        assert_eq!(domain.reflection().unwrap().returns, 1);
        assert!(domain.reflection().unwrap().work_ms >= work_before_return);
        let started = domain
            .history()
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.payload,
                    EventKind::InterruptionStarted {
                        interruption_id: id,
                        interruption_kind: InterruptionKind::Distraction,
                    } if id == interruption_id
                )
            })
            .count();
        let returned = domain
            .history()
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.payload,
                    EventKind::InterruptionEnded {
                        interruption_id: id,
                        end: pomodoro_core::InterruptionEnd {
                            outcome: pomodoro_core::InterruptionOutcome::Returned,
                            ..
                        },
                    } if id == interruption_id
                )
            })
            .count();
        assert_eq!((started, returned), (1, 1));
    }
}

#[test]
fn executable_edits_retained_task_after_quick_start_reset() {
    let dir = tempfile::tempdir().unwrap();
    let location = location(dir.path());
    let mut tui = Tui::spawn(dir.path());
    tui.expect("2: Quick Start (2 min)");
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, "原稿を書く");
    tui.expect("原稿を書く");
    tui.send(b"\r");
    tui.expect("Task saved");
    tui.send(b"2");
    tui.expect("Running");
    tui.send(b"r");
    tui.expect("Quick Start · Ready");
    tui.send(b"t");
    tui.expect("Current Task · unsaved edit");
    paste(&mut tui, " 次");
    tui.expect("次");
    tui.send(b"\r");
    tui.expect("Task saved");
    tui.send(b" ");
    tui.expect("Pause");
    tui.send(b"q");
    assert!(tui.finish().success());
    let store = loaded(&location);
    let domain = store.saved_state().unwrap().domain();
    assert_eq!(domain.history().sessions.len(), 1);
    assert_eq!(
        domain.history().sessions[0].end.unwrap().outcome,
        SessionOutcome::Reset
    );
    assert_eq!(
        domain.history().sessions[0]
            .current_task
            .as_ref()
            .unwrap()
            .as_str(),
        "原稿を書く"
    );
    let ProgressState::Active { session, .. } = &domain.snapshot().state else {
        panic!("expected restarted Quick Start")
    };
    assert_eq!(session.kind, SessionKind::QuickStart);
    assert_eq!(
        session.current_task.as_ref().unwrap().as_str(),
        "原稿を書く 次"
    );
}

fn seed_awaiting_quick_start(location: &StorageLocation) {
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected new store")
    };
    let mut domain = DomainState::new(TimerConfig::default()).unwrap();
    domain
        .apply(
            DomainCommand::SetCurrentTask(CurrentTask::parse("原稿を書く").unwrap()),
            Timestamp(1_000),
        )
        .unwrap();
    domain
        .apply(
            DomainCommand::Start(SessionKind::QuickStart),
            Timestamp(1_000),
        )
        .unwrap();
    for at in (2_000..=121_000).step_by(1_000) {
        domain
            .observe(Observation {
                previous_at: Timestamp(at - 1_000),
                at: Timestamp(at),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
    }
    assert!(matches!(
        domain.snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    store.save(&domain, Timestamp(121_000)).unwrap();
}

#[test]
fn executable_keeps_quick_start_choice_across_restarts_until_finish_or_continue() {
    for choice in [b'f', b'c'] {
        let dir = tempfile::tempdir().unwrap();
        let location = location(dir.path());
        seed_awaiting_quick_start(&location);
        let mut tui = Tui::spawn(dir.path());
        tui.expect("Choose finish or continue");
        tui.send(b"q");
        assert!(tui.finish().success());
        let store = loaded(&location);
        assert!(matches!(
            store.saved_state().unwrap().domain().snapshot().state,
            ProgressState::AwaitingQuickStartDecision { .. }
        ));
        drop(store);

        let mut tui = Tui::spawn(dir.path());
        tui.expect("Choose finish or continue");
        tui.send(&[choice]);
        tui.expect(if choice == b'f' {
            "Space: Start"
        } else {
            "Space: Pause"
        });
        tui.send(b"q");
        assert!(tui.finish().success());
        let store = loaded(&location);
        let domain = store.saved_state().unwrap().domain();
        assert_eq!(domain.history().sessions.len(), 1);
        assert_eq!(
            domain.history().sessions[0].end.unwrap().outcome,
            SessionOutcome::Completed
        );
        assert_eq!(
            domain.snapshot().round_progress.completed_focuses_in_round,
            0
        );
        assert_eq!(
            domain
                .history()
                .events
                .iter()
                .filter(|event| matches!(event.payload, EventKind::QuickStartDecisionMade { .. }))
                .count(),
            1
        );
        if choice == b'f' {
            let ProgressState::Ready {
                next_kind,
                current_task_draft,
            } = &domain.snapshot().state
            else {
                panic!("expected Focus ready")
            };
            assert_eq!(*next_kind, SessionKind::Focus);
            assert_eq!(current_task_draft.as_ref().unwrap().as_str(), "原稿を書く");
        } else {
            let ProgressState::Active { session, .. } = &domain.snapshot().state else {
                panic!("expected linked Focus")
            };
            assert_eq!(session.kind, SessionKind::Focus);
            assert_eq!(
                session.continued_from_quick_start,
                Some(domain.history().sessions[0].id)
            );
            assert_eq!(
                session.current_task.as_ref().unwrap().as_str(),
                "原稿を書く"
            );
            assert_eq!(session.planned_duration_ms, 25 * 60 * 1_000);
        }
    }
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
