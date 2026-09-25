use std::{env, error::Error, io, process::ExitCode};

use pomodoro_platform::StorageLocation;
use pomodoro_tui::{controller::ExitOutcome, terminal};

fn main() -> ExitCode {
    match run() {
        Ok(ExitOutcome::Saved | ExitOutcome::Cancelled) => ExitCode::SUCCESS,
        Ok(ExitOutcome::Unsaved) => {
            eprintln!("Exited without a confirmed save.");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("Cannot continue: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitOutcome, Box<dyn Error>> {
    let location = match env::var_os("POMODORO_STATE_DIR") {
        Some(directory) if directory.is_empty() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "POMODORO_STATE_DIR cannot be empty",
            )
            .into());
        }
        Some(directory) => StorageLocation::at(directory.into()),
        None => StorageLocation::discover()?,
    };
    terminal::run(location)
}
