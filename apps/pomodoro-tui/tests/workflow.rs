#![cfg(target_os = "linux")]

use std::{cell::Cell, fs, path::Path, rc::Rc};

use crossterm::event::{Event, KeyCode};
use pomodoro_core::{
    DomainState, Observation, ProgressState, SessionKind, SessionOutcome, TimerConfig, Timestamp,
};
use pomodoro_platform::{LoadOutcome, StorageLocation, TimeError};
use pomodoro_tui::{
    app::App,
    controller::{Clock, Controller, ExitOutcome},
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
    let copy = tempfile::tempdir().unwrap();
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

#[test]
fn quick_start_completion_and_choice_match_saved_file_and_notification() {
    for choice in ['f', 'c'] {
        let directory = tempfile::tempdir().unwrap();
        let location = StorageLocation::at(directory.path().to_owned());
        let LoadOutcome::New(mut store) = location.clone().lock().unwrap().load().unwrap() else {
            panic!("new store expected")
        };
        store
            .save(
                &DomainState::new(TimerConfig::new(30, 1, 1, 2).unwrap()).unwrap(),
                Timestamp(0),
            )
            .unwrap();
        let state_path = location.state_path().clone();
        let alert_count = Rc::new(Cell::new(0));
        let alerts = alert_count.clone();
        let notifier = move |kind| {
            assert_eq!(kind, SessionKind::QuickStart);
            let saved = decode_saved_bytes(&state_path);
            assert!(matches!(
                saved.snapshot().state,
                ProgressState::AwaitingQuickStartDecision { .. }
            ));
            assert_eq!(
                saved.history().sessions[0].end.unwrap().outcome,
                SessionOutcome::Completed
            );
            alerts.set(alerts.get() + 1);
            Ok(())
        };
        let samples: std::collections::VecDeque<u64> = [0, 0]
            .into_iter()
            .chain((1_000..=120_000).step_by(1_000))
            .chain([120_000, 120_000]) // The choice and shutdown each observe once.
            .collect();
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

        app.handle_key(KeyCode::Char(choice));
        assert_eq!(decode_saved_bytes(&location.state_path()), *app.state());
        assert_eq!(alert_count.get(), 1);
        assert_saved_choice(&app, choice);
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
            120_000
        );
    }
}
