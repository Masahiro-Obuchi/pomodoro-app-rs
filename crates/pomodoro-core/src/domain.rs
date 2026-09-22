use crate::{
    CurrentTask, DomainError, EventKind, GapReason, History, HistoryEvent, IdAllocators,
    Interruption, InterruptionEnd, InterruptionId, InterruptionKind, InterruptionOutcome,
    MeasuredDuration, Observation, PomodoroState, ProgressState, QuickStartChoice,
    QuickStartDecision, ReflectionSummary, RoundProgress, RunState, Session, SessionEnd, SessionId,
    SessionKind, SessionOutcome, TimeUncertainty, TimerConfig, TimerState, Timestamp,
};

/// User/lifecycle input, separate from clock observation and persistence.
/// Session-scoped commands carry an ID so a completion cannot retarget the same
/// input to the next session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SetCurrentTask(Option<CurrentTask>),
    Configure(TimerConfig),
    Start(SessionKind),
    Pause(SessionId),
    Distraction(SessionId),
    Resume(SessionId),
    Return(SessionId),
    End {
        session_id: SessionId,
        outcome: SessionOutcome,
    },
    ResetReady,
    SkipReady,
    DecideQuickStart {
        session_id: SessionId,
        choice: QuickStartChoice,
    },
    CloseApp,
    RestoreApp,
}

/// A domain unit: authoritative snapshot, analytical history, and ID allocation.
///
/// Applying a transition is NOT a durable commit. The application owns pending
/// and committed values, saving and notification. This type performs no I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainState {
    pub(crate) snapshot: PomodoroState,
    pub(crate) history: History,
    pub(crate) ids: IdAllocators,
}

impl DomainState {
    /// Creates domain values only; the storage layer decides whether a new store
    /// is allowed. A load error must never call this as a fallback.
    ///
    /// # Errors
    /// Returns an error for invalid settings.
    pub fn new(settings: TimerConfig) -> Result<Self, DomainError> {
        settings.validate()?;
        Ok(Self {
            snapshot: PomodoroState {
                settings,
                round_progress: RoundProgress::default(),
                state: ready(SessionKind::Focus, None),
            },
            history: History::default(),
            ids: IdAllocators::default(),
        })
    }

    /// Adopts decoded domain values directly, without replaying history.
    ///
    /// # Errors
    /// Rejects inconsistent values without repairing or resetting them.
    pub fn from_parts(
        snapshot: PomodoroState,
        history: History,
        ids: IdAllocators,
    ) -> Result<Self, DomainError> {
        let state = Self {
            snapshot,
            history,
            ids,
        };
        state.validate()?;
        Ok(state)
    }

    #[must_use]
    pub const fn snapshot(&self) -> &PomodoroState {
        &self.snapshot
    }

    #[must_use]
    pub const fn history(&self) -> &History {
        &self.history
    }

    #[must_use]
    pub const fn id_allocators(&self) -> &IdAllocators {
        &self.ids
    }

    /// Applies an input atomically to this in-memory candidate. Rejected inputs
    /// leave snapshot, history and allocation unchanged. Observe time first;
    /// a separate, already-applied completion is not rolled back on rejection.
    ///
    /// # Errors
    /// Returns an error for disallowed input, a stale target, missing observation
    /// or numeric overflow. Success does not mean the candidate has been saved.
    pub fn apply(&mut self, command: Command, at: Timestamp) -> Result<(), DomainError> {
        self.atomic(|state| state.apply_inner(command, at))
    }

    /// Credits only continuously observed monotonic time. A gap is handled before
    /// completion and never filled from a wall-clock deadline. Ordinary ticks do
    /// not emit events. Runtime clock anchors are owned by the caller.
    ///
    /// # Errors
    /// Returns an error on counter overflow without partially applying the tick.
    pub fn observe(&mut self, observation: Observation) -> Result<(), DomainError> {
        self.atomic(|state| state.observe_inner(observation))
    }

    /// Derives the minimal reflection view, including the active session, without
    /// introducing a second persisted aggregate.
    ///
    /// # Errors
    /// Returns an error if the total work duration exceeds its integer range.
    pub fn reflection(&self) -> Result<ReflectionSummary, DomainError> {
        let summary = self.history.reflection()?;
        match &self.snapshot.state {
            ProgressState::Active { session, .. } => summary.including_session(session),
            _ => Ok(summary),
        }
    }

    // Transitions only append history. Rollback clones the small snapshot, not
    // the entire event history on every 100ms tick.
    fn atomic(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        let snapshot = self.snapshot.clone();
        let ids = self.ids.clone();
        let session_count = self.history.sessions.len();
        let event_count = self.history.events.len();
        if let Err(error) = change(self) {
            self.snapshot = snapshot;
            self.ids = ids;
            self.history.sessions.truncate(session_count);
            self.history.events.truncate(event_count);
            return Err(error);
        }
        Ok(())
    }

    fn apply_inner(&mut self, command: Command, at: Timestamp) -> Result<(), DomainError> {
        match command {
            Command::SetCurrentTask(task) => {
                let ProgressState::Ready {
                    next_kind,
                    current_task_draft,
                } = &mut self.snapshot.state
                else {
                    return Err(DomainError::InvalidOperation);
                };
                if !next_kind.is_work() {
                    return Err(DomainError::InvalidOperation);
                }
                *current_task_draft = task;
            }
            Command::Configure(settings) => {
                if !matches!(self.snapshot.state, ProgressState::Ready { .. }) {
                    return Err(DomainError::InvalidOperation);
                }
                settings.validate()?;
                self.snapshot.settings = settings;
                self.snapshot.round_progress = RoundProgress::default();
            }
            Command::Start(kind) => self.start_ready(kind, at)?,
            Command::Pause(id) | Command::Distraction(id) => {
                self.require_running(id, at)?;
                let kind = if matches!(command, Command::Pause(_)) {
                    InterruptionKind::Pause
                } else {
                    InterruptionKind::Distraction
                };
                if kind == InterruptionKind::Distraction && !self.active()?.0.kind.is_work() {
                    return Err(DomainError::InvalidOperation);
                }
                self.interrupt(kind, at, at, None)?;
            }
            Command::Resume(id) => self.resume(id, false, at)?,
            Command::Return(id) => self.resume(id, true, at)?,
            Command::End {
                session_id,
                outcome,
            } => {
                if outcome == SessionOutcome::Completed {
                    return Err(DomainError::InvalidOperation);
                }
                self.require_active(session_id)?;
                self.end_session(outcome, at)?;
            }
            Command::ResetReady => {
                if !matches!(self.snapshot.state, ProgressState::Ready { .. }) {
                    return Err(DomainError::InvalidOperation);
                }
            }
            Command::SkipReady => {
                let ProgressState::Ready { next_kind, .. } = self.snapshot.state else {
                    return Err(DomainError::InvalidOperation);
                };
                let next = self.next_kind(next_kind, SessionOutcome::Skipped)?;
                self.snapshot.state = ready(next, None);
            }
            Command::DecideQuickStart { session_id, choice } => {
                self.decide(session_id, choice, at)?;
            }
            Command::CloseApp => self.close_app(at)?,
            Command::RestoreApp => self.restore_app(at)?,
        }
        Ok(())
    }

    fn start_ready(&mut self, kind: SessionKind, at: Timestamp) -> Result<(), DomainError> {
        let ProgressState::Ready {
            next_kind,
            current_task_draft,
        } = &self.snapshot.state
        else {
            return Err(DomainError::InvalidOperation);
        };
        if *next_kind != kind && !(next_kind.is_work() && kind.is_work()) {
            return Err(DomainError::InvalidOperation);
        }
        self.start_session(kind, current_task_draft.clone(), None, at)?;
        Ok(())
    }

    fn start_session(
        &mut self,
        kind: SessionKind,
        task: Option<CurrentTask>,
        source: Option<SessionId>,
        at: Timestamp,
    ) -> Result<SessionId, DomainError> {
        let id = SessionId(allocate(&mut self.ids.next_session_id)?);
        self.snapshot.state = ProgressState::Active {
            session: Session {
                id,
                kind,
                current_task: task,
                planned_duration_ms: self.snapshot.settings.duration_millis(kind),
                started_at: at,
                elapsed_ms: 0,
                continued_from_quick_start: source,
                end: None,
            },
            timer: running(at, 0),
        };
        Ok(id)
    }

    fn active(&self) -> Result<(&Session, &TimerState), DomainError> {
        match &self.snapshot.state {
            ProgressState::Active { session, timer } => Ok((session, timer)),
            _ => Err(DomainError::InvalidOperation),
        }
    }

    fn require_active(&self, id: SessionId) -> Result<(), DomainError> {
        if !matches!(&self.snapshot.state, ProgressState::Active { session, .. } if session.id == id)
        {
            return Err(DomainError::WrongSession);
        }
        Ok(())
    }

    fn require_running(&self, id: SessionId, at: Timestamp) -> Result<(), DomainError> {
        self.require_active(id)?;
        let TimerState::Running { run } = self.active()?.1 else {
            return Err(DomainError::InvalidOperation);
        };
        if run.last_confirmed_at != at {
            return Err(DomainError::ObservationRequired);
        }
        Ok(())
    }

    fn emit(
        &mut self,
        session_id: SessionId,
        effective_at: Timestamp,
        recorded_at: Timestamp,
        payload: EventKind,
    ) -> Result<(), DomainError> {
        let sequence = allocate(&mut self.ids.next_event_sequence)?;
        self.history.events.push(HistoryEvent {
            sequence,
            session_id,
            effective_at,
            recorded_at,
            payload,
        });
        Ok(())
    }

    fn close_run(&mut self, recorded_at: Timestamp) -> Result<(), DomainError> {
        let (session, timer) = self.active()?;
        let TimerState::Running { run } = timer else {
            return Err(DomainError::InvalidOperation);
        };
        self.emit(
            session.id,
            run.last_confirmed_at,
            recorded_at,
            EventKind::RunIntervalRecorded {
                started_at: run.started_at,
                ended_at: run.last_confirmed_at,
                credited_ms: session
                    .elapsed_ms
                    .checked_sub(run.elapsed_ms_at_start)
                    .ok_or(DomainError::InvalidState("run boundary"))?,
                time_uncertainty: run.time_uncertainty,
            },
        )
    }

    fn interrupt(
        &mut self,
        kind: InterruptionKind,
        boundary: Timestamp,
        recorded_at: Timestamp,
        uncertainty: Option<TimeUncertainty>,
    ) -> Result<(), DomainError> {
        self.close_run(recorded_at)?;
        let id = InterruptionId(allocate(&mut self.ids.next_interruption_id)?);
        let session_id = self.active()?.0.id;
        self.emit(
            session_id,
            boundary,
            recorded_at,
            EventKind::InterruptionStarted {
                interruption_id: id,
                interruption_kind: kind,
            },
        )?;
        let ProgressState::Active { timer, .. } = &mut self.snapshot.state else {
            unreachable!()
        };
        *timer = TimerState::Interrupted {
            interruption: Interruption {
                id,
                kind,
                started_at: boundary,
                recorded_at,
                time_uncertainty: uncertainty,
                end: None,
            },
        };
        Ok(())
    }

    fn close_interruption(
        &mut self,
        outcome: InterruptionOutcome,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        let (session, timer) = self.active()?;
        let TimerState::Interrupted { interruption } = timer else {
            return Err(DomainError::InvalidOperation);
        };
        let duration = match interruption.time_uncertainty {
            Some(reason) => MeasuredDuration::Unknown { reason },
            None => match at.0.checked_sub(interruption.started_at.0) {
                Some(elapsed_ms) if at >= interruption.recorded_at => {
                    MeasuredDuration::Known { elapsed_ms }
                }
                _ => MeasuredDuration::Unknown {
                    reason: TimeUncertainty::ClockMovedBackward,
                },
            },
        };
        self.emit(
            session.id,
            at,
            at,
            EventKind::InterruptionEnded {
                interruption_id: interruption.id,
                end: InterruptionEnd {
                    ended_at: at,
                    outcome,
                    duration,
                },
            },
        )
    }

    fn resume(&mut self, id: SessionId, returning: bool, at: Timestamp) -> Result<(), DomainError> {
        self.require_active(id)?;
        let TimerState::Interrupted { interruption } = self.active()?.1 else {
            return Err(DomainError::InvalidOperation);
        };
        if (interruption.kind == InterruptionKind::Distraction) != returning {
            return Err(DomainError::InvalidOperation);
        }
        self.mark_clock_uncertainty(at);
        self.close_interruption(
            if returning {
                InterruptionOutcome::Returned
            } else {
                InterruptionOutcome::Resumed
            },
            at,
        )?;
        let ProgressState::Active { session, timer } = &mut self.snapshot.state else {
            unreachable!()
        };
        *timer = running(at, session.elapsed_ms);
        Ok(())
    }

    fn end_session(&mut self, outcome: SessionOutcome, at: Timestamp) -> Result<(), DomainError> {
        match self.active()?.1 {
            TimerState::Running { run } => {
                if run.last_confirmed_at != at {
                    return Err(DomainError::ObservationRequired);
                }
                self.close_run(at)?;
            }
            TimerState::Interrupted { .. } => {
                self.mark_clock_uncertainty(at);
                self.close_interruption(
                    InterruptionOutcome::SessionEnded {
                        session_outcome: outcome,
                    },
                    at,
                )?;
            }
        }
        let mut session = self.active()?.0.clone();
        session.end = Some(SessionEnd {
            ended_at: at,
            outcome,
        });
        self.snapshot.state =
            if session.kind == SessionKind::QuickStart && outcome == SessionOutcome::Completed {
                ProgressState::AwaitingQuickStartDecision {
                    quick_start_session_id: session.id,
                    current_task: session.current_task.clone(),
                }
            } else {
                let next = self.next_kind(session.kind, outcome)?;
                ready(
                    next,
                    if outcome == SessionOutcome::Reset {
                        session.current_task.clone()
                    } else {
                        None
                    },
                )
            };
        self.history.sessions.push(session);
        Ok(())
    }

    fn next_kind(
        &mut self,
        kind: SessionKind,
        outcome: SessionOutcome,
    ) -> Result<SessionKind, DomainError> {
        if outcome == SessionOutcome::Reset {
            return Ok(kind);
        }
        if kind == SessionKind::LongBreak {
            self.snapshot.round_progress = RoundProgress::default();
        }
        if outcome == SessionOutcome::Cancelled {
            return Ok(SessionKind::Focus);
        }
        if kind == SessionKind::Focus {
            let count = &mut self.snapshot.round_progress.completed_focuses_in_round;
            if outcome == SessionOutcome::Completed {
                *count = count.checked_add(1).ok_or(DomainError::Overflow)?;
                if *count > self.snapshot.settings.focuses_before_long_break() {
                    return Err(DomainError::InvalidState("round progress"));
                }
            }
            return Ok(
                if *count >= self.snapshot.settings.focuses_before_long_break() {
                    SessionKind::LongBreak
                } else {
                    SessionKind::ShortBreak
                },
            );
        }
        Ok(SessionKind::Focus)
    }

    fn decide(
        &mut self,
        id: SessionId,
        choice: QuickStartChoice,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        let ProgressState::AwaitingQuickStartDecision {
            quick_start_session_id,
            current_task,
        } = &self.snapshot.state
        else {
            return Err(DomainError::InvalidOperation);
        };
        if *quick_start_session_id != id {
            return Err(DomainError::WrongSession);
        }
        let task = current_task.clone();
        let decision = match choice {
            QuickStartChoice::Finish => {
                self.snapshot.state = ready(SessionKind::Focus, task);
                QuickStartDecision::Finish
            }
            QuickStartChoice::Continue => QuickStartDecision::Continue {
                focus_session_id: self.start_session(SessionKind::Focus, task, Some(id), at)?,
            },
        };
        self.emit(id, at, at, EventKind::QuickStartDecisionMade { decision })
    }

    fn close_app(&mut self, at: Timestamp) -> Result<(), DomainError> {
        if let ProgressState::Active {
            session,
            timer: TimerState::Running { .. },
        } = &self.snapshot.state
        {
            self.require_running(session.id, at)?;
            self.interrupt(InterruptionKind::AppExit, at, at, None)?;
        }
        self.mark_clock_uncertainty(at);
        if let Some(id) = self.lifecycle_session() {
            self.emit(id, at, at, EventKind::AppClosing)?;
        }
        Ok(())
    }

    fn restore_app(&mut self, at: Timestamp) -> Result<(), DomainError> {
        if let ProgressState::Active {
            timer: TimerState::Running { run },
            ..
        } = &self.snapshot.state
        {
            let boundary = run.last_confirmed_at;
            let uncertainty = (at < boundary).then_some(TimeUncertainty::ClockMovedBackward);
            self.record_gap(boundary, at, GapReason::Restart, uncertainty)?;
        }
        self.mark_clock_uncertainty(at);
        if let Some(id) = self.lifecycle_session() {
            self.emit(id, at, at, EventKind::AppRestored)?;
        }
        Ok(())
    }

    fn lifecycle_session(&self) -> Option<SessionId> {
        match &self.snapshot.state {
            ProgressState::Active { session, .. } => Some(session.id),
            ProgressState::AwaitingQuickStartDecision {
                quick_start_session_id,
                ..
            } => Some(*quick_start_session_id),
            ProgressState::Ready { .. } => None,
        }
    }

    fn mark_clock_uncertainty(&mut self, at: Timestamp) {
        if let ProgressState::Active {
            session,
            timer: TimerState::Interrupted { interruption },
        } = &mut self.snapshot.state
        {
            let last_recorded = self
                .history
                .events
                .iter()
                .rev()
                .find(|event| event.session_id == session.id)
                .map(|event| event.recorded_at);
            if at < interruption.recorded_at || last_recorded.is_some_and(|last| at < last) {
                interruption
                    .time_uncertainty
                    .get_or_insert(TimeUncertainty::ClockMovedBackward);
            }
        }
    }

    fn observe_inner(&mut self, observation: Observation) -> Result<(), DomainError> {
        if !matches!(self.snapshot.state, ProgressState::Active { .. }) {
            return Ok(());
        }
        // Even a consistent pair can predate the interruption or a lifecycle
        // event. Keep that evidence before ignoring an interrupted timer's tick.
        self.mark_clock_uncertainty(observation.at);
        let mut classification = observation.classify();
        if let TimerState::Running { run } = self.active()?.1 {
            if observation.at < run.last_confirmed_at {
                classification = Err((
                    GapReason::ClockAnomaly,
                    Some(TimeUncertainty::ClockMovedBackward),
                ));
            } else if run.last_confirmed_at != observation.previous_at && classification.is_ok() {
                // Preserve pair-level anomalies, but never credit a healthy pair
                // that is not linked to the last confirmed observation.
                classification = Err((
                    GapReason::ObservationDiscontinuity,
                    Some(TimeUncertainty::InsufficientClockEvidence),
                ));
            }
        }
        let elapsed = match classification {
            Ok(elapsed) => elapsed,
            Err((reason, uncertainty)) => {
                let boundary = match self.active()?.1 {
                    TimerState::Running { run } => run.last_confirmed_at,
                    TimerState::Interrupted { .. } => observation.previous_at,
                };
                return self.record_gap(boundary, observation.at, reason, uncertainty);
            }
        };
        let ProgressState::Active {
            session,
            timer: TimerState::Running { run },
        } = &mut self.snapshot.state
        else {
            return Ok(());
        };
        session.elapsed_ms += elapsed.min(session.remaining_ms());
        run.last_confirmed_at = observation.at;
        if session.remaining_ms() == 0 {
            self.end_session(SessionOutcome::Completed, observation.at)?;
        }
        Ok(())
    }

    fn record_gap(
        &mut self,
        boundary: Timestamp,
        at: Timestamp,
        reason: GapReason,
        uncertainty: Option<TimeUncertainty>,
    ) -> Result<(), DomainError> {
        let id = self.active()?.0.id;
        self.emit(
            id,
            boundary,
            at,
            EventKind::ObservationGapDetected {
                last_confirmed_at: boundary,
                detected_at: at,
                reason,
            },
        )?;
        if matches!(self.active()?.1, TimerState::Running { .. }) {
            self.interrupt(InterruptionKind::ObservationGap, boundary, at, uncertainty)?;
        } else if let ProgressState::Active {
            timer: TimerState::Interrupted { interruption },
            ..
        } = &mut self.snapshot.state
        {
            interruption.time_uncertainty = interruption.time_uncertainty.or(uncertainty);
        }
        Ok(())
    }
}

fn ready(kind: SessionKind, task: Option<CurrentTask>) -> ProgressState {
    ProgressState::Ready {
        next_kind: kind,
        current_task_draft: task,
    }
}

fn running(at: Timestamp, elapsed_ms: u64) -> TimerState {
    TimerState::Running {
        run: RunState {
            started_at: at,
            elapsed_ms_at_start: elapsed_ms,
            last_confirmed_at: at,
            time_uncertainty: None,
        },
    }
}

fn allocate(next: &mut u64) -> Result<u64, DomainError> {
    let id = *next;
    *next = next.checked_add(1).ok_or(DomainError::Overflow)?;
    Ok(id)
}
