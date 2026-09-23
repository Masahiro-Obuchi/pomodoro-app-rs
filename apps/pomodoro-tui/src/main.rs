use std::{error::Error, io, process::ExitCode, time::Duration};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pomodoro_core::TimerConfig;
use pomodoro_platform::{NotifySendNotifier, ObservationClock, StorageLocation};
use pomodoro_tui::{
    app::App,
    controller::{Controller, ExitOutcome, Startup},
    startup_gate::StartupGate,
    ui,
};
use ratatui::{Terminal, backend::CrosstermBackend};

fn main() -> ExitCode {
    match run() {
        Ok(ExitOutcome::Saved | ExitOutcome::Cancelled) => ExitCode::SUCCESS,
        Ok(ExitOutcome::Unsaved) => {
            eprintln!("保存を確認できないまま終了しました。");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("実行を継続できません: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitOutcome, Box<dyn Error>> {
    // Lock and validate before entering the alternate screen. An unsupported or
    // broken state fails here without ever initializing a replacement.
    let startup_clock = ObservationClock::new()?;
    let startup = Startup::open(
        StorageLocation::discover()?,
        TimerConfig::default(),
        startup_clock.at(),
    )?;

    enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut gate = StartupGate::from(startup);
    let store = loop {
        gate = gate.advance();
        match gate {
            StartupGate::Ready(store) => break *store,
            StartupGate::Exited(outcome) => return Ok(outcome),
            waiting => {
                terminal.draw(|frame| ui::draw_startup(frame, &waiting))?;
                gate = loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            let at = ObservationClock::new()?.at();
                            break waiting.handle_key(key.code, at)?;
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
    let controller = Controller::from_saved(store, clock, NotifySendNotifier)?;
    let mut app = App::new(controller);
    while !app.should_quit() {
        app.tick();
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }
    }
    Ok(app.exit())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}
