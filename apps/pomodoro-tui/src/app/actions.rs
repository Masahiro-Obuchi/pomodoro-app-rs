use crossterm::event::KeyCode;
use pomodoro_core::{
    Command, InterruptionKind, PomodoroState, ProgressState, QuickStartChoice, SessionKind,
    SessionOutcome, TimerState,
};

pub(super) enum NormalAction {
    Command(Command),
    RequestTask,
    RequestSettings,
    ToggleHelp,
    Shutdown,
}

struct Binding {
    key: KeyCode,
    hint: Option<&'static str>,
    action: NormalAction,
}

impl Binding {
    fn new(key: char, hint: Option<&'static str>, action: NormalAction) -> Self {
        Self {
            key: KeyCode::Char(key),
            hint,
            action,
        }
    }

    fn command(key: char, hint: &'static str, command: Command) -> Self {
        Self::new(key, Some(hint), NormalAction::Command(command))
    }
}

/// Keys and visible hints for the snapshot currently on screen. Resolve this
/// before the controller observes time, so completion never retargets an input.
pub(super) struct NormalControls {
    session: Vec<Binding>,
    common: Vec<Binding>,
}

impl NormalControls {
    pub(super) fn for_snapshot(snapshot: &PomodoroState) -> Self {
        let session = match &snapshot.state {
            ProgressState::Ready { next_kind, .. } => {
                let mut bindings = vec![
                    Binding::command(' ', "Space: Start", Command::Start(*next_kind)),
                    Binding::command('r', "r: Reset", Command::ResetReady),
                    Binding::command('n', "n: Skip", Command::SkipReady),
                ];
                if *next_kind == SessionKind::Focus {
                    bindings.push(Binding::new(
                        't',
                        Some("t: Edit task"),
                        NormalAction::RequestTask,
                    ));
                    bindings.push(Binding::command(
                        '2',
                        "2: Quick Start (2 min)",
                        Command::Start(SessionKind::QuickStart),
                    ));
                }
                bindings
            }
            ProgressState::Active { session, timer } => {
                let (space_hint, space_command) = match timer {
                    TimerState::Running { .. } => ("Space: Pause", Command::Pause(session.id)),
                    TimerState::Interrupted { interruption }
                        if interruption.kind == InterruptionKind::Distraction =>
                    {
                        ("Space: Return", Command::Return(session.id))
                    }
                    TimerState::Interrupted { .. } => {
                        ("Space: Resume", Command::Resume(session.id))
                    }
                };
                vec![
                    Binding::command(' ', space_hint, space_command),
                    Binding::command(
                        'r',
                        "r: Reset",
                        Command::End {
                            session_id: session.id,
                            outcome: SessionOutcome::Reset,
                        },
                    ),
                    Binding::command(
                        'n',
                        "n: Skip",
                        Command::End {
                            session_id: session.id,
                            outcome: SessionOutcome::Skipped,
                        },
                    ),
                ]
            }
            ProgressState::AwaitingQuickStartDecision {
                quick_start_session_id,
                ..
            } => vec![
                Binding::command(
                    'f',
                    "f: Finish Quick Start",
                    Command::DecideQuickStart {
                        session_id: *quick_start_session_id,
                        choice: QuickStartChoice::Finish,
                    },
                ),
                Binding::command(
                    'c',
                    "c: Continue to Focus",
                    Command::DecideQuickStart {
                        session_id: *quick_start_session_id,
                        choice: QuickStartChoice::Continue,
                    },
                ),
            ],
        };
        let settings_hint =
            matches!(snapshot.state, ProgressState::Ready { .. }).then_some("s: Settings");
        let common = vec![
            // Keep the existing explanation for s outside Ready, but do not
            // advertise settings as an available operation there.
            Binding::new('s', settings_hint, NormalAction::RequestSettings),
            Binding::new('?', Some("?: Help"), NormalAction::ToggleHelp),
            Binding::new('q', Some("q: Save & quit"), NormalAction::Shutdown),
        ];
        Self { session, common }
    }

    pub(super) fn action_for_key(self, key: KeyCode) -> Option<NormalAction> {
        self.session
            .into_iter()
            .chain(self.common)
            .find(|binding| binding.key == key)
            .map(|binding| binding.action)
    }

    pub(super) fn hint_lines(&self) -> [String; 2] {
        [hints(&self.session), hints(&self.common)]
    }
}

fn hints(bindings: &[Binding]) -> String {
    bindings
        .iter()
        .filter_map(|binding| binding.hint)
        .collect::<Vec<_>>()
        .join("   ")
}
