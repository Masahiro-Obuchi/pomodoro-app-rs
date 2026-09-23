use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use pomodoro_core::{
    CurrentTask, EventKind, InterruptionKind, Observation, SessionId, TimerConfig, TimerState,
    Timestamp,
};
use pomodoro_platform::{NotificationError, SaveError, TimeError};
use ratatui::{Terminal, backend::TestBackend};

use super::*;

mod reflection;

type Log = Rc<RefCell<Vec<&'static str>>>;

struct Store {
    saved: DomainState,
    pending: Option<DomainState>,
    failures: Rc<Cell<u32>>,
    fail_after: Rc<Cell<Option<u32>>>,
    log: Log,
}

impl Store {
    fn attempt(&mut self) -> Result<(), SaveError> {
        if let Some(left) = self.fail_after.get() {
            if left == 0 {
                self.fail_after.set(None);
                self.failures.set(1);
            } else {
                self.fail_after.set(Some(left - 1));
            }
        }
        let failures = self.failures.get();
        if failures > 0 {
            self.failures.set(failures - 1);
            self.log.borrow_mut().push("failed");
            return Err(SaveError::Conflict {
                path: "state.json".into(),
            });
        }
        self.saved = self.pending.take().unwrap();
        self.log.borrow_mut().push("saved");
        Ok(())
    }
}

impl SaveStore for Store {
    fn saved_domain(&self) -> Option<&DomainState> {
        Some(&self.saved)
    }
    fn has_pending_save(&self) -> bool {
        self.pending.is_some()
    }
    fn save(&mut self, domain: &DomainState, _: Timestamp) -> Result<(), SaveError> {
        assert!(self.pending.is_none());
        self.pending = Some(domain.clone());
        self.attempt()
    }
    fn retry_pending(&mut self) -> Result<(), SaveError> {
        self.attempt()
    }
}

struct TestClock {
    at: Rc<Cell<u64>>,
    previous: u64,
    broken: bool,
    fail_next: Rc<Cell<bool>>,
}
impl Clock for TestClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        if self.fail_next.replace(false) {
            return Err(TimeError::BeforeUnixEpoch);
        }
        let at = self.at.get();
        let result = Observation {
            previous_at: Timestamp(self.previous),
            at: Timestamp(at),
            monotonic_elapsed_ms: (!self.broken).then_some(at.saturating_sub(self.previous)),
        };
        self.previous = at;
        self.broken = false;
        Ok(result)
    }
    fn break_continuity(&mut self) {
        self.broken = true;
    }
}

struct Notifier {
    log: Log,
    fail: Rc<Cell<bool>>,
}
impl CompletionNotifier for Notifier {
    fn session_completed(&mut self, _: SessionKind) -> Result<(), NotificationError> {
        self.log.borrow_mut().push("notify");
        if self.fail.get() {
            Err(NotificationError::UnsuccessfulExit(Some(1)))
        } else {
            Ok(())
        }
    }
}

type TestApp = App<Store, TestClock, Notifier>;
struct Harness {
    app: TestApp,
    at: Rc<Cell<u64>>,
    clock_failure: Rc<Cell<bool>>,
    failures: Rc<Cell<u32>>,
    fail_after: Rc<Cell<Option<u32>>>,
    notification_failure: Rc<Cell<bool>>,
    log: Log,
}

fn ready() -> DomainState {
    DomainState::new(TimerConfig::new(1, 1, 1, 2).unwrap()).unwrap()
}

fn harness(domain: DomainState, at: u64) -> Harness {
    let at = Rc::new(Cell::new(at));
    let clock_failure = Rc::new(Cell::new(false));
    let failures = Rc::new(Cell::new(0));
    let fail_after = Rc::new(Cell::new(None));
    let notification_failure = Rc::new(Cell::new(false));
    let log = Log::default();
    let controller = Controller::from_saved(
        Store {
            saved: domain,
            pending: None,
            failures: failures.clone(),
            fail_after: fail_after.clone(),
            log: log.clone(),
        },
        TestClock {
            at: at.clone(),
            previous: at.get(),
            broken: false,
            fail_next: clock_failure.clone(),
        },
        Notifier {
            log: log.clone(),
            fail: notification_failure.clone(),
        },
    )
    .unwrap();
    Harness {
        app: App::new(controller),
        at,
        clock_failure,
        failures,
        fail_after,
        notification_failure,
        log,
    }
}

fn press(app: &mut TestApp, key: char) {
    app.handle_key(KeyCode::Char(key));
}
fn render(app: &TestApp, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += u16::try_from(ratatui::text::Span::raw(symbol).width().max(1)).unwrap();
        }
        text.push('\n');
    }
    text
}
fn active_id(app: &TestApp) -> SessionId {
    let ProgressState::Active { session, .. } = &app.state().snapshot().state else {
        panic!("active")
    };
    session.id
}
fn assert_running(app: &TestApp) {
    assert!(matches!(
        app.state().snapshot().state,
        ProgressState::Active {
            timer: TimerState::Running { .. },
            ..
        }
    ));
}

#[test]
fn existing_keys_use_commands_and_reset_starts_a_distinct_session() {
    let mut h = harness(ready(), 0);
    let ready_display = render(&h.app, 100, 30);
    for hint in [
        "Space: Start",
        "r: Reset",
        "n: Skip",
        "s: Settings",
        "?: Help",
        "q: Save & quit",
    ] {
        assert!(ready_display.contains(hint), "{hint}: {ready_display}");
    }
    press(&mut h.app, '?');
    assert!(h.app.show_help());
    press(&mut h.app, '?');
    press(&mut h.app, ' ');
    let first = active_id(&h.app);
    let running_display = render(&h.app, 100, 30);
    assert!(running_display.contains("Space: Pause"));
    assert!(!running_display.contains("s: Settings"));
    h.at.set(100);
    press(&mut h.app, ' ');
    let paused_display = render(&h.app, 100, 30);
    assert!(paused_display.contains("Paused"));
    assert!(paused_display.contains("Space: Resume"));
    h.at.set(200);
    press(&mut h.app, ' ');
    assert_running(&h.app);
    press(&mut h.app, 'r');
    assert_eq!(
        h.app.state().history().sessions[0].end.unwrap().outcome,
        SessionOutcome::Reset
    );
    assert_eq!(h.app.state().history().sessions[0].elapsed_ms, 100);
    press(&mut h.app, ' ');
    assert_ne!(active_id(&h.app), first);
    press(&mut h.app, 'n');
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::ShortBreak,
            ..
        }
    ));
    press(&mut h.app, 'n');
    assert_eq!(h.app.state().history().sessions.len(), 2);
}

#[test]
fn settings_edit_cancel_and_save_only_from_ready() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 's');
    assert!(render(&h.app, 100, 30).contains("Enter: Save"));
    h.app.handle_key(KeyCode::Right);
    h.app.handle_key(KeyCode::Esc);
    assert_eq!(h.app.state().snapshot().settings.focus_seconds(), 1);
    press(&mut h.app, 's');
    h.app.handle_key(KeyCode::Right);
    h.app.handle_key(KeyCode::Down);
    h.app.handle_key(KeyCode::Right);
    h.app.handle_key(KeyCode::Enter);
    assert!(h.app.settings().is_none());
    assert_eq!(h.app.state().snapshot().settings.focus_seconds(), 61);
    assert_eq!(h.app.state().snapshot().settings.short_break_seconds(), 61);
    assert_eq!(h.app.state(), h.app.controller.saved_state());
    press(&mut h.app, ' ');
    press(&mut h.app, 's');
    assert!(h.app.settings().is_none());
    assert!(h.app.message().contains("while ready"));
    press(&mut h.app, ' ');
    press(&mut h.app, 's');
    assert!(h.app.settings().is_none());
}

#[test]
fn settings_input_consumes_normal_keys_until_cancelled() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 's');
    let before = h.app.state().clone();
    for key in ['q', 'Q', 'r', 'n', ' ', '?'] {
        press(&mut h.app, key);
    }
    assert!(h.app.settings().is_some());
    assert_eq!(h.app.state(), &before);
    assert!(!h.app.show_help());
    assert!(!h.app.should_quit());
    assert!(h.log.borrow().is_empty());

    h.app.handle_key(KeyCode::Esc);
    assert!(h.app.settings().is_none());
    press(&mut h.app, '?');
    assert!(h.app.show_help());
    press(&mut h.app, 'q');
    assert!(h.app.should_quit());
}

#[test]
fn settings_failure_displays_old_settings_until_retry_succeeds() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, 's');
    h.app.handle_key(KeyCode::Right);
    h.failures.set(1);
    h.app.handle_key(KeyCode::Enter);
    assert!(h.app.settings().is_none());
    assert_eq!(h.app.state().snapshot().settings.focus_seconds(), 1);
    assert!(!h.app.message().contains("Settings saved"));
    press(&mut h.app, 'r');
    assert_eq!(h.app.state().snapshot().settings.focus_seconds(), 61);
    assert!(h.app.message().contains("Settings saved"));
}

#[test]
fn completion_boundary_discards_input_and_notifies_after_save() {
    for key in [' ', 'r', 'n'] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, ' ');
        h.log.borrow_mut().clear();
        h.at.set(1_000);
        press(&mut h.app, key);
        assert!(matches!(
            h.app.state().snapshot().state,
            ProgressState::Ready {
                next_kind: SessionKind::ShortBreak,
                ..
            }
        ));
        assert_eq!(*h.log.borrow(), ["saved", "notify"]);
        assert_eq!(h.app.state().history().sessions.len(), 1);
        assert!(h.app.message().contains("complete"));
    }
}

#[test]
fn failed_completion_blocks_input_and_ticks_until_single_saved_notification() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.log.borrow_mut().clear();
    h.at.set(1_000);
    h.failures.set(1);
    h.app.tick();
    assert_eq!(*h.log.borrow(), ["failed"]);
    assert!(!h.app.message().contains("complete"));
    let pending = h.app.pending_state().unwrap().clone();
    for key in [' ', 'n', 's', 'q', 'c', 'f', '?'] {
        press(&mut h.app, key);
    }
    assert!(!h.app.show_help());
    press(&mut h.app, 'Q');
    assert!(h.app.confirming_unsaved_exit());
    for key in ['r', 'q', '?', ' '] {
        press(&mut h.app, key);
    }
    assert!(h.app.confirming_unsaved_exit());
    assert_eq!(*h.log.borrow(), ["failed"]);
    press(&mut h.app, 'n');
    assert!(!h.app.confirming_unsaved_exit());
    h.at.set(1_100);
    h.app.tick();
    assert_eq!(h.app.pending_state(), Some(&pending));
    assert!(!h.app.should_quit());
    let display = render(&h.app, 100, 30);
    assert!(display.contains("Save pending"));
    assert!(display.contains("Unconfirmed save"));
    assert!(display.contains("r: Retry save"));
    assert!(!display.contains("Space: Pause"));
    assert!(!display.contains("q: Save & quit"));
    h.notification_failure.set(true);
    press(&mut h.app, 'r');
    assert_eq!(*h.log.borrow(), ["failed", "saved", "notify"]);
    assert!(h.app.pending_state().is_none());
    assert!(h.app.message().contains("Notification failed"));
    h.app.tick();
    assert_eq!(*h.log.borrow(), ["failed", "saved", "notify"]);
}

#[test]
fn shutdown_clock_failure_accepts_only_retry_or_unsaved_exit() {
    let mut h = harness(ready(), 0);
    h.clock_failure.set(true);
    press(&mut h.app, 'q');
    assert!(h.app.shutdown_failed());
    assert!(h.app.pending_state().is_none());

    for key in ['q', 'n', 's', ' ', '?'] {
        press(&mut h.app, key);
    }
    assert!(h.app.shutdown_failed());
    assert!(!h.app.show_help());
    assert!(!h.app.should_quit());
    assert!(h.log.borrow().is_empty());

    press(&mut h.app, 'Q');
    assert!(h.app.confirming_unsaved_exit());
    press(&mut h.app, 'r');
    assert!(h.app.confirming_unsaved_exit());
    h.app.handle_key(KeyCode::Esc);
    assert!(!h.app.confirming_unsaved_exit());
    press(&mut h.app, 'r');
    assert!(h.app.should_quit());
    assert_eq!(h.app.exit(), ExitOutcome::Saved);
}

#[test]
fn retry_keeps_operations_blocked_through_the_second_gap_save() {
    let mut h = harness(ready(), 0);
    h.failures.set(1);
    press(&mut h.app, ' ');
    h.at.set(100);
    // Retry the operation successfully, then fail the recovery gap save.
    h.fail_after.set(Some(1));
    press(&mut h.app, 'r');
    assert!(h.app.pending_state().is_some());
    press(&mut h.app, ' ');
    press(&mut h.app, 'r');
    assert!(h.app.pending_state().is_none());
    assert!(render(&h.app, 100, 30).contains("Timing gap · Awaiting Resume"));
    assert_eq!(
        h.app
            .state()
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::ObservationGapDetected { .. }))
            .count(),
        1
    );
}

#[test]
fn shutdown_failure_requires_retry_or_confirmed_unsaved_exit() {
    for save in [true, false] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, ' ');
        h.failures.set(1);
        press(&mut h.app, 'q');
        assert!(!h.app.should_quit());
        press(&mut h.app, 'Q');
        let display = render(&h.app, 100, 30);
        assert!(display.contains("Exit without saving"));
        assert!(display.contains("y: Exit unsaved"));
        assert!(!display.contains("r: Retry save"));
        assert!(!display.contains("q: Save & quit"));
        h.app.handle_key(KeyCode::Esc);
        assert!(!h.app.confirming_unsaved_exit());
        assert!(!h.app.should_quit());
        if save {
            press(&mut h.app, 'r');
        } else {
            press(&mut h.app, 'Q');
            press(&mut h.app, 'y');
        }
        assert!(h.app.should_quit());
        assert_eq!(
            h.app.exit(),
            if save {
                ExitOutcome::Saved
            } else {
                ExitOutcome::Unsaved
            }
        );
    }
}

fn started(kind: SessionKind) -> DomainState {
    let initial = ready();
    let mut snapshot = initial.snapshot().clone();
    snapshot.state = ProgressState::Ready {
        next_kind: kind,
        current_task_draft: None,
    };
    let mut domain = DomainState::from_parts(
        snapshot,
        initial.history().clone(),
        initial.id_allocators().clone(),
    )
    .unwrap();
    if kind.is_work() {
        domain
            .apply(
                Command::SetCurrentTask(CurrentTask::parse("原稿を書く").unwrap()),
                Timestamp(0),
            )
            .unwrap();
    }
    domain.apply(Command::Start(kind), Timestamp(0)).unwrap();
    domain
}

fn awaiting() -> DomainState {
    let mut domain = started(SessionKind::QuickStart);
    for at in (1_000..=120_000).step_by(1_000) {
        domain
            .observe(Observation {
                previous_at: Timestamp(at - 1_000),
                at: Timestamp(at),
                monotonic_elapsed_ms: Some(1_000),
            })
            .unwrap();
    }
    domain
}

#[test]
fn all_restored_interruption_kinds_render_and_offer_the_correct_resume_command() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        for interruption_kind in [
            InterruptionKind::Pause,
            InterruptionKind::Distraction,
            InterruptionKind::AppExit,
            InterruptionKind::ObservationGap,
        ] {
            if interruption_kind == InterruptionKind::Distraction && !kind.is_work() {
                continue;
            }
            let mut domain = started(kind);
            match interruption_kind {
                InterruptionKind::Pause => domain
                    .apply(Command::Pause(SessionId(1)), Timestamp(0))
                    .unwrap(),
                InterruptionKind::Distraction => domain
                    .apply(Command::Distraction(SessionId(1)), Timestamp(0))
                    .unwrap(),
                InterruptionKind::AppExit => domain.apply(Command::CloseApp, Timestamp(0)).unwrap(),
                InterruptionKind::ObservationGap => domain
                    .observe(Observation {
                        previous_at: Timestamp(0),
                        at: Timestamp(10),
                        monotonic_elapsed_ms: None,
                    })
                    .unwrap(),
            }
            domain.apply(Command::RestoreApp, Timestamp(100)).unwrap();
            let mut h = harness(domain, 100);
            let before = h.app.state().clone();
            let display = render(&h.app, 100, 30);
            let label = match interruption_kind {
                InterruptionKind::Pause => "Paused",
                InterruptionKind::Distraction => "Awaiting Return",
                InterruptionKind::AppExit => "Stopped on exit",
                InterruptionKind::ObservationGap => "Timing gap",
            };
            assert!(display.contains(label), "{display}");
            let space_hint = if interruption_kind == InterruptionKind::Distraction {
                "Space: Return"
            } else {
                "Space: Resume"
            };
            assert!(display.contains(space_hint), "{display}");
            assert!(!display.contains("s: Settings"));
            assert!(display.contains("Total:"));
            assert!(!display.contains("Today:"));
            if kind.is_work() {
                assert!(display.contains("原稿を書く"));
            }
            for size in [(0, 0), (12, 5), (50, 16)] {
                render(&h.app, size.0, size.1);
            }
            assert_eq!(h.app.state(), &before);
            press(&mut h.app, 's');
            assert!(h.app.settings().is_none());
            h.at.set(200);
            press(&mut h.app, ' ');
            assert_running(&h.app);
            assert_eq!(active_id(&h.app), SessionId(1));
            let outcome = h
                .app
                .state()
                .history()
                .events
                .iter()
                .find_map(|event| match event.payload {
                    EventKind::InterruptionEnded { end, .. } => Some(end.outcome),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                outcome,
                if interruption_kind == InterruptionKind::Distraction {
                    pomodoro_core::InterruptionOutcome::Returned
                } else {
                    pomodoro_core::InterruptionOutcome::Resumed
                }
            );
        }
    }
}

#[test]
fn restored_running_sessions_wait_for_explicit_resume_and_do_not_credit_downtime() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut domain = started(kind);
        domain
            .apply(Command::RestoreApp, Timestamp(10_000))
            .unwrap();
        let mut h = harness(domain, 10_000);
        assert!(render(&h.app, 100, 30).contains("Timing gap · Awaiting Resume"));
        h.at.set(20_000);
        h.app.tick();
        let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
            panic!("active")
        };
        assert_eq!(session.elapsed_ms, 0);
        press(&mut h.app, ' ');
        assert_running(&h.app);
    }
}

#[test]
fn restored_ready_kinds_draw_and_start_without_creating_history_on_render() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut domain = started(kind);
        domain
            .apply(
                Command::End {
                    session_id: SessionId(1),
                    outcome: SessionOutcome::Reset,
                },
                Timestamp(0),
            )
            .unwrap();
        domain.apply(Command::RestoreApp, Timestamp(100)).unwrap();
        let mut h = harness(domain, 100);
        let display = render(&h.app, 100, 30);
        assert!(display.contains("Ready"));
        assert!(display.contains("Space: Start"));
        assert!(display.contains("s: Settings"));
        assert!(h.log.borrow().is_empty());
        press(&mut h.app, ' ');
        let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
            panic!("active")
        };
        assert_eq!(session.kind, kind);
    }
}

#[test]
fn restored_quick_start_choice_requires_explicit_finish_or_continue_and_can_quit() {
    for key in ['f', 'c', 'q'] {
        let mut domain = awaiting();
        domain
            .apply(Command::RestoreApp, Timestamp(130_000))
            .unwrap();
        let mut h = harness(domain, 130_000);
        let display = render(&h.app, 100, 30);
        assert!(display.contains("Choose finish or continue"));
        assert!(display.contains("原稿を書く"));
        assert!(display.contains("f: Finish Quick Start"));
        assert!(display.contains("c: Continue to Focus"));
        assert!(display.contains("q: Save & quit"));
        for unavailable in ["Space:", "r: Reset", "n: Skip", "s: Settings"] {
            assert!(!display.contains(unavailable), "{unavailable}: {display}");
        }
        press(&mut h.app, 's');
        assert!(h.app.settings().is_none());
        for ignored in [' ', 'r', 'n'] {
            press(&mut h.app, ignored);
        }
        assert!(matches!(
            h.app.state().snapshot().state,
            ProgressState::AwaitingQuickStartDecision { .. }
        ));
        h.at.set(140_000);
        press(&mut h.app, key);
        match key {
            'f' => assert!(matches!(
                h.app.state().snapshot().state,
                ProgressState::Ready {
                    next_kind: SessionKind::Focus,
                    ..
                }
            )),
            'c' => {
                assert_running(&h.app);
                let ProgressState::Active { session, .. } = &h.app.state().snapshot().state else {
                    unreachable!()
                };
                assert_eq!(session.continued_from_quick_start, Some(SessionId(1)));
                assert_eq!(session.elapsed_ms, 0);
                assert_eq!(
                    session.current_task.as_ref().unwrap().as_str(),
                    "原稿を書く"
                );
            }
            'q' => {
                assert!(h.app.should_quit());
                assert!(matches!(
                    h.app.state().snapshot().state,
                    ProgressState::AwaitingQuickStartDecision { .. }
                ));
                assert_eq!(h.app.exit(), ExitOutcome::Saved);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn settings_preserve_ready_kind_and_history_while_resetting_round_progress() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, ' ');
    h.at.set(1_000);
    h.app.tick();
    assert_eq!(
        h.app
            .state()
            .snapshot()
            .round_progress
            .completed_focuses_in_round,
        1
    );
    let history = h.app.state().history().clone();
    press(&mut h.app, 's');
    h.app.handle_key(KeyCode::Enter);
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::Ready {
            next_kind: SessionKind::ShortBreak,
            ..
        }
    ));
    assert_eq!(
        h.app
            .state()
            .snapshot()
            .round_progress
            .completed_focuses_in_round,
        0
    );
    assert_eq!(h.app.state().history(), &history);
}

#[test]
fn settings_support_restored_values_outside_the_old_editor_limits() {
    let domain = DomainState::new(TimerConfig::new(1, 1, 1, 100).unwrap()).unwrap();
    let mut h = harness(domain, 0);
    press(&mut h.app, 's');
    h.app.handle_key(KeyCode::Left);
    for _ in 0..3 {
        h.app.handle_key(KeyCode::Down);
    }
    h.app.handle_key(KeyCode::Right);
    h.app.handle_key(KeyCode::Enter);
    assert_eq!(h.app.state().snapshot().settings.focus_seconds(), 1);
    assert_eq!(
        h.app
            .state()
            .snapshot()
            .settings
            .focuses_before_long_break(),
        101
    );
}

#[test]
fn quick_start_continue_retry_does_not_create_another_focus_or_repeat_decision() {
    let mut h = harness(awaiting(), 120_000);
    h.failures.set(1);
    press(&mut h.app, 'c');
    assert!(matches!(
        h.app.state().snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    assert!(!h.app.message().contains("choice saved"));
    let candidate = h.app.pending_state().unwrap().clone();
    press(&mut h.app, 'f');
    press(&mut h.app, 'c');
    assert_eq!(h.app.pending_state(), Some(&candidate));
    h.at.set(120_100);
    press(&mut h.app, 'r');
    let ProgressState::Active {
        session,
        timer: TimerState::Interrupted { interruption },
    } = &h.app.state().snapshot().state
    else {
        panic!("saved continuation must wait for resume")
    };
    assert_eq!(session.id, SessionId(2));
    assert_eq!(session.elapsed_ms, 0);
    assert_eq!(interruption.kind, InterruptionKind::ObservationGap);
    assert_eq!(
        h.app
            .state()
            .history()
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventKind::QuickStartDecisionMade { .. }))
            .count(),
        1
    );
    assert_eq!(*h.log.borrow(), ["failed", "saved", "saved"]);
}
