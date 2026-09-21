use std::{cell::RefCell, collections::VecDeque, io, rc::Rc};

use pomodoro_core::{
    DomainError, EventKind, InterruptionKind, QuickStartChoice, SessionId, TimeUncertainty,
    TimerConfig,
};
use pomodoro_platform::{SaveError, SaveStage, TimeError};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trace {
    Save(Timestamp),
    Retry,
    Saved,
    Notify(SessionKind),
}

type Log = Rc<RefCell<Vec<Trace>>>;

#[derive(Clone, Copy)]
enum Failure {
    BeforeCandidate,
    Pending,
    Uncertain,
}

struct Store {
    saved: Option<DomainState>,
    pending: Option<(DomainState, Timestamp)>,
    attempts: Vec<(DomainState, Timestamp)>,
    outcomes: VecDeque<Option<Failure>>,
    generation: u64,
    log: Log,
}

impl Store {
    fn attempt(&mut self) -> Result<(), SaveError> {
        let (domain, at) = self.pending.as_ref().expect("a candidate was prepared");
        self.attempts.push((domain.clone(), *at));
        match self.outcomes.pop_front().flatten() {
            Some(Failure::BeforeCandidate) => {
                self.pending = None;
                Err(SaveError::Conflict {
                    path: "state.json".into(),
                })
            }
            Some(failure) => Err(SaveError::Io {
                stage: if matches!(failure, Failure::Uncertain) {
                    SaveStage::SyncPrimaryDirectory
                } else {
                    SaveStage::WritePrimaryTemp
                },
                path: "state.json".into(),
                source: io::Error::other("injected save failure"),
            }),
            None => {
                self.saved = Some(self.pending.take().unwrap().0);
                self.generation += 1;
                self.log.borrow_mut().push(Trace::Saved);
                Ok(())
            }
        }
    }
}

impl SaveStore for Store {
    fn saved_domain(&self) -> Option<&DomainState> {
        self.saved.as_ref()
    }
    fn has_pending_save(&self) -> bool {
        self.pending.is_some()
    }
    fn save(&mut self, domain: &DomainState, at: Timestamp) -> Result<(), SaveError> {
        assert!(
            self.pending.is_none(),
            "must retry a store's existing candidate"
        );
        self.log.borrow_mut().push(Trace::Save(at));
        self.pending = Some((domain.clone(), at));
        self.attempt()
    }
    fn retry_pending(&mut self) -> Result<(), SaveError> {
        self.log.borrow_mut().push(Trace::Retry);
        self.attempt()
    }
}

struct TestClock {
    at: Timestamp,
    readings: VecDeque<Result<(Timestamp, Option<u64>), TimeError>>,
    broken: bool,
    samples: usize,
    breaks: usize,
}

impl TestClock {
    fn push(&mut self, at: u64) {
        self.readings
            .push_back(Ok((Timestamp(at), Some(at.saturating_sub(self.at.0)))));
    }
}

impl Clock for TestClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        self.samples += 1;
        let (at, elapsed) = self
            .readings
            .pop_front()
            .expect("test must supply a clock reading")?;
        let observation = Observation {
            previous_at: self.at,
            at,
            monotonic_elapsed_ms: if self.broken { None } else { elapsed },
        };
        self.at = at;
        self.broken = false;
        Ok(observation)
    }
    fn break_continuity(&mut self) {
        self.broken = true;
        self.breaks += 1;
    }
}

struct Notifier {
    log: Log,
    fail: bool,
}

impl CompletionNotifier for Notifier {
    fn session_completed(&mut self, kind: SessionKind) -> Result<(), NotificationError> {
        self.log.borrow_mut().push(Trace::Notify(kind));
        if self.fail {
            Err(NotificationError::UnsuccessfulExit(Some(1)))
        } else {
            Ok(())
        }
    }
}

type TestController = Controller<Store, TestClock, Notifier>;

fn ready() -> DomainState {
    DomainState::new(TimerConfig::new(10, 5, 20, 2).unwrap()).unwrap()
}

fn controller(domain: DomainState, at: u64) -> TestController {
    let log = Rc::new(RefCell::new(Vec::new()));
    Controller::from_saved(
        Store {
            saved: Some(domain),
            pending: None,
            attempts: Vec::new(),
            outcomes: VecDeque::new(),
            generation: 1,
            log: log.clone(),
        },
        TestClock {
            at: Timestamp(at),
            readings: VecDeque::new(),
            broken: false,
            samples: 0,
            breaks: 0,
        },
        Notifier { log, fail: false },
    )
    .unwrap()
}

fn running() -> TestController {
    let mut domain = ready();
    domain
        .apply(Command::Start(SessionKind::Focus), Timestamp(0))
        .unwrap();
    controller(domain, 0)
}

fn elapsed(domain: &DomainState) -> u64 {
    let ProgressState::Active { session, .. } = &domain.snapshot().state else {
        panic!("expected active");
    };
    session.elapsed_ms
}

fn gap_count(domain: &DomainState) -> usize {
    domain
        .history()
        .events
        .iter()
        .filter(|event| matches!(event.payload, EventKind::ObservationGapDetected { .. }))
        .count()
}

fn assert_valid(domain: &DomainState) {
    DomainState::from_parts(
        domain.snapshot().clone(),
        domain.history().clone(),
        domain.id_allocators().clone(),
    )
    .unwrap();
}

mod operations;
mod recovery;
