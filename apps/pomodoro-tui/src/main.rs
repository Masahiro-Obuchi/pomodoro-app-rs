use std::{error::Error, process::ExitCode};

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
    terminal::run(StorageLocation::discover()?)
}
