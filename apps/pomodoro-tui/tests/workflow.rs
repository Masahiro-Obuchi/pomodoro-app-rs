#![cfg(any(target_os = "linux", target_os = "macos"))]

mod support;

use std::{cell::Cell, fs, path::Path, rc::Rc};

use crossterm::event::{Event, KeyCode};
use pomodoro_core::{
    DomainState, EventKind, Observation, ProgressState, QuickStartDecision, SessionId, SessionKind,
    SessionOutcome, TimerConfig, TimerState, Timestamp,
};
use pomodoro_platform::{LoadOutcome, StorageLocation, TimeError};
use pomodoro_tui::{
    app::App,
    controller::{Clock, Controller, ExitOutcome, Startup},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};

struct ScriptedClock {
    at: u64,
    samples: std::collections::VecDeque<u64>,
    broken: bool,
}

impl Clock for ScriptedClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        let at = self.samples.pop_front().expect("unexpected clock sample");
        let observation = Observation {
            previous_at: Timestamp(self.at),
            at: Timestamp(at),
            monotonic_elapsed_ms: (!self.broken).then_some(at - self.at),
        };
        self.at = at;
        self.broken = false;
        Ok(observation)
    }

    fn break_continuity(&mut self) {
        self.broken = true;
    }
}

fn decode_saved_bytes(path: &Path) -> DomainState {
    let copy = support::tempdir();
    let location = StorageLocation::at(copy.path().to_owned());
    fs::write(location.state_path(), fs::read(path).unwrap()).unwrap();
    let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
        panic!("expected valid V1 file")
    };
    store.saved_state().unwrap().domain().clone()
}

fn rendered<
    S: pomodoro_tui::controller::SaveStore,
    C: Clock,
    N: pomodoro_tui::controller::CompletionNotifier,
>(
    app: &App<S, C, N>,
) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..30)
        .map(|y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_saved_choice<
    S: pomodoro_tui::controller::SaveStore,
    C: Clock,
    N: pomodoro_tui::controller::CompletionNotifier,
>(
    app: &App<S, C, N>,
    choice: char,
) {
    assert_eq!(app.state().history().sessions.len(), 1);
    assert_eq!(
        app.state().history().sessions[0].end.unwrap().outcome,
        SessionOutcome::Completed
    );
    if choice == 'f' {
        let ProgressState::Ready {
            next_kind,
            current_task_draft,
        } = &app.state().snapshot().state
        else {
            panic!("expected Focus ready after Finish")
        };
        assert_eq!(*next_kind, SessionKind::Focus);
        assert_eq!(current_task_draft.as_ref().unwrap().as_str(), "write draft");
        assert!(rendered(app).contains("Focus · Ready"));
    } else {
        let ProgressState::Active { session, .. } = &app.state().snapshot().state else {
            panic!("expected linked Focus after Continue")
        };
        assert_eq!(session.kind, SessionKind::Focus);
        assert_eq!(session.started_at, Timestamp(130_000));
        assert_eq!(session.planned_duration_ms, 25 * 60_000);
        assert_eq!(session.elapsed_ms, 0);
        assert_eq!(
            session.current_task.as_ref().unwrap().as_str(),
            "write draft"
        );
        assert_eq!(
            session.continued_from_quick_start,
            Some(app.state().history().sessions[0].id)
        );
        assert!(rendered(app).contains("Focus · Running"));
    }
}

fn assert_saved_completion(kind: SessionKind, saved: &DomainState) {
    match kind {
        SessionKind::QuickStart => {
            assert!(matches!(
                saved.snapshot().state,
                ProgressState::AwaitingQuickStartDecision { .. }
            ));
            assert_eq!(saved.history().sessions.len(), 1);
            assert_eq!(saved.reflection().unwrap().work_ms, 120_000);
        }
        SessionKind::Focus => {
            assert!(matches!(
                saved.snapshot().state,
                ProgressState::Ready {
                    next_kind: SessionKind::ShortBreak,
                    ..
                }
            ));
            assert_eq!(saved.history().sessions.len(), 2);
            assert_eq!(saved.reflection().unwrap().work_ms, 27 * 60_000);
            assert_eq!(saved.reflection().unwrap().completed_focus_sessions, 1);
        }
        _ => panic!("work completion expected"),
    }
    assert_eq!(
        saved
            .history()
            .sessions
            .last()
            .unwrap()
            .end
            .unwrap()
            .outcome,
        SessionOutcome::Completed
    );
}

fn assert_continued_focus_completed<
    S: pomodoro_tui::controller::SaveStore,
    C: Clock,
    N: pomodoro_tui::controller::CompletionNotifier,
>(
    app: &mut App<S, C, N>,
    path: &Path,
    alert_count: &Cell<u32>,
) {
    for tick in 1_u64..=299 {
        app.tick();
        let ProgressState::Active {
            session,
            timer: TimerState::Running { .. },
        } = &app.state().snapshot().state
        else {
            panic!("Focus completed before its full 25 minutes at tick {tick}")
        };
        assert_eq!(session.kind, SessionKind::Focus);
        assert_eq!(session.elapsed_ms, tick * 5_000);
        assert_eq!(alert_count.get(), 1);
    }
    app.tick(); // The 300th five-second interval reaches the 25-minute boundary.
    assert_eq!(alert_count.get(), 2);
    let saved = decode_saved_bytes(path);
    assert_eq!(saved, *app.state());
    assert_eq!(saved.history().sessions.len(), 2);
    let quick_start = &saved.history().sessions[0];
    let focus = &saved.history().sessions[1];
    assert_eq!(quick_start.id, SessionId(1));
    assert_eq!(quick_start.kind, SessionKind::QuickStart);
    assert_eq!(quick_start.elapsed_ms, 120_000);
    assert_eq!(focus.id, SessionId(2));
    assert_eq!(focus.kind, SessionKind::Focus);
    assert_eq!(focus.continued_from_quick_start, Some(quick_start.id));
    assert_eq!(focus.planned_duration_ms, 25 * 60_000);
    assert_eq!(focus.elapsed_ms, 25 * 60_000);
    assert_eq!(focus.end.unwrap().outcome, SessionOutcome::Completed);
    assert_eq!(focus.end.unwrap().ended_at, Timestamp(1_630_000));
    assert_eq!(focus.current_task, quick_start.current_task);
    assert_eq!(
        saved
            .history()
            .events
            .iter()
            .filter(|event| matches!(
                event.payload,
                EventKind::QuickStartDecisionMade {
                    decision: QuickStartDecision::Continue {
                        focus_session_id: SessionId(2)
                    }
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        saved.snapshot().round_progress.completed_focuses_in_round,
        1
    );
    assert_eq!(saved.reflection().unwrap().work_ms, 27 * 60_000);
    assert_eq!(saved.reflection().unwrap().completed_focus_sessions, 1);
    app.handle_key(KeyCode::Char('h'));
    let history = rendered(app);
    assert!(history.contains("Recorded work: 0:27:00"), "{history}");
    assert!(history.contains("Focus completed: 1"), "{history}");
    app.handle_key(KeyCode::Char('h'));
}

#[test]
fn quick_start_completion_and_choice_match_saved_file_and_notification() {
    for choice in ['f', 'c'] {
        let directory = support::tempdir();
        let location = StorageLocation::at(directory.path().to_owned());
        let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
            panic!("new store expected")
        };
        store
            .save(
                &DomainState::new(TimerConfig::default()).unwrap(),
                Timestamp(0),
            )
            .unwrap();
        let state_path = location.state_path().clone();
        let alert_count = Rc::new(Cell::new(0));
        let alerts = alert_count.clone();
        let notifier = move |kind| {
            let saved = decode_saved_bytes(&state_path);
            assert_saved_completion(kind, &saved);
            alerts.set(alerts.get() + 1);
            Ok(())
        };
        let mut samples: std::collections::VecDeque<u64> = [0, 0]
            .into_iter()
            .chain((1_000..=120_000).step_by(1_000))
            .chain([125_000, 130_000]) // Wait for a choice, then choose.
            .collect();
        if choice == 'c' {
            samples.extend((135_000..=1_630_000).step_by(5_000));
            samples.push_back(1_630_000); // Shutdown after Focus completes.
        } else {
            samples.push_back(130_000); // Shutdown after Finish.
        }
        let controller = Controller::from_saved(
            store,
            ScriptedClock {
                at: 0,
                samples,
                broken: false,
            },
            notifier,
        )
        .unwrap();
        let mut app = App::new(controller);

        app.handle_key(KeyCode::Char('t'));
        app.handle_event(Event::Paste("write draft".into()));
        app.handle_key(KeyCode::Enter);
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        app.handle_key(KeyCode::Char('2'));
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        assert!(rendered(&app).contains("Quick Start · Running"));
        assert_eq!(alert_count.get(), 0);

        for _ in 0..120 {
            app.tick();
        }
        assert_eq!(alert_count.get(), 1);
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        assert!(rendered(&app).contains("Choose finish or continue"));
        assert_eq!(app.state().reflection().unwrap().work_ms, 120_000);
        app.tick(); // Choice time does not add work or create a Focus.
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        assert_eq!(app.state().reflection().unwrap().work_ms, 120_000);

        app.handle_key(KeyCode::Char(choice));
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        assert_eq!(alert_count.get(), 1);
        assert_saved_choice(&app, choice);
        if choice == 'c' {
            assert_continued_focus_completed(&mut app, &location.state_path(), &alert_count);
        }
        app.handle_key(KeyCode::Char('q'));
        assert!(app.should_quit());
        assert_eq!(app.exit(), ExitOutcome::Saved);
        let LoadOutcome::Loaded(store) = location.lock().unwrap().load().unwrap() else {
            panic!("expected persisted result")
        };
        assert_eq!(
            store
                .saved_state()
                .unwrap()
                .domain()
                .reflection()
                .unwrap()
                .work_ms,
            if choice == 'c' { 27 * 60_000 } else { 120_000 }
        );
    }
}

#[test]
fn cancelling_focus_keeps_work_but_not_a_completion_after_restart() {
    let directory = support::tempdir();
    let location = StorageLocation::at(directory.path().to_owned());
    let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
        panic!("new store expected")
    };
    store
        .save(
            &DomainState::new(TimerConfig::default()).unwrap(),
            Timestamp(0),
        )
        .unwrap();
    let samples = [0]
        .into_iter()
        .chain((1_000..=61_000).step_by(1_000))
        .chain([61_000, 61_000]) // Cancel and shut down.
        .collect();
    let controller = Controller::from_saved(
        store,
        ScriptedClock {
            at: 0,
            samples,
            broken: false,
        },
        |_| panic!("cancelled Focus must not notify completion"),
    )
    .unwrap();
    let mut app = App::new(controller);

    app.handle_key(KeyCode::Char(' '));
    for _ in 0..61 {
        app.tick();
    }
    app.handle_key(KeyCode::Char('x'));
    let saved = decode_saved_bytes(&location.state_path());
    assert_eq!(saved, *app.state());
    assert_eq!(saved.history().sessions.len(), 1);
    assert_eq!(saved.history().sessions[0].kind, SessionKind::Focus);
    assert_eq!(saved.history().sessions[0].elapsed_ms, 61_000);
    assert_eq!(
        saved.history().sessions[0].end.unwrap().outcome,
        SessionOutcome::Cancelled
    );
    assert_eq!(
        saved.snapshot().round_progress.completed_focuses_in_round,
        0
    );
    assert_eq!(saved.reflection().unwrap().work_ms, 61_000);
    assert_eq!(saved.reflection().unwrap().completed_focus_sessions, 0);
    app.handle_key(KeyCode::Char('h'));
    let history = rendered(&app);
    assert!(history.contains("Recorded work: 0:01:01"), "{history}");
    assert!(history.contains("Focus completed: 0"), "{history}");
    app.handle_key(KeyCode::Char('h'));
    app.handle_key(KeyCode::Char('q'));
    assert_eq!(app.exit(), ExitOutcome::Saved);

    let Startup::Saving(startup) =
        Startup::open(location.clone(), TimerConfig::default(), Timestamp(62_000)).unwrap()
    else {
        panic!("valid saved state must restart normally")
    };
    let store = startup.save().unwrap();
    assert_eq!(
        store
            .saved_state()
            .unwrap()
            .domain()
            .reflection()
            .unwrap()
            .work_ms,
        61_000
    );
    let controller = Controller::from_saved(
        store,
        ScriptedClock {
            at: 62_000,
            samples: std::collections::VecDeque::default(),
            broken: false,
        },
        |_| panic!("restart must not notify completion"),
    )
    .unwrap();
    let mut restarted = App::new(controller);
    restarted.handle_key(KeyCode::Char('h'));
    let history = rendered(&restarted);
    assert!(history.contains("Recorded work: 0:01:01"), "{history}");
    assert!(history.contains("Focus completed: 0"), "{history}");
    assert_eq!(restarted.state().history().sessions.len(), 1);
}
