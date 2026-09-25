#![cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use pomodoro_core::{ProgressState, TimerState};
use pomodoro_platform::{LoadOutcome, StorageLocation};
use pomodoro_tui::{controller::ExitOutcome, terminal};
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

const TIMEOUT: Duration = Duration::from_secs(15);

#[test]
#[ignore = "PTY subprocess helper invoked by native terminal tests"]
fn native_terminal_probe() {
    let directory = PathBuf::from(std::env::var_os("POMODORO_TEST_STATE_DIR").unwrap());
    assert_eq!(
        terminal::run(StorageLocation::at(directory)).unwrap(),
        ExitOutcome::Saved
    );
}

struct Tui {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    received: Vec<u8>,
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
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "native_terminal_probe",
            "--ignored",
            "--nocapture",
        ]);
        command.env("POMODORO_TEST_STATE_DIR", directory);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        command.env("TERM", "xterm-256color");
        let child = pty.slave.spawn_command(command).unwrap();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().unwrap();
        let writer = pty.master.take_writer().unwrap();
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
        }
    }

    fn expect(&mut self, phrase: &str) {
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| {
                    !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘')
                })
                .collect::<String>()
        };
        let expected = compact(phrase);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let emitted = without_csi(&self.received);
            if compact(&emitted).contains(&expected) {
                self.received.clear();
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.received.extend(bytes),
                Err(error) => panic!("waiting for {phrase:?}: {error}; output: {emitted}"),
            }
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }

    fn resize(&self, width: u16, height: u16) {
        self.master
            .resize(PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    fn finish(mut self) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "PTY helper exited with {status:?}");
                return;
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
    tui.finish();

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
    tui.finish();
}
