use crossterm::event::KeyCode;
use pomodoro_core::{SessionKind, TimerConfig, TimerError, TimerEvent, TimerStatus};
use pomodoro_platform::{NativeStorage, NotifySendNotifier, PersistedState, local_date_at};

const DURATION_STEP_SECONDS: u64 = 60;
const MIN_DURATION_SECONDS: u64 = 60;
const MAX_DURATION_SECONDS: u64 = 24 * 60 * 60;
const MAX_FOCUSES_BEFORE_LONG_BREAK: u32 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    FocusDuration,
    ShortBreakDuration,
    LongBreakDuration,
    FocusesBeforeLongBreak,
}

impl SettingsField {
    const fn next(self) -> Self {
        match self {
            Self::FocusDuration => Self::ShortBreakDuration,
            Self::ShortBreakDuration => Self::LongBreakDuration,
            Self::LongBreakDuration => Self::FocusesBeforeLongBreak,
            Self::FocusesBeforeLongBreak => Self::FocusDuration,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::FocusDuration => Self::FocusesBeforeLongBreak,
            Self::ShortBreakDuration => Self::FocusDuration,
            Self::LongBreakDuration => Self::ShortBreakDuration,
            Self::FocusesBeforeLongBreak => Self::LongBreakDuration,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    focus_seconds: u64,
    short_break_seconds: u64,
    long_break_seconds: u64,
    focuses_before_long_break: u32,
    selected: SettingsField,
}

impl SettingsDraft {
    fn from_config(config: &TimerConfig) -> Self {
        Self {
            focus_seconds: config.focus_seconds(),
            short_break_seconds: config.short_break_seconds(),
            long_break_seconds: config.long_break_seconds(),
            focuses_before_long_break: config.focuses_before_long_break(),
            selected: SettingsField::FocusDuration,
        }
    }

    pub const fn selected(&self) -> SettingsField {
        self.selected
    }

    pub const fn focus_seconds(&self) -> u64 {
        self.focus_seconds
    }

    pub const fn short_break_seconds(&self) -> u64 {
        self.short_break_seconds
    }

    pub const fn long_break_seconds(&self) -> u64 {
        self.long_break_seconds
    }

    pub const fn focuses_before_long_break(&self) -> u32 {
        self.focuses_before_long_break
    }

    fn select_next(&mut self) {
        self.selected = self.selected.next();
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.previous();
    }

    fn adjust(&mut self, increase: bool) {
        match self.selected {
            SettingsField::FocusDuration => {
                adjust_duration(&mut self.focus_seconds, increase);
            }
            SettingsField::ShortBreakDuration => {
                adjust_duration(&mut self.short_break_seconds, increase);
            }
            SettingsField::LongBreakDuration => {
                adjust_duration(&mut self.long_break_seconds, increase);
            }
            SettingsField::FocusesBeforeLongBreak => {
                self.focuses_before_long_break = if increase {
                    self.focuses_before_long_break
                        .saturating_add(1)
                        .min(MAX_FOCUSES_BEFORE_LONG_BREAK)
                } else {
                    self.focuses_before_long_break.saturating_sub(1).max(1)
                };
            }
        }
    }

    fn build_config(&self) -> Result<TimerConfig, pomodoro_core::ConfigError> {
        TimerConfig::new(
            self.focus_seconds,
            self.short_break_seconds,
            self.long_break_seconds,
            self.focuses_before_long_break,
        )
    }
}

fn adjust_duration(seconds: &mut u64, increase: bool) {
    *seconds = if increase {
        seconds
            .saturating_add(DURATION_STEP_SECONDS)
            .min(MAX_DURATION_SECONDS)
    } else {
        seconds
            .saturating_sub(DURATION_STEP_SECONDS)
            .max(MIN_DURATION_SECONDS)
    };
}

pub struct App {
    state: PersistedState,
    storage: NativeStorage,
    should_quit: bool,
    show_help: bool,
    settings: Option<SettingsDraft>,
    message: String,
}

impl App {
    pub fn new(state: PersistedState, storage: NativeStorage) -> Self {
        Self {
            state,
            storage,
            should_quit: false,
            show_help: false,
            settings: None,
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

    pub const fn settings(&self) -> Option<&SettingsDraft> {
        self.settings.as_ref()
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
        if self.settings.is_some() {
            if self.handle_settings_key(key) {
                self.save();
            }
            return;
        }

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
            KeyCode::Char('s') => self.open_settings(),
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

    fn open_settings(&mut self) {
        if self.state.timer.status() == TimerStatus::Idle {
            self.settings = Some(SettingsDraft::from_config(self.state.timer.config()));
            self.show_help = false;
        } else {
            "設定は待機中のみ変更できます。現在のセッションをリセットしてください。"
                .clone_into(&mut self.message);
        }
    }

    fn handle_settings_key(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Esc | KeyCode::Char('s') => {
                self.settings = None;
                "設定の変更をキャンセルしました".clone_into(&mut self.message);
                false
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.settings
                    .as_mut()
                    .expect("settings are open")
                    .select_previous();
                false
            }
            KeyCode::Down | KeyCode::Tab => {
                self.settings
                    .as_mut()
                    .expect("settings are open")
                    .select_next();
                false
            }
            KeyCode::Left | KeyCode::Char('-') => {
                self.settings
                    .as_mut()
                    .expect("settings are open")
                    .adjust(false);
                false
            }
            KeyCode::Right | KeyCode::Char('+' | '=') => {
                self.settings
                    .as_mut()
                    .expect("settings are open")
                    .adjust(true);
                false
            }
            KeyCode::Enter => self.apply_settings(),
            _ => false,
        }
    }

    fn apply_settings(&mut self) -> bool {
        let draft = self.settings.take().expect("settings are open");
        let result = draft
            .build_config()
            .map_err(TimerError::from)
            .and_then(|config| self.state.timer.reconfigure(config));

        match result {
            Ok(()) => {
                "設定を保存し、現在のセッションとラウンドをリセットしました"
                    .clone_into(&mut self.message);
                true
            }
            Err(error) => {
                self.settings = Some(draft);
                self.message = format!("設定を適用できませんでした: {error}");
                false
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
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use pomodoro_core::TimerConfig;

    use super::*;

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn app() -> App {
        let test_id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        App::new(
            PersistedState::new(TimerConfig::default()).unwrap(),
            NativeStorage::at(PathBuf::from(format!(
                "/tmp/pomodoro-tui-test-{}-{test_id}.json",
                std::process::id()
            ))),
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

    #[test]
    fn settings_can_be_edited_and_applied_while_idle() {
        let mut app = app();

        app.handle_key(KeyCode::Char('s'), 0);
        assert_eq!(
            app.settings().map(SettingsDraft::selected),
            Some(SettingsField::FocusDuration)
        );

        app.handle_key(KeyCode::Right, 0);
        app.handle_key(KeyCode::Down, 0);
        app.handle_key(KeyCode::Left, 0);
        app.handle_key(KeyCode::Enter, 0);

        assert!(app.settings().is_none());
        assert_eq!(app.state.timer.config().focus_seconds(), 26 * 60);
        assert_eq!(app.state.timer.config().short_break_seconds(), 4 * 60);
        assert_eq!(app.state.timer.status(), TimerStatus::Idle);
        assert_eq!(app.state.timer.remaining_seconds(0), 26 * 60);

        let restored = app.storage.load().unwrap().unwrap();
        assert_eq!(restored.timer.config().focus_seconds(), 26 * 60);
        assert_eq!(restored.timer.config().short_break_seconds(), 4 * 60);
    }

    #[test]
    fn settings_cannot_open_while_timer_is_running() {
        let mut app = app();
        app.toggle_timer(0);

        app.handle_key(KeyCode::Char('s'), 0);

        assert!(app.settings().is_none());
        assert!(app.message().contains("待機中"));
    }
}
