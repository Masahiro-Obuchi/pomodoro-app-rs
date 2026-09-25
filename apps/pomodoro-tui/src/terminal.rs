//! Native terminal event loop shared by the executable and isolated PTY tests.

use std::{error::Error, io, time::Duration};

use crate::{
    app::App,
    controller::{Controller, ExitOutcome, Startup},
    startup_gate::StartupGate,
    ui,
};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pomodoro_core::TimerConfig;
use pomodoro_platform::{DesktopNotifier, ObservationClock, StorageLocation};
use ratatui::{Terminal, backend::CrosstermBackend};

/// Runs the TUI with an already chosen storage location.
///
/// Normal startup uses [`StorageLocation::discover`]. Passing a location lets
/// native terminal tests work in an isolated directory on every target OS.
///
/// # Errors
/// Returns startup, terminal, clock, or storage errors without adopting an
/// uncertain save candidate.
pub fn run(location: StorageLocation) -> Result<ExitOutcome, Box<dyn Error>> {
    // Lock and validate before entering the alternate screen. An unsupported or
    // broken state fails here without ever initializing a replacement.
    let startup_clock = ObservationClock::new()?;
    let startup = Startup::open(location, TimerConfig::default(), startup_clock.at())?;

    enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut gate = StartupGate::from(startup);
    let store = loop {
        gate = gate.advance();
        match gate {
            StartupGate::Ready(store) => break *store,
            StartupGate::Exited(outcome) => return Ok(outcome),
            waiting => {
                let mut recovery_prompt_visible = false;
                terminal.draw(|frame| {
                    recovery_prompt_visible = ui::draw_startup(frame, &waiting);
                })?;
                gate = loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            break waiting.handle_key(key.code, recovery_prompt_visible, || {
                                ObservationClock::new().map(|clock| clock.at())
                            })?;
                        }
                        Event::Resize(..) => break waiting,
                        _ => {}
                    }
                };
            }
        }
    };

    // A fresh monotonic anchor excludes time spent on startup and recovery UI.
    let clock = ObservationClock::new()?;
    let controller = Controller::from_saved(store, clock, DesktopNotifier)?;
    let mut app = App::new(controller);
    while !app.should_quit() {
        app.tick();
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            app.handle_event(event::read()?);
        }
    }
    Ok(app.exit())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Attempt each restoration even when a previous terminal operation fails.
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
