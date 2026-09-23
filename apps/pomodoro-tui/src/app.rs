//! Terminal interaction over the save-confirmed controller. No timing policy or I/O lives here.

use std::{cell::RefCell, fmt::Write as _};

use crossterm::event::KeyCode;
use pomodoro_core::{
    Command, DomainError, DomainState, ProgressState, ReflectionSummary, SessionKind,
    SessionOutcome,
};

use crate::controller::{
    Clock, Commit, CompletionNotifier, Controller, ControllerError, ExitOutcome, SaveStore,
};
pub use crate::settings::{SettingsDraft, SettingsField};

mod actions;
mod reflection;
use actions::{NormalAction, NormalControls};
use reflection::HistoryReflection;

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnsavedExit {
    None,
    Confirming,
    Confirmed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputContext {
    Closed,
    ConfirmUnsavedExit,
    SaveBlocked,
    Settings,
    Normal,
}

pub struct App<S, C, N> {
    controller: Controller<S, C, N>,
    history_reflection: RefCell<HistoryReflection>,
    show_help: bool,
    settings: Option<SettingsDraft>,
    message: String,
    unsaved_exit: UnsavedExit,
    shutdown_failed: bool,
}

impl<S: SaveStore, C: Clock, N: CompletionNotifier> App<S, C, N> {
    #[must_use]
    pub fn new(controller: Controller<S, C, N>) -> Self {
        Self {
            controller,
            history_reflection: RefCell::default(),
            show_help: false,
            settings: None,
            message: String::new(),
            unsaved_exit: UnsavedExit::None,
            shutdown_failed: false,
        }
    }

    #[must_use]
    pub fn state(&self) -> &DomainState {
        self.controller.state()
    }

    /// Reflects the displayed state, keeping pending saves out of the summary.
    /// History is cached separately from the active session's live elapsed time.
    ///
    /// # Errors
    /// Returns an error when the total work duration exceeds its integer range.
    pub fn reflection(&self) -> Result<ReflectionSummary, DomainError> {
        self.history_reflection.borrow_mut().get(self.state())
    }

    #[must_use]
    pub fn pending_state(&self) -> Option<&DomainState> {
        self.controller.pending_state()
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn show_help(&self) -> bool {
        self.show_help
    }

    #[must_use]
    pub const fn settings(&self) -> Option<&SettingsDraft> {
        self.settings.as_ref()
    }

    #[must_use]
    pub const fn confirming_unsaved_exit(&self) -> bool {
        matches!(self.unsaved_exit, UnsavedExit::Confirming)
    }

    #[must_use]
    pub const fn shutdown_failed(&self) -> bool {
        self.shutdown_failed
    }

    #[must_use]
    pub fn should_quit(&self) -> bool {
        self.unsaved_exit == UnsavedExit::Confirmed || self.controller.is_closed()
    }

    /// Releases the controller and its lock; unsaved exits never become saved exits.
    #[must_use]
    pub fn exit(self) -> ExitOutcome {
        self.controller.exit()
    }

    pub fn tick(&mut self) {
        if self.should_quit() || self.pending_state().is_some() || self.shutdown_failed {
            return;
        }
        match self.controller.tick() {
            Ok(Some(commit)) => self.on_commit(commit),
            Ok(None) => {}
            Err(error) => self.on_error(&error),
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        match self.input_context() {
            InputContext::Closed => {}
            InputContext::ConfirmUnsavedExit => match key {
                KeyCode::Char('y') => self.unsaved_exit = UnsavedExit::Confirmed,
                KeyCode::Esc | KeyCode::Char('n') => self.unsaved_exit = UnsavedExit::None,
                _ => {}
            },
            InputContext::SaveBlocked => self.handle_save_blocked_key(key),
            InputContext::Settings => self.handle_settings_key(key),
            InputContext::Normal => self.handle_normal_key(key),
        }
    }

    // Derive the input priority from the current application and controller state.
    // Phase 3 can add task editing without another early-return path in handle_key.
    pub(crate) fn input_context(&self) -> InputContext {
        if self.should_quit() {
            InputContext::Closed
        } else if self.confirming_unsaved_exit() {
            InputContext::ConfirmUnsavedExit
        } else if self.pending_state().is_some() || self.shutdown_failed {
            InputContext::SaveBlocked
        } else if self.settings.is_some() {
            InputContext::Settings
        } else {
            InputContext::Normal
        }
    }

    fn handle_save_blocked_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('r') => {
                let result = if self.pending_state().is_some() {
                    self.controller.retry()
                } else {
                    self.controller.shutdown()
                };
                match result {
                    Ok(commit) => {
                        self.shutdown_failed = false;
                        self.on_commit(commit);
                    }
                    Err(error) => self.on_error(&error),
                }
            }
            KeyCode::Char('Q') => self.unsaved_exit = UnsavedExit::Confirming,
            _ => {}
        }
    }

    fn handle_normal_key(&mut self, key: KeyCode) {
        let action = NormalControls::for_snapshot(self.state().snapshot()).action_for_key(key);
        match action {
            Some(NormalAction::ToggleHelp) => self.show_help = !self.show_help,
            Some(NormalAction::Shutdown) => match self.controller.shutdown() {
                Ok(commit) => self.on_commit(commit),
                Err(error) => {
                    self.shutdown_failed = true;
                    self.on_error(&error);
                }
            },
            Some(NormalAction::RequestSettings) => self.open_settings(),
            Some(NormalAction::Command(command)) => match self.controller.execute(command) {
                Ok(commit) => self.on_commit(commit),
                Err(error) => self.on_error(&error),
            },
            None => {}
        }
    }

    pub(crate) fn normal_hint_lines(&self) -> [String; 2] {
        NormalControls::for_snapshot(self.state().snapshot()).hint_lines()
    }

    fn open_settings(&mut self) {
        if matches!(self.state().snapshot().state, ProgressState::Ready { .. }) {
            self.settings = Some(SettingsDraft::from_config(
                &self.state().snapshot().settings,
            ));
            self.show_help = false;
            self.message.clear();
        } else {
            "Settings are available only while ready.".clone_into(&mut self.message);
        }
    }

    fn handle_settings_key(&mut self, key: KeyCode) {
        let draft = self.settings.as_mut().expect("settings are open");
        match key {
            KeyCode::Esc | KeyCode::Char('s') => {
                self.settings = None;
                "Settings changes canceled.".clone_into(&mut self.message);
            }
            KeyCode::Up | KeyCode::BackTab => draft.select_previous(),
            KeyCode::Down | KeyCode::Tab => draft.select_next(),
            KeyCode::Left | KeyCode::Char('-') => draft.adjust(false),
            KeyCode::Right | KeyCode::Char('+' | '=') => draft.adjust(true),
            KeyCode::Enter => match draft.build_config() {
                Ok(config) => match self.controller.execute(Command::Configure(config)) {
                    Ok(commit) => {
                        self.settings = None;
                        self.on_commit(commit);
                    }
                    Err(error) => {
                        if self.pending_state().is_some() {
                            self.settings = None;
                        }
                        self.on_error(&error);
                    }
                },
                Err(error) => self.message = format!("Could not apply settings: {error}"),
            },
            _ => {}
        }
    }

    fn on_error(&mut self, error: &ControllerError) {
        self.message = if self.pending_state().is_some() || self.shutdown_failed {
            format!("Could not confirm save: {error}")
        } else {
            format!("Action failed: {error}")
        };
    }

    fn on_commit(&mut self, commit: Commit) {
        if let Some(kind) = commit.completed {
            completion_message(kind).clone_into(&mut self.message);
        } else if let Some(command) = commit.command {
            command_message(&command).clone_into(&mut self.message);
        } else if self.pending_state().is_none() {
            // A checkpoint/recovery may clear an earlier error but is not an input success.
            self.message.clear();
        }
        if let Some(error) = commit.notification_error {
            let _ = write!(self.message, " (Notification failed: {error})");
        }
    }
}

fn command_message(command: &Command) -> &'static str {
    match command {
        Command::Configure(_) => "Settings saved; round progress reset.",
        Command::Start(_) => "Session started and saved.",
        Command::Pause(_) => "Paused and saved.",
        Command::Resume(_) | Command::Return(_) => "Resumed and saved.",
        Command::End {
            outcome: SessionOutcome::Reset,
            ..
        } => "Reset to ready. The next start creates a new session.",
        Command::ResetReady => "Already ready; nothing changed.",
        Command::End { .. } | Command::SkipReady => "Ready for the next session.",
        Command::DecideQuickStart { .. } => "Quick Start choice saved.",
        Command::CloseApp => "State saved. Exiting.",
        _ => "Action saved.",
    }
}

const fn completion_message(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Focus => "Focus complete. Take a break!",
        SessionKind::QuickStart => "Quick Start complete. Finish or continue to Focus.",
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            "Break complete. Ready for the next Focus."
        }
    }
}

#[cfg(test)]
mod tests;
