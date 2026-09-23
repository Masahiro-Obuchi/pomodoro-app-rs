#![cfg(target_os = "linux")]

use std::fs;

use crossterm::event::KeyCode;
use pomodoro_core::{DomainState, TimerConfig, Timestamp};
use pomodoro_platform::{LoadOutcome, StorageLocation, StorageLockError, TimeError};
use pomodoro_tui::{controller::Startup, startup_gate::StartupGate};
use ratatui::{Terminal, backend::TestBackend, text::Span};

fn open(location: &StorageLocation) -> StartupGate {
    Startup::open(location.clone(), TimerConfig::default(), Timestamp(1_000))
        .unwrap()
        .into()
}

fn render(gate: &StartupGate, width: u16, height: u16) -> (String, bool) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut visible = false;
    terminal
        .draw(|frame| visible = pomodoro_tui::ui::draw_startup(frame, gate))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += u16::try_from(Span::raw(symbol).width().max(1)).unwrap();
        }
        text.push('\n');
    }
    (text, visible)
}

fn unavailable_time() -> Result<Timestamp, TimeError> {
    Err(TimeError::BeforeUnixEpoch)
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
    let (prompt, visible) = render(&gate, 80, 24);
    assert!(visible);
    assert!(prompt.contains("1970-01-01 00:00:00.500 UTC"));
    assert!(prompt.contains("may be lost"));
    assert!(prompt.contains("y: Recover"));
    assert!(matches!(
        location.clone().lock(),
        Err(StorageLockError::InUse { .. })
    ));
    let gate = gate
        .handle_key(KeyCode::Char('x'), true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::Recovery(_)));
    let gate = gate
        .handle_key(KeyCode::Char('n'), true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::Exited(_)));
    assert_eq!(fs::read(location.state_path()).unwrap(), broken);
    assert_eq!(fs::read(location.backup_path()).unwrap(), backup);
    assert!(location.clone().lock().is_ok());

    let gate = open(&location)
        .handle_key(KeyCode::Esc, true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::Exited(_)));
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
        .handle_key(KeyCode::Char('y'), true, || Ok(Timestamp(2_000)))
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
    assert!(render(&gate, 80, 24).0.contains("Q: Confirm unsaved exit"));
    let gate = gate
        .handle_key(KeyCode::Char('q'), true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::ConfirmUnsaved { .. }));
    assert!(render(&gate, 80, 24).0.contains("y: Exit unsaved"));
    let gate = gate
        .handle_key(KeyCode::Esc, true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::SaveFailed { .. }));
    fs::remove_dir(location.state_path()).unwrap();
    let gate = gate
        .handle_key(KeyCode::Char('r'), true, unavailable_time)
        .unwrap()
        .advance();
    let StartupGate::Ready(store) = gate else {
        panic!("retry should hand over a saved store");
    };
    assert_eq!(store.saved_state().unwrap().saved_at(), Timestamp(1_000));
    assert_eq!(store.saved_state().unwrap().save_generation(), 1);
}

#[test]
fn unavailable_clock_does_not_block_confirmed_unsaved_exit() {
    let dir = tempfile::tempdir().unwrap();
    let location = StorageLocation::at(dir.path().to_owned());
    let gate = open(&location);
    fs::create_dir(location.state_path()).unwrap();
    let gate = gate.advance();
    assert!(matches!(gate, StartupGate::SaveFailed { .. }));
    let gate = gate
        .handle_key(KeyCode::Char('Q'), true, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::ConfirmUnsaved { .. }));
    let gate = gate
        .handle_key(KeyCode::Char('y'), true, unavailable_time)
        .unwrap();
    assert!(matches!(
        gate,
        StartupGate::Exited(pomodoro_tui::controller::ExitOutcome::Unsaved)
    ));
    assert!(location.lock().is_ok());
}

#[test]
fn recovery_consent_waits_until_the_warning_and_keys_fit_on_screen() {
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

    let gate = open(&location);
    let (small, visible) = render(&gate, 40, 8);
    assert!(!visible);
    assert!(small.contains("Enlarge the terminal"));
    assert!(!small.contains("y: Recover"));
    let gate = gate
        .handle_key(KeyCode::Char('y'), visible, unavailable_time)
        .unwrap();
    assert!(matches!(gate, StartupGate::Recovery(_)));
    assert_eq!(fs::read(location.state_path()).unwrap(), b"broken primary");
    let (enough, visible) = render(&gate, 50, 9);
    assert!(visible);
    assert!(enough.contains("1970-01-01 00:00:00.500 UTC"));
    assert!(enough.contains("may be lost"));
    assert!(enough.contains("y: Recover"));
    let gate = gate
        .handle_key(KeyCode::Char('y'), visible, || Ok(Timestamp(2_000)))
        .unwrap()
        .advance();
    assert!(matches!(gate, StartupGate::Ready(_)));
}
