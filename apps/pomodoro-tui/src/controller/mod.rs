//! Save-confirmed operations and lifecycle. Terminal keys and confirmation UX
//! are connected separately; core remains the only timing policy.

mod error;
mod ports;
mod shutdown;
mod startup;

pub use error::ControllerError;
pub use ports::{Clock, CompletionNotifier, SaveStore};
pub use shutdown::ExitOutcome;
pub use startup::{Startup, StartupError, StartupSave, StartupSaveError};

use pomodoro_core::{
    Command, DomainState, Observation, PomodoroState, ProgressState, SessionKind, SessionOutcome,
    TimerState, Timestamp,
};
use pomodoro_platform::NotificationError;

// Application scheduling policy, separate from core's observation-gap policy.
const CHECKPOINT_INTERVAL_MS: u64 = 5_000;

/// An operation may be presented as successful only after this report exists.
/// `command == None` on execute means an observed transition took precedence:
/// the input was not applied and is never queued for a later session.
#[derive(Debug)]
pub struct Commit {
    pub command: Option<Command>,
    pub completed: Option<SessionKind>,
    pub notification_error: Option<NotificationError>,
}

#[derive(Debug, Default)]
struct Effects {
    command: Option<Command>,
    completed: Option<SessionKind>,
}

#[derive(Debug)]
struct PendingCommit {
    at: Timestamp,
    effects: Effects,
    kind: SaveKind,
    recover_after_save: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveKind {
    Operation,
    Recovery,
    Shutdown,
}

#[derive(Debug)]
enum Mode {
    Operating,
    Saving(PendingCommit),
    Recovering(Effects),
    Closed,
}

/// Owns the save handle for the lifetime of normal application operation.
/// The store owns the last durable state; `domain` is either live checkpointable
/// state or a fixed pending candidate, as determined by `mode`.
pub struct Controller<S, C, N> {
    store: S,
    clock: C,
    notifier: N,
    domain: DomainState,
    mode: Mode,
    dirty: bool,
    checkpoint_elapsed_ms: u64,
}

impl<S: SaveStore, C: Clock, N: CompletionNotifier> Controller<S, C, N> {
    /// Adopts an already saved application state. Startup must perform any
    /// required `RestoreApp` and save it before calling this constructor.
    /// This never initializes state after a failed load or adopts a pending save.
    ///
    /// # Errors
    /// Rejects a store without a durable state or with a pending candidate.
    pub fn from_saved(store: S, clock: C, notifier: N) -> Result<Self, ControllerError> {
        if store.has_pending_save() {
            return Err(ControllerError::StorageAlreadyPending);
        }
        let domain = store
            .saved_domain()
            .ok_or(ControllerError::MissingSavedState)?
            .clone();
        Ok(Self {
            store,
            clock,
            notifier,
            domain,
            mode: Mode::Operating,
            dirty: false,
            checkpoint_elapsed_ms: 0,
        })
    }

    /// Normal display state. A failed meaningful transition is shown separately
    /// as pending, never as a successful operation.
    #[must_use]
    pub fn state(&self) -> &DomainState {
        if self.is_save_pending() {
            self.saved_state()
        } else {
            &self.domain
        }
    }

    /// Last durable state, including while a newer candidate is pending.
    ///
    /// # Panics
    /// Panics if a custom [`SaveStore`] violates its contract by discarding the
    /// baseline after this controller has adopted it.
    #[must_use]
    pub fn saved_state(&self) -> &DomainState {
        self.store
            .saved_domain()
            .expect("controller always has a saved baseline")
    }

    /// Candidate for an explicit pending-save display, not a success indicator.
    #[must_use]
    pub fn pending_state(&self) -> Option<&DomainState> {
        self.is_save_pending().then_some(&self.domain)
    }

    #[must_use]
    pub fn is_save_pending(&self) -> bool {
        matches!(self.mode, Mode::Saving(_) | Mode::Recovering(_))
    }

    /// Observes time. Ordinary ticks only dirty the live state; meaningful
    /// transitions and due checkpoints are saved immediately.
    ///
    /// # Errors
    /// Blocks during save recovery; clock/domain/save failures are reported.
    pub fn tick(&mut self) -> Result<Option<Commit>, ControllerError> {
        self.observe_and_checkpoint(false)
    }

    /// Observes time and saves any outstanding checkpoint, even before its due time.
    ///
    /// # Errors
    /// Same failure and pending-save behavior as [`Self::tick`].
    pub fn checkpoint(&mut self) -> Result<Option<Commit>, ControllerError> {
        self.observe_and_checkpoint(true)
    }

    /// Observes before applying an input. If observation itself causes a durable
    /// transition, saves that transition and discards this input. Session IDs and
    /// operations are never recomputed against a newly completed session.
    /// A no-op succeeds without saving when there are no outstanding changes.
    /// `CloseApp` uses [`Self::shutdown`], including its terminal closed state.
    ///
    /// # Errors
    /// Reports blocked input or clock/domain/save failures. Failed saves retain
    /// the candidate; retry never executes the input a second time.
    pub fn execute(&mut self, command: Command) -> Result<Commit, ControllerError> {
        if command == Command::CloseApp {
            return self.shutdown();
        }
        self.require_operating()?;
        let observation = self.sample()?;
        let (meaningful, effects) = self.apply_observation(observation)?;
        if meaningful {
            self.begin_save(observation.at, effects, SaveKind::Operation);
            return self.flush();
        }
        // Domain transitions only append history; avoid cloning the full history
        // to detect no-ops, while checking snapshot and allocation changes too.
        let before = self.domain.snapshot().clone();
        let ids = self.domain.id_allocators().clone();
        let sessions = self.domain.history().sessions.len();
        let events = self.domain.history().events.len();
        self.domain
            .apply(command.clone(), observation.at)
            .map_err(ControllerError::Domain)?;
        self.dirty |= &before != self.domain.snapshot()
            || &ids != self.domain.id_allocators()
            || sessions != self.domain.history().sessions.len()
            || events != self.domain.history().events.len();
        let effects = Effects {
            command: Some(command),
            completed: None,
        };
        if !self.dirty {
            return Ok(self.finish(effects));
        }
        self.begin_save(observation.at, effects, SaveKind::Operation);
        self.flush()
    }

    /// Retries the fixed candidate, then persists a recovery observation before
    /// normal input becomes available. A second save failure retains that gap
    /// candidate; subsequent retries do not generate the gap or effects again.
    ///
    /// # Errors
    /// Reports no pending operation, clock/domain failure during recovery, or
    /// save failure. The controller remains blocked until recovery completes.
    pub fn retry(&mut self) -> Result<Commit, ControllerError> {
        if !self.is_save_pending() {
            return Err(ControllerError::NoPendingSave);
        }
        self.flush()
    }

    fn require_operating(&self) -> Result<(), ControllerError> {
        if matches!(self.mode, Mode::Closed) {
            Err(ControllerError::Closed)
        } else if self.is_save_pending() {
            Err(ControllerError::SavePending)
        } else {
            Ok(())
        }
    }

    fn sample(&mut self) -> Result<Observation, ControllerError> {
        self.clock.observe().map_err(|error| {
            self.clock.break_continuity();
            ControllerError::Time(error)
        })
    }

    fn observe_and_checkpoint(&mut self, force: bool) -> Result<Option<Commit>, ControllerError> {
        self.require_operating()?;
        let observation = self.sample()?;
        let (meaningful, effects) = self.apply_observation(observation)?;
        if self.dirty
            && (force || meaningful || self.checkpoint_elapsed_ms >= CHECKPOINT_INTERVAL_MS)
        {
            self.begin_save(observation.at, effects, SaveKind::Operation);
            return self.flush().map(Some);
        }
        Ok(None)
    }

    fn apply_observation(
        &mut self,
        observation: Observation,
    ) -> Result<(bool, Effects), ControllerError> {
        // Only the small snapshot is cloned on a tick, never the event history.
        let before = self.domain.snapshot().clone();
        let sessions = self.domain.history().sessions.len();
        let events = self.domain.history().events.len();
        self.domain.observe(observation).map_err(|error| {
            self.clock.break_continuity();
            ControllerError::Domain(error)
        })?;
        let history_changed = sessions != self.domain.history().sessions.len()
            || events != self.domain.history().events.len();
        let snapshot_changed = &before != self.domain.snapshot();
        self.dirty |= history_changed || snapshot_changed;
        self.checkpoint_elapsed_ms = self
            .checkpoint_elapsed_ms
            .saturating_add(observation.monotonic_elapsed_ms.unwrap_or(0));
        // Normal running observations change only elapsed/last-confirmed fields.
        // An interrupted snapshot can change uncertainty with no new event.
        let ordinary_running = is_running(&before) && is_running(self.domain.snapshot());
        let meaningful = history_changed || (snapshot_changed && !ordinary_running);
        let completed = self
            .domain
            .history()
            .sessions
            .get(sessions)
            .filter(|session| {
                session
                    .end
                    .is_some_and(|end| end.outcome == SessionOutcome::Completed)
            })
            .map(|session| session.kind);
        Ok((
            meaningful,
            Effects {
                command: None,
                completed,
            },
        ))
    }

    fn begin_save(&mut self, at: Timestamp, effects: Effects, kind: SaveKind) {
        self.mode = Mode::Saving(PendingCommit {
            at,
            effects,
            kind,
            recover_after_save: false,
        });
    }

    fn flush(&mut self) -> Result<Commit, ControllerError> {
        loop {
            if matches!(self.mode, Mode::Recovering(_)) {
                if let Some(commit) = self.recover_observation()? {
                    return Ok(commit);
                }
            }
            let Mode::Saving(pending) = &mut self.mode else {
                unreachable!("flush requires pending work")
            };
            let result = if self.store.has_pending_save() {
                self.store.retry_pending()
            } else {
                // Save can fail before the store creates its fixed JSON candidate.
                // The application's domain and timestamp stay fixed in that case too.
                self.store.save(&self.domain, pending.at)
            };
            if let Err(error) = result {
                pending.recover_after_save |= pending.kind == SaveKind::Operation;
                self.clock.break_continuity();
                return Err(ControllerError::Save(error));
            }
            self.dirty = false;
            self.checkpoint_elapsed_ms = 0;
            let Mode::Saving(pending) = std::mem::replace(&mut self.mode, Mode::Operating) else {
                unreachable!()
            };
            if pending.recover_after_save {
                self.mode = Mode::Recovering(pending.effects);
            } else {
                if pending.kind == SaveKind::Shutdown {
                    self.mode = Mode::Closed;
                }
                return Ok(self.finish(pending.effects));
            }
        }
    }

    fn recover_observation(&mut self) -> Result<Option<Commit>, ControllerError> {
        let mut observation = self.sample()?;
        // This is lost continuity, even if a clock implementation supplies a delta.
        // Core chooses the boundary and preserves existing interruption kinds.
        observation.monotonic_elapsed_ms = None;
        let (changed, _) = self.apply_observation(observation)?;
        let Mode::Recovering(effects) = std::mem::replace(&mut self.mode, Mode::Operating) else {
            unreachable!()
        };
        if changed {
            self.begin_save(observation.at, effects, SaveKind::Recovery);
            Ok(None)
        } else {
            Ok(Some(self.finish(effects)))
        }
    }

    fn finish(&mut self, effects: Effects) -> Commit {
        let notification_error = effects
            .completed
            .and_then(|kind| self.notifier.session_completed(kind).err());
        Commit {
            command: effects.command,
            completed: effects.completed,
            notification_error,
        }
    }
}

fn is_running(snapshot: &PomodoroState) -> bool {
    matches!(
        snapshot.state,
        ProgressState::Active {
            timer: TimerState::Running { .. },
            ..
        }
    )
}

#[cfg(test)]
mod tests;
