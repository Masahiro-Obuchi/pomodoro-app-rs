#![cfg(target_os = "linux")]

use std::fs;

use crossterm::event::KeyCode;
use pomodoro_core::{DomainState, TimerConfig, Timestamp};
use pomodoro_platform::{LoadOutcome, StorageLocation, StorageLockError};
use pomodoro_tui::{controller::Startup, startup_gate::StartupGate};
use ratatui::{Terminal, backend::TestBackend, text::Span};

fn open(location: &StorageLocation) -> StartupGate {
    Startup::open(location.clone(), TimerConfig::default(), Timestamp(1_000))
        .unwrap()
        .into()
}

fn render(gate: &StartupGate) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| pomodoro_tui::ui::draw_startup(frame, gate))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..24 {
        let mut x = 0;
        while x < 80 {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += u16::try_from(Span::raw(symbol).width().max(1)).unwrap();
        }
        text.push('\n');
    }
    text
}

#[test]
fn recovery_prompt_requires_consent_and_decline_preserves_both_files() {
    let dir = tempfile::tempdir().unwrap();
    // Long paths/diagnostics must not push the time, loss warning, and consent
    // keys out of a standard terminal's visible area.
    let location = StorageLocation::at(dir.path().join("a".repeat(150)).join("b".repeat(150)));
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected new store");
    };
    store
        .save(
            &DomainState::new(TimerConfig::default()).unwrap(),
            Timestamp(500),
        )
        .unwrap();
    drop(store);
    let backup = fs::read(location.state_path()).unwrap();
    fs::write(location.backup_path(), &backup).unwrap();
    let broken = b"broken primary";
    fs::write(location.state_path(), broken).unwrap();

    let gate = open(&location);
    let prompt = render(&gate);
    assert!(prompt.contains("1970-01-01 00:00:00.500 UTC"));
    assert!(prompt.contains("失われる可能性"));
    assert!(prompt.contains("y: 復旧して続行"));
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    let gate = gate
        .handle_key(KeyCode::Char('x'), Timestamp(2_000))
        .unwrap();
    assert!(matches!(gate, StartupGate::Recovery(_)));
    let gate = gate.handle_key(KeyCode::Esc, Timestamp(2_000)).unwrap();
    assert!(matches!(gate, StartupGate::Exited(_)));
    assert_eq!(fs::read(location.state_path()).unwrap(), broken);
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
    assert!(location.lock().is_ok());
}

#[test]
fn recovery_acceptance_saves_once_before_normal_operation() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("expected new store");
    };
    store
        .save(
            &DomainState::new(TimerConfig::default()).unwrap(),
            Timestamp(500),
        )
        .unwrap();
    drop(store);
    let backup = fs::read(location.state_path()).unwrap();
    fs::write(location.backup_path(), &backup).unwrap();
    fs::write(location.state_path(), b"broken primary").unwrap();

    let gate = open(&location)
        .handle_key(KeyCode::Char('y'), Timestamp(2_000))
        .unwrap()
        .advance();
    let StartupGate::Ready(store) = gate else {
        panic!("recovery should have been saved before normal input");
    };
    let saved = store.saved_state().unwrap();
    assert_eq!(saved.save_generation(), 2);
    assert_eq!(saved.saved_at(), Timestamp(2_000));
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
}

#[test]
fn startup_save_failure_requires_retry_or_confirmed_unsaved_exit() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let gate = open(&location);
    fs::create_dir(location.state_path()).unwrap();
    let gate = gate.advance();
    assert!(matches!(gate, StartupGate::SaveFailed { .. }));
    let gate = gate
        .handle_key(KeyCode::Char('q'), Timestamp(2_000))
        .unwrap();
    assert!(matches!(gate, StartupGate::ConfirmUnsaved { .. }));
    let gate = gate.handle_key(KeyCode::Esc, Timestamp(2_000)).unwrap();
    assert!(matches!(gate, StartupGate::SaveFailed { .. }));
    fs::remove_dir(location.state_path()).unwrap();
    let gate = gate
        .handle_key(KeyCode::Char('r'), Timestamp(2_000))
        .unwrap()
        .advance();
    let StartupGate::Ready(store) = gate else {
        panic!("retry should hand over a saved store");
    };
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(1_000));
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
}
