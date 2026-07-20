mod app;
mod ui;

use std::{error::Error, io, time::Duration};

use app::App;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pomodoro_core::TimerConfig;
use pomodoro_platform::{NativeStorage, PersistedState, unix_time_millis};
use ratatui::{Terminal, backend::CrosstermBackend};

fn main() -> Result<(), Box<dyn Error>> {
    let storage = NativeStorage::discover()?;
    let state = match storage.load() {
        Ok(Some(state)) => state,
        Ok(None) => PersistedState::new(TimerConfig::default())?,
        Err(error) => {
            eprintln!("保存データを読み込めなかったため、初期状態で起動します: {error}");
            PersistedState::new(TimerConfig::default())?
        }
    };
    let mut app = App::new(state, storage);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    while !app.should_quit() {
        let now_ms = unix_time_millis()?;
        app.tick(now_ms);
        terminal.draw(|frame| ui::draw(frame, &app, now_ms))?;

        if event::poll(Duration::from_millis(100))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Press {
                app.handle_key(key.code, unix_time_millis()?);
            }
        }
    }

    app.save();
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}
