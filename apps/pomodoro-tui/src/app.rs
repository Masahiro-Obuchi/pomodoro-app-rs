use crossterm::event::KeyCode;
use pomodoro_core::{SessionKind, TimerError, TimerEvent, TimerStatus};
use pomodoro_platform::{NativeStorage, NotifySendNotifier, PersistedState, local_date_at};

pub struct App {
    state: PersistedState,
    storage: NativeStorage,
    should_quit: bool,
    show_help: bool,
    message: String,
}

impl App {
    pub fn new(state: PersistedState, storage: NativeStorage) -> Self {
        Self {
            state,
            storage,
            should_quit: false,
            show_help: false,
            message: String::new(),
        }
    }

    pub const fn state(&self) -> &PersistedState {
        &self.state
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn show_help(&self) -> bool {
        self.show_help
    }

    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn tick(&mut self, now_ms: u64) {
        if let Some(event) = self.state.timer.tick(now_ms) {
            self.on_timer_event(event);
            self.save();
        }
    }

    pub fn handle_key(&mut self, key: KeyCode, now_ms: u64) {
        self.message.clear();
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char(' ') => self.toggle_timer(now_ms),
            KeyCode::Char('r') => {
                self.state.timer.reset();
                "現在のセッションをリセットしました".clone_into(&mut self.message);
            }
            KeyCode::Char('n') => {
                let event = self.state.timer.skip();
                self.on_timer_event(event);
                "次のセッションへ移動しました".clone_into(&mut self.message);
            }
            _ => return,
        }
        self.save();
    }

    pub fn save(&mut self) {
        if let Err(error) = self.storage.save(&self.state) {
            self.message = format!("保存に失敗しました: {error}");
        }
    }

    fn toggle_timer(&mut self, now_ms: u64) {
        let result = match self.state.timer.status() {
            TimerStatus::Idle => self.state.timer.start(now_ms),
            TimerStatus::Running => self.state.timer.pause(now_ms),
            TimerStatus::Paused => self.state.timer.resume(now_ms),
        };
        if let Err(error) = result {
            if error == TimerError::SessionAlreadyElapsed {
                self.tick(now_ms);
            } else {
                self.message = format!("操作できませんでした: {error}");
            }
        }
    }

    fn on_timer_event(&mut self, event: TimerEvent) {
        let TimerEvent::SessionCompleted {
            session,
            completed_at_ms,
            ..
        } = event
        else {
            return;
        };

        match local_date_at(completed_at_ms) {
            Ok(date) => {
                if let Err(error) = self.state.history.record_event(&date, &event) {
                    self.message = format!("履歴の更新に失敗しました: {error}");
                }
            }
            Err(error) => self.message = format!("完了日時を変換できませんでした: {error}"),
        }

        if let Err(error) = NotifySendNotifier.session_completed(session) {
            self.message = format!("セッションは完了しました（通知失敗: {error}）");
        } else {
            completion_message(session).clone_into(&mut self.message);
        }
    }
}

const fn completion_message(session: SessionKind) -> &'static str {
    match session {
        SessionKind::Focus => "集中タイムが完了しました。休憩しましょう！",
        SessionKind::ShortBreak | SessionKind::LongBreak => {
            "休憩が完了しました。次の集中タイムを始められます。"
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pomodoro_core::TimerConfig;

    use super::*;

    fn app() -> App {
        App::new(
            PersistedState::new(TimerConfig::new(10, 5, 20, 4).unwrap()).unwrap(),
            NativeStorage::at(PathBuf::from("/tmp/pomodoro-tui-test-state.json")),
        )
    }

    #[test]
    fn space_toggles_start_pause_and_resume() {
        let mut app = app();

        app.toggle_timer(0);
        assert_eq!(app.state.timer.status(), TimerStatus::Running);
        app.toggle_timer(1_000);
        assert_eq!(app.state.timer.status(), TimerStatus::Paused);
        app.toggle_timer(2_000);
        assert_eq!(app.state.timer.status(), TimerStatus::Running);
    }

    #[test]
    fn skip_moves_to_the_next_session_without_history() {
        let mut app = app();

        app.handle_key(KeyCode::Char('n'), 0);

        assert_eq!(app.state.timer.session(), SessionKind::ShortBreak);
        assert!(app.state.history.days().is_empty());
    }
}
