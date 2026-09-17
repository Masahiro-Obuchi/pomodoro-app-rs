use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DomainError, DomainState, EventKind, GapReason, HistoryEvent, Interruption, InterruptionEnd,
    InterruptionId, InterruptionKind, InterruptionOutcome, MeasuredDuration, ProgressState,
    QuickStartDecision, Session, SessionId, SessionKind, SessionOutcome, TimerState, Timestamp,
};

fn ensure(condition: bool, reason: &'static str) -> Result<(), DomainError> {
    if condition {
        Ok(())
    } else {
        Err(DomainError::InvalidState(reason))
    }
}

impl DomainState {
    /// Validates ownership, references, allocation and interval accounting. This
    /// checks a supplied snapshot; it never reconstructs or repairs it by replay.
    ///
    /// # Errors
    /// Rejects inconsistent domain data, including arithmetic overflow.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.snapshot.settings.validate()?;
        ensure(
            self.snapshot.round_progress.completed_focuses_in_round
                <= self.snapshot.settings.focuses_before_long_break(),
            "round progress",
        )?;
        ensure(
            self.ids.next_session_id > 0
                && self.ids.next_interruption_id > 0
                && self.ids.next_event_sequence > 0,
            "zero allocator",
        )?;
        let mut sessions = BTreeMap::new();
        for session in &self.history.sessions {
            ensure(session.end.is_some(), "unfinished history session")?;
            ensure(
                sessions.insert(session.id, session).is_none(),
                "duplicate session",
            )?;
        }
        if let ProgressState::Active { session, .. } = &self.snapshot.state {
            ensure(
                session.end.is_none() && session.elapsed_ms < session.planned_duration_ms,
                "ended active session",
            )?;
            // Settings cannot change while active. Historical sessions keep the
            // duration captured at their start, even after settings change.
            ensure(
                session.planned_duration_ms == self.snapshot.settings.duration_millis(session.kind),
                "active session duration",
            )?;
            ensure(
                sessions.insert(session.id, session).is_none(),
                "duplicate active session",
            )?;
        }
        for session in sessions.values() {
            validate_session(session)?;
            ensure(session.id.0 < self.ids.next_session_id, "session allocator")?;
        }
        validate_snapshot(&self.snapshot.state, &sessions)?;
        let mut facts: BTreeMap<_, _> = sessions
            .iter()
            .map(|(id, session)| (*id, SessionFacts::new(session)))
            .collect();
        let mut interruption_ids = BTreeSet::new();
        let mut decisions = BTreeMap::new();
        let mut sequence = 1_u64;
        for event in &self.history.events {
            ensure(event.sequence == sequence, "event sequence")?;
            sequence = sequence.checked_add(1).ok_or(DomainError::Overflow)?;
            let facts = facts
                .get_mut(&event.session_id)
                .ok_or(DomainError::InvalidState("event session reference"))?;
            if let Some(open) = &mut facts.open {
                open.record_clock_evidence(event);
            }
            validate_event(facts, event, &mut interruption_ids, &mut decisions)?;
        }
        ensure(self.ids.next_event_sequence == sequence, "event allocator")?;
        ensure(
            interruption_ids
                .last()
                .is_none_or(|id: &InterruptionId| id.0 < self.ids.next_interruption_id),
            "interruption allocator",
        )?;
        for (id, facts) in &facts {
            validate_accounting(*id, facts, &self.snapshot.state)?;
        }
        validate_decisions(&sessions, &decisions, &self.snapshot.state)
    }
}

fn validate_session(session: &Session) -> Result<(), DomainError> {
    ensure(session.id.0 > 0, "zero session ID")?;
    ensure(
        (1_000..=86_400_000).contains(&session.planned_duration_ms)
            && session.planned_duration_ms % 1_000 == 0,
        "planned duration",
    )?;
    ensure(
        session.elapsed_ms <= session.planned_duration_ms,
        "elapsed duration",
    )?;
    if let Some(end) = session.end {
        ensure(
            (end.outcome == SessionOutcome::Completed)
                == (session.elapsed_ms == session.planned_duration_ms),
            "completion duration",
        )?;
    }
    ensure(
        session.kind != SessionKind::QuickStart || session.planned_duration_ms == 120_000,
        "quick start duration",
    )?;
    ensure(
        session.kind.is_work() || session.current_task.is_none(),
        "break task",
    )?;
    ensure(
        session.kind == SessionKind::Focus || session.continued_from_quick_start.is_none(),
        "continuation kind",
    )
}

fn validate_snapshot(
    state: &ProgressState,
    sessions: &BTreeMap<SessionId, &Session>,
) -> Result<(), DomainError> {
    match state {
        ProgressState::Ready {
            next_kind,
            current_task_draft,
        } => ensure(
            next_kind.is_work() || current_task_draft.is_none(),
            "break draft",
        ),
        ProgressState::Active { session, timer } => match timer {
            TimerState::Running { run } => {
                ensure(
                    run.elapsed_ms_at_start <= session.elapsed_ms,
                    "run elapsed boundary",
                )?;
                ensure(
                    run.last_confirmed_at >= run.started_at || run.time_uncertainty.is_some(),
                    "run timestamps",
                )
            }
            TimerState::Interrupted { interruption } => {
                ensure(
                    interruption.id.0 > 0 && interruption.end.is_none(),
                    "open interruption",
                )?;
                ensure(
                    interruption.kind != InterruptionKind::Distraction || session.kind.is_work(),
                    "break distraction",
                )
            }
        },
        ProgressState::AwaitingQuickStartDecision {
            quick_start_session_id,
            current_task,
        } => {
            let source = sessions
                .get(quick_start_session_id)
                .ok_or(DomainError::InvalidState("awaiting reference"))?;
            ensure(
                is_completed_quick_start(source) && *current_task == source.current_task,
                "awaiting source",
            )
        }
    }
}

struct OpenInterruption {
    id: InterruptionId,
    kind: InterruptionKind,
    started_at: Timestamp,
    recorded_at: Timestamp,
    last_recorded_at: Timestamp,
    clock_uncertain: bool,
}

impl OpenInterruption {
    fn record_clock_evidence(&mut self, event: &HistoryEvent) {
        // Only evidence within this interruption applies to its duration. A
        // later healthy timestamp cannot undo an earlier clock anomaly.
        self.clock_uncertain |= event.recorded_at < self.last_recorded_at;
        if let EventKind::ObservationGapDetected {
            last_confirmed_at,
            detected_at,
            reason,
        } = &event.payload
        {
            self.clock_uncertain |=
                detected_at < last_confirmed_at || *reason == GapReason::ClockAnomaly;
        }
        self.last_recorded_at = event.recorded_at;
    }
}

struct SessionFacts<'a> {
    session: &'a Session,
    credited_ms: u64,
    run_closed: bool,
    run_start: Timestamp,
    run_end: Option<Timestamp>,
    open: Option<OpenInterruption>,
    pending_gap_reason: Option<GapReason>,
    ended_by_interruption: bool,
}

impl<'a> SessionFacts<'a> {
    fn new(session: &'a Session) -> Self {
        Self {
            session,
            credited_ms: 0,
            run_closed: false,
            run_start: session.started_at,
            run_end: None,
            open: None,
            pending_gap_reason: None,
            ended_by_interruption: false,
        }
    }

    // This describes the position in the event sequence, not the final snapshot.
    // The decision map separately records whether this wait has already ended.
    fn awaiting_quick_start_decision(&self) -> bool {
        is_completed_quick_start(self.session)
            && self.run_closed
            && self.open.is_none()
            && self.credited_ms == self.session.planned_duration_ms
    }
}

fn validate_event(
    facts: &mut SessionFacts<'_>,
    event: &HistoryEvent,
    ids: &mut BTreeSet<InterruptionId>,
    decisions: &mut BTreeMap<SessionId, QuickStartDecision>,
) -> Result<(), DomainError> {
    match &event.payload {
        EventKind::RunIntervalRecorded {
            started_at,
            ended_at,
            credited_ms,
            time_uncertainty,
        } => {
            ensure(
                !facts.run_closed && facts.open.is_none() && !facts.ended_by_interruption,
                "duplicate or interrupted interval",
            )?;
            ensure(
                *started_at == facts.run_start && *ended_at == event.effective_at,
                "interval boundary",
            )?;
            ensure(
                ended_at >= started_at || time_uncertainty.is_some(),
                "interval timestamps",
            )?;
            facts.credited_ms = facts
                .credited_ms
                .checked_add(*credited_ms)
                .ok_or(DomainError::Overflow)?;
            facts.run_closed = true;
            facts.run_end = Some(*ended_at);
        }
        EventKind::InterruptionStarted {
            interruption_id,
            interruption_kind,
        } => validate_interruption_start(facts, event, *interruption_id, *interruption_kind, ids)?,
        EventKind::InterruptionEnded {
            interruption_id,
            end,
        } => {
            let open = facts
                .open
                .take()
                .ok_or(DomainError::InvalidState("unpaired interruption end"))?;
            ensure(
                *interruption_id == open.id && end.ended_at == event.effective_at,
                "interruption end reference",
            )?;
            validate_interruption_end(&open, *end, facts.session)?;
            facts.ended_by_interruption =
                matches!(end.outcome, InterruptionOutcome::SessionEnded { .. });
            facts.run_closed = facts.ended_by_interruption;
            facts.run_start = end.ended_at;
        }
        EventKind::QuickStartDecisionMade { decision } => {
            ensure(
                facts.awaiting_quick_start_decision(),
                "decision before completion",
            )?;
            ensure(
                decisions.insert(event.session_id, *decision).is_none(),
                "duplicate quick start decision",
            )?;
        }
        EventKind::ObservationGapDetected {
            last_confirmed_at,
            detected_at,
            reason,
        } => {
            validate_observation_gap(facts, event, *last_confirmed_at, *detected_at)?;
            if facts.open.is_none() {
                ensure(
                    facts.pending_gap_reason.is_none(),
                    "duplicate observation gap",
                )?;
                facts.pending_gap_reason = Some(*reason);
            }
        }
        EventKind::AppClosing | EventKind::AppRestored => {
            validate_lifecycle_event(facts, decisions)?;
        }
    }
    Ok(())
}

fn validate_interruption_start(
    facts: &mut SessionFacts<'_>,
    event: &HistoryEvent,
    interruption_id: InterruptionId,
    interruption_kind: InterruptionKind,
    ids: &mut BTreeSet<InterruptionId>,
) -> Result<(), DomainError> {
    let gap_reason = if interruption_kind == InterruptionKind::ObservationGap {
        Some(
            facts
                .pending_gap_reason
                .take()
                .ok_or(DomainError::InvalidState("missing observation gap"))?,
        )
    } else {
        ensure(
            facts.pending_gap_reason.is_none(),
            "unhandled observation gap",
        )?;
        None
    };
    ensure(
        interruption_kind == InterruptionKind::ObservationGap
            || event.effective_at == event.recorded_at,
        "manual interruption timestamps",
    )?;
    ensure(
        facts.run_closed && facts.open.is_none() && !facts.ended_by_interruption,
        "overlapping interruption",
    )?;
    ensure(
        facts.run_end == Some(event.effective_at),
        "interruption boundary",
    )?;
    ensure(
        interruption_id.0 > 0 && ids.insert(interruption_id),
        "duplicate interruption ID",
    )?;
    ensure(
        interruption_kind != InterruptionKind::Distraction || facts.session.kind.is_work(),
        "break distraction event",
    )?;
    facts.open = Some(OpenInterruption {
        id: interruption_id,
        kind: interruption_kind,
        started_at: event.effective_at,
        recorded_at: event.recorded_at,
        last_recorded_at: event.recorded_at,
        clock_uncertain: event.recorded_at < event.effective_at
            || gap_reason == Some(GapReason::ClockAnomaly),
    });
    Ok(())
}

fn validate_observation_gap(
    facts: &SessionFacts<'_>,
    event: &HistoryEvent,
    last_confirmed_at: Timestamp,
    detected_at: Timestamp,
) -> Result<(), DomainError> {
    ensure(
        !facts.run_closed || facts.open.is_some(),
        "gap outside active session",
    )?;
    ensure(
        last_confirmed_at == event.effective_at && detected_at == event.recorded_at,
        "gap timestamps",
    )
}

fn validate_lifecycle_event(
    facts: &SessionFacts<'_>,
    decisions: &BTreeMap<SessionId, QuickStartDecision>,
) -> Result<(), DomainError> {
    // Lifecycle commands interrupt a running session before emitting these
    // facts. Completed Quick Starts remain eligible until a decision.
    ensure(
        facts.open.is_some()
            || (facts.awaiting_quick_start_decision()
                && !decisions.contains_key(&facts.session.id)),
        "lifecycle outside interruption or quick start decision wait",
    )
}

fn validate_interruption_end(
    open: &OpenInterruption,
    end: InterruptionEnd,
    session: &Session,
) -> Result<(), DomainError> {
    match end.outcome {
        InterruptionOutcome::Resumed => ensure(
            open.kind != InterruptionKind::Distraction,
            "distraction resume",
        )?,
        InterruptionOutcome::Returned => ensure(
            open.kind == InterruptionKind::Distraction,
            "non-distraction return",
        )?,
        InterruptionOutcome::SessionEnded { session_outcome } => {
            ensure(
                session_outcome != SessionOutcome::Completed,
                "interrupted completion",
            )?;
            ensure(
                session.end.is_some_and(|ended| {
                    ended.outcome == session_outcome && ended.ended_at == end.ended_at
                }),
                "interruption session outcome",
            )?;
        }
    }
    if let MeasuredDuration::Known { elapsed_ms } = end.duration {
        ensure(!open.clock_uncertain, "interruption clock evidence")?;
        ensure(
            end.ended_at.0.checked_sub(open.started_at.0) == Some(elapsed_ms)
                && end.ended_at >= open.recorded_at,
            "interruption duration",
        )?;
    }
    Ok(())
}

fn same_open(open: &OpenInterruption, interruption: &Interruption) -> bool {
    open.id == interruption.id
        && open.kind == interruption.kind
        && open.started_at == interruption.started_at
        && open.recorded_at == interruption.recorded_at
        && (!open.clock_uncertain || interruption.time_uncertainty.is_some())
}

fn validate_accounting(
    id: SessionId,
    facts: &SessionFacts<'_>,
    state: &ProgressState,
) -> Result<(), DomainError> {
    if let ProgressState::Active { session, timer } = state {
        if session.id == id {
            return match timer {
                TimerState::Running { run } => {
                    ensure(
                        facts.open.is_none()
                            && facts.pending_gap_reason.is_none()
                            && !facts.run_closed,
                        "running with closed interval",
                    )?;
                    ensure(
                        run.elapsed_ms_at_start == facts.credited_ms
                            && run.started_at == facts.run_start,
                        "running accounting",
                    )
                }
                TimerState::Interrupted { interruption } => {
                    ensure(
                        facts
                            .open
                            .as_ref()
                            .is_some_and(|open| same_open(open, interruption)),
                        "snapshot interruption mismatch",
                    )?;
                    ensure(
                        session.elapsed_ms == facts.credited_ms,
                        "interrupted accounting",
                    )
                }
            };
        }
    }
    ensure(
        facts.open.is_none()
            && facts.pending_gap_reason.is_none()
            && facts.run_closed
            && facts.session.elapsed_ms == facts.credited_ms,
        "ended accounting",
    )?;
    if !facts.ended_by_interruption {
        ensure(
            facts
                .session
                .end
                .is_some_and(|end| Some(end.ended_at) == facts.run_end),
            "session end boundary",
        )?;
    }
    Ok(())
}

fn is_completed_quick_start(session: &Session) -> bool {
    session.kind == SessionKind::QuickStart
        && session
            .end
            .is_some_and(|end| end.outcome == SessionOutcome::Completed)
}

fn validate_decisions(
    sessions: &BTreeMap<SessionId, &Session>,
    decisions: &BTreeMap<SessionId, QuickStartDecision>,
    state: &ProgressState,
) -> Result<(), DomainError> {
    for session in sessions.values() {
        if is_completed_quick_start(session) {
            let awaiting = matches!(state, ProgressState::AwaitingQuickStartDecision { quick_start_session_id, .. } if *quick_start_session_id == session.id);
            ensure(
                awaiting != decisions.contains_key(&session.id),
                "quick start decision coverage",
            )?;
        }
        if let Some(source_id) = session.continued_from_quick_start {
            let source = sessions
                .get(&source_id)
                .ok_or(DomainError::InvalidState("continuation source"))?;
            ensure(
                is_completed_quick_start(source) && session.current_task == source.current_task,
                "continuation task or source",
            )?;
            ensure(
                decisions.get(&source_id)
                    == Some(&QuickStartDecision::Continue {
                        focus_session_id: session.id,
                    }),
                "continuation reverse reference",
            )?;
        }
    }
    for (source_id, decision) in decisions {
        if let QuickStartDecision::Continue { focus_session_id } = decision {
            let focus = sessions
                .get(focus_session_id)
                .ok_or(DomainError::InvalidState("continuation target"))?;
            ensure(
                focus.kind == SessionKind::Focus
                    && focus.continued_from_quick_start == Some(*source_id),
                "continuation forward reference",
            )?;
        }
    }
    Ok(())
}
