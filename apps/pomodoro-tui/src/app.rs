//! Terminal interaction over the save-confirmed controller. No timing policy or I/O lives here.

use std::{cell::RefCell, fmt::Write as _};

use crossterm::event::KeyCode;
use pomodoro_core::{
    Command, DomainError, DomainState, InterruptionKind, ProgressState, QuickStartChoice,
    ReflectionSummary, SessionKind, SessionOutcome, TimerState,
};

use crate::controller::{
    Clock, Commit, CompletionNotifier, Controller, ControllerError, ExitOutcome, SaveStore,
};
pub use crate::settings::{SettingsDraft, SettingsField};

mod reflection;
use reflection::HistoryReflection;

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnsavedExit {
    None,
    Confirming,
    Confirmed,
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
        if self.should_quit() {
            return;
        }
        if self.confirming_unsaved_exit() {
            match key {
                KeyCode::Char('y') => self.unsaved_exit = UnsavedExit::Confirmed,
                KeyCode::Esc | KeyCode::Char('n') => self.unsaved_exit = UnsavedExit::None,
                _ => {}
            }
            return;
        }
        if key == KeyCode::Char('?') {
            self.show_help = !self.show_help;
            return;
        }
        if self.pending_state().is_some() || self.shutdown_failed {
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
            return;
        }
        if self.settings.is_some() {
            self.handle_settings_key(key);
            return;
        }
        match key {
            KeyCode::Char('q') => match self.controller.shutdown() {
                Ok(commit) => self.on_commit(commit),
                Err(error) => {
                    self.shutdown_failed = true;
                    self.on_error(&error);
                }
            },
            KeyCode::Char('s') => self.open_settings(),
            _ => {
                if let Some(command) = self.command_for_key(key) {
                    match self.controller.execute(command) {
                        Ok(commit) => self.on_commit(commit),
                        Err(error) => self.on_error(&error),
                    }
                }
            }
        }
    }

    // Resolve against the state the user sees, before execute observes time.
    // Controller discards this command if that observation completes the session.
    fn command_for_key(&self, key: KeyCode) -> Option<Command> {
        match (&self.state().snapshot().state, key) {
            (ProgressState::Ready { next_kind, .. }, KeyCode::Char(' ')) => {
                Some(Command::Start(*next_kind))
            }
            (ProgressState::Ready { .. }, KeyCode::Char('r')) => Some(Command::ResetReady),
            (ProgressState::Ready { .. }, KeyCode::Char('n')) => Some(Command::SkipReady),
            (ProgressState::Active { session, timer }, KeyCode::Char(' ')) => Some(match timer {
                TimerState::Running { .. } => Command::Pause(session.id),
                TimerState::Interrupted { interruption }
                    if interruption.kind == InterruptionKind::Distraction =>
                {
                    Command::Return(session.id)
                }
                TimerState::Interrupted { .. } => Command::Resume(session.id),
            }),
            (ProgressState::Active { session, .. }, KeyCode::Char(key @ ('r' | 'n'))) => {
                Some(Command::End {
                    session_id: session.id,
                    outcome: if key == 'r' {
                        SessionOutcome::Reset
                    } else {
                        SessionOutcome::Skipped
                    },
                })
            }
            (
                ProgressState::AwaitingQuickStartDecision {
                    quick_start_session_id,
                    ..
                },
                KeyCode::Char(key @ ('c' | 'f')),
            ) => Some(Command::DecideQuickStart {
                session_id: *quick_start_session_id,
                choice: if key == 'c' {
                    QuickStartChoice::Continue
                } else {
                    QuickStartChoice::Finish
                },
            }),
            _ => None,
        }
    }

    fn open_settings(&mut self) {
        if matches!(self.state().snapshot().state, ProgressState::Ready { .. }) {
            self.settings = Some(SettingsDraft::from_config(
                &self.state().snapshot().settings,
            ));
            self.show_help = false;
            self.message.clear();
        } else {
            "設定は待機中のみ変更できます。".clone_into(&mut self.message);
        }
    }

    fn handle_settings_key(&mut self, key: KeyCode) {
        let draft = self.settings.as_mut().expect("settings are open");
        match key {
            KeyCode::Esc | KeyCode::Char('s') => {
                self.settings = None;
                "設定の変更をキャンセルしました".clone_into(&mut self.message);
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
                Err(error) => self.message = format!("設定を適用できませんでした: {error}"),
            },
            _ => {}
        }
    }

    fn on_error(&mut self, error: &ControllerError) {
        self.message = if self.pending_state().is_some() || self.shutdown_failed {
            format!("保存を確認できませんでした: {error}")
        } else {
            format!("操作できませんでした: {error}")
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
            let _ = write!(self.message, "（通知失敗: {error}）");
        }
    }
}

fn command_message(command: &Command) -> &'static str {
    match command {
        Command::Configure(_) => "設定を保存し、ラウンドをリセットしました。",
        Command::Start(_) => "開始操作を保存しました。",
        Command::Pause(_) => "一時停止しました。",
        Command::Resume(_) | Command::Return(_) => "復帰操作を保存しました。",
        Command::End {
            outcome: SessionOutcome::Reset,
            ..
        } => "開始待ちに戻しました。次の開始は新しいセッションになります。",
        Command::ResetReady => "すでに開始待ちです。状態は変更していません。",
        Command::End { .. } | Command::SkipReady => "次のセッションの開始待ちへ移動しました。",
        Command::DecideQuickStart { .. } => "Quick Startの選択を保存しました。",
        Command::CloseApp => "状態を保存しました。終了します。",
        _ => "操作を保存しました。",
    }
}

const fn completion_message(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Focus => "集中タイムが完了しました。休憩しましょう！",
        SessionKind::QuickStart => "Quick Startが完了しました。終了または継続を選んでください。",
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            "休憩が完了しました。次の集中タイムを始められます。"
        }
    }
}

#[cfg(test)]
mod tests;
