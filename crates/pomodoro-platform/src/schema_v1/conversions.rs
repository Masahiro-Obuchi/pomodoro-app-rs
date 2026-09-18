//! Explicit value conversion only. Cross-object validation belongs to core and
//! is invoked by the envelope codec after all objects have been converted.

use pomodoro_core as core;

use super::{
    codec::CodecError,
    dto::{
        EventKindV1, EventV1, HistoryV1, IdAllocatorsV1, InterruptionEndV1, InterruptionOutcomeV1,
        InterruptionV1, MeasuredDurationV1, ProgressV1, QuickStartDecisionV1, RoundProgressV1,
        RunV1, SessionEndV1, SessionV1, SnapshotV1, TimerV1,
    },
};

impl From<&core::IdAllocators> for IdAllocatorsV1 {
    fn from(value: &core::IdAllocators) -> Self {
        Self {
            next_session_id: value.next_session_id,
            next_interruption_id: value.next_interruption_id,
            next_event_sequence: value.next_event_sequence,
        }
    }
}

impl From<IdAllocatorsV1> for core::IdAllocators {
    fn from(value: IdAllocatorsV1) -> Self {
        Self {
            next_session_id: value.next_session_id,
            next_interruption_id: value.next_interruption_id,
            next_event_sequence: value.next_event_sequence,
        }
    }
}

impl TryFrom<&core::PomodoroState> for SnapshotV1 {
    type Error = CodecError;

    fn try_from(value: &core::PomodoroState) -> Result<Self, Self::Error> {
        Ok(Self {
            settings: value.settings.try_into().map_err(core::DomainError::from)?,
            round_progress: RoundProgressV1 {
                completed_focuses_in_round: value.round_progress.completed_focuses_in_round,
            },
            state: match &value.state {
                core::ProgressState::Ready {
                    next_kind,
                    current_task_draft,
                } => ProgressV1::Ready {
                    next_kind: (*next_kind).into(),
                    current_task_draft: current_task_draft.clone().map(Into::into),
                },
                core::ProgressState::Active { session, timer } => ProgressV1::Active {
                    session: session.try_into()?,
                    timer: timer.try_into()?,
                },
                core::ProgressState::AwaitingQuickStartDecision {
                    quick_start_session_id,
                    current_task,
                } => ProgressV1::AwaitingQuickStartDecision {
                    quick_start_session_id: (*quick_start_session_id).try_into()?,
                    current_task: current_task.clone().map(Into::into),
                },
            },
        })
    }
}

impl From<SnapshotV1> for core::PomodoroState {
    fn from(value: SnapshotV1) -> Self {
        Self {
            settings: value.settings.into(),
            round_progress: core::RoundProgress {
                completed_focuses_in_round: value.round_progress.completed_focuses_in_round,
            },
            state: match value.state {
                ProgressV1::Ready {
                    next_kind,
                    current_task_draft,
                } => core::ProgressState::Ready {
                    next_kind: next_kind.into(),
                    current_task_draft: current_task_draft.map(Into::into),
                },
                ProgressV1::Active { session, timer } => core::ProgressState::Active {
                    session: session.into(),
                    timer: timer.into(),
                },
                ProgressV1::AwaitingQuickStartDecision {
                    quick_start_session_id,
                    current_task,
                } => core::ProgressState::AwaitingQuickStartDecision {
                    quick_start_session_id: quick_start_session_id.into(),
                    current_task: current_task.map(Into::into),
                },
            },
        }
    }
}

impl TryFrom<&core::Session> for SessionV1 {
    type Error = CodecError;

    fn try_from(value: &core::Session) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id.try_into()?,
            kind: value.kind.into(),
            current_task: value.current_task.clone().map(Into::into),
            planned_duration_ms: value.planned_duration_ms,
            started_at: value.started_at.try_into()?,
            elapsed_ms: value.elapsed_ms,
            continued_from_quick_start: value
                .continued_from_quick_start
                .map(TryInto::try_into)
                .transpose()?,
            end: value.end.map(TryInto::try_into).transpose()?,
        })
    }
}

impl From<SessionV1> for core::Session {
    fn from(value: SessionV1) -> Self {
        Self {
            id: value.id.into(),
            kind: value.kind.into(),
            current_task: value.current_task.map(Into::into),
            planned_duration_ms: value.planned_duration_ms,
            started_at: value.started_at.into(),
            elapsed_ms: value.elapsed_ms,
            continued_from_quick_start: value.continued_from_quick_start.map(Into::into),
            end: value.end.map(Into::into),
        }
    }
}

impl TryFrom<core::SessionEnd> for SessionEndV1 {
    type Error = CodecError;

    fn try_from(value: core::SessionEnd) -> Result<Self, Self::Error> {
        Ok(Self {
            ended_at: value.ended_at.try_into()?,
            outcome: value.outcome.into(),
        })
    }
}

impl From<SessionEndV1> for core::SessionEnd {
    fn from(value: SessionEndV1) -> Self {
        Self {
            ended_at: value.ended_at.into(),
            outcome: value.outcome.into(),
        }
    }
}

impl TryFrom<&core::TimerState> for TimerV1 {
    type Error = CodecError;

    fn try_from(value: &core::TimerState) -> Result<Self, Self::Error> {
        Ok(match value {
            core::TimerState::Running { run } => Self::Running {
                run: RunV1 {
                    started_at: run.started_at.try_into()?,
                    elapsed_ms_at_start: run.elapsed_ms_at_start,
                    last_confirmed_at: run.last_confirmed_at.try_into()?,
                    time_uncertainty: run.time_uncertainty.map(Into::into),
                },
            },
            core::TimerState::Interrupted { interruption } => Self::Interrupted {
                interruption: InterruptionV1 {
                    id: interruption.id.try_into()?,
                    kind: interruption.kind.into(),
                    started_at: interruption.started_at.try_into()?,
                    recorded_at: interruption.recorded_at.try_into()?,
                    time_uncertainty: interruption.time_uncertainty.map(Into::into),
                    end: interruption.end.map(TryInto::try_into).transpose()?,
                },
            },
        })
    }
}

impl From<TimerV1> for core::TimerState {
    fn from(value: TimerV1) -> Self {
        match value {
            TimerV1::Running { run } => Self::Running {
                run: core::RunState {
                    started_at: run.started_at.into(),
                    elapsed_ms_at_start: run.elapsed_ms_at_start,
                    last_confirmed_at: run.last_confirmed_at.into(),
                    time_uncertainty: run.time_uncertainty.map(Into::into),
                },
            },
            TimerV1::Interrupted { interruption } => Self::Interrupted {
                interruption: core::Interruption {
                    id: interruption.id.into(),
                    kind: interruption.kind.into(),
                    started_at: interruption.started_at.into(),
                    recorded_at: interruption.recorded_at.into(),
                    time_uncertainty: interruption.time_uncertainty.map(Into::into),
                    end: interruption.end.map(Into::into),
                },
            },
        }
    }
}

impl TryFrom<core::InterruptionEnd> for InterruptionEndV1 {
    type Error = CodecError;

    fn try_from(value: core::InterruptionEnd) -> Result<Self, Self::Error> {
        Ok(Self {
            ended_at: value.ended_at.try_into()?,
            outcome: match value.outcome {
                core::InterruptionOutcome::Resumed => InterruptionOutcomeV1::Resumed {},
                core::InterruptionOutcome::Returned => InterruptionOutcomeV1::Returned {},
                core::InterruptionOutcome::SessionEnded { session_outcome } => {
                    InterruptionOutcomeV1::SessionEnded {
                        session_outcome: session_outcome.into(),
                    }
                }
            },
            duration: match value.duration {
                core::MeasuredDuration::Known { elapsed_ms } => {
                    MeasuredDurationV1::Known { elapsed_ms }
                }
                core::MeasuredDuration::Unknown { reason } => MeasuredDurationV1::Unknown {
                    reason: reason.into(),
                },
            },
        })
    }
}

impl From<InterruptionEndV1> for core::InterruptionEnd {
    fn from(value: InterruptionEndV1) -> Self {
        Self {
            ended_at: value.ended_at.into(),
            outcome: match value.outcome {
                InterruptionOutcomeV1::Resumed {} => core::InterruptionOutcome::Resumed,
                InterruptionOutcomeV1::Returned {} => core::InterruptionOutcome::Returned,
                InterruptionOutcomeV1::SessionEnded { session_outcome } => {
                    core::InterruptionOutcome::SessionEnded {
                        session_outcome: session_outcome.into(),
                    }
                }
            },
            duration: match value.duration {
                MeasuredDurationV1::Known { elapsed_ms } => {
                    core::MeasuredDuration::Known { elapsed_ms }
                }
                MeasuredDurationV1::Unknown { reason } => core::MeasuredDuration::Unknown {
                    reason: reason.into(),
                },
            },
        }
    }
}

impl TryFrom<&core::History> for HistoryV1 {
    type Error = CodecError;

    fn try_from(value: &core::History) -> Result<Self, Self::Error> {
        Ok(Self {
            sessions: value
                .sessions
                .iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            events: value
                .events
                .iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<HistoryV1> for core::History {
    fn from(value: HistoryV1) -> Self {
        Self {
            sessions: value.sessions.into_iter().map(Into::into).collect(),
            events: value.events.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<&core::HistoryEvent> for EventV1 {
    type Error = CodecError;

    fn try_from(value: &core::HistoryEvent) -> Result<Self, Self::Error> {
        Ok(Self {
            sequence: value.sequence,
            session_id: value.session_id.try_into()?,
            effective_at: value.effective_at.try_into()?,
            recorded_at: value.recorded_at.try_into()?,
            payload: match value.payload {
                core::EventKind::RunIntervalRecorded {
                    started_at,
                    ended_at,
                    credited_ms,
                    time_uncertainty,
                } => EventKindV1::RunIntervalRecorded {
                    started_at: started_at.try_into()?,
                    ended_at: ended_at.try_into()?,
                    credited_ms,
                    time_uncertainty: time_uncertainty.map(Into::into),
                },
                core::EventKind::InterruptionStarted {
                    interruption_id,
                    interruption_kind,
                } => EventKindV1::InterruptionStarted {
                    interruption_id: interruption_id.try_into()?,
                    interruption_kind: interruption_kind.into(),
                },
                core::EventKind::InterruptionEnded {
                    interruption_id,
                    end,
                } => EventKindV1::InterruptionEnded {
                    interruption_id: interruption_id.try_into()?,
                    end: end.try_into()?,
                },
                core::EventKind::QuickStartDecisionMade { decision } => {
                    EventKindV1::QuickStartDecisionMade {
                        decision: match decision {
                            core::QuickStartDecision::Finish => QuickStartDecisionV1::Finish {},
                            core::QuickStartDecision::Continue { focus_session_id } => {
                                QuickStartDecisionV1::Continue {
                                    focus_session_id: focus_session_id.try_into()?,
                                }
                            }
                        },
                    }
                }
                core::EventKind::AppClosing => EventKindV1::AppClosing {},
                core::EventKind::AppRestored => EventKindV1::AppRestored {},
                core::EventKind::ObservationGapDetected {
                    last_confirmed_at,
                    detected_at,
                    reason,
                } => EventKindV1::ObservationGapDetected {
                    last_confirmed_at: last_confirmed_at.try_into()?,
                    detected_at: detected_at.try_into()?,
                    reason: reason.into(),
                },
            },
        })
    }
}

impl From<EventV1> for core::HistoryEvent {
    fn from(value: EventV1) -> Self {
        Self {
            sequence: value.sequence,
            session_id: value.session_id.into(),
            effective_at: value.effective_at.into(),
            recorded_at: value.recorded_at.into(),
            payload: match value.payload {
                EventKindV1::RunIntervalRecorded {
                    started_at,
                    ended_at,
                    credited_ms,
                    time_uncertainty,
                } => core::EventKind::RunIntervalRecorded {
                    started_at: started_at.into(),
                    ended_at: ended_at.into(),
                    credited_ms,
                    time_uncertainty: time_uncertainty.map(Into::into),
                },
                EventKindV1::InterruptionStarted {
                    interruption_id,
                    interruption_kind,
                } => core::EventKind::InterruptionStarted {
                    interruption_id: interruption_id.into(),
                    interruption_kind: interruption_kind.into(),
                },
                EventKindV1::InterruptionEnded {
                    interruption_id,
                    end,
                } => core::EventKind::InterruptionEnded {
                    interruption_id: interruption_id.into(),
                    end: end.into(),
                },
                EventKindV1::QuickStartDecisionMade { decision } => {
                    core::EventKind::QuickStartDecisionMade {
                        decision: match decision {
                            QuickStartDecisionV1::Finish {} => core::QuickStartDecision::Finish,
                            QuickStartDecisionV1::Continue { focus_session_id } => {
                                core::QuickStartDecision::Continue {
                                    focus_session_id: focus_session_id.into(),
                                }
                            }
                        },
                    }
                }
                EventKindV1::AppClosing {} => core::EventKind::AppClosing,
                EventKindV1::AppRestored {} => core::EventKind::AppRestored,
                EventKindV1::ObservationGapDetected {
                    last_confirmed_at,
                    detected_at,
                    reason,
                } => core::EventKind::ObservationGapDetected {
                    last_confirmed_at: last_confirmed_at.into(),
                    detected_at: detected_at.into(),
                    reason: reason.into(),
                },
            },
        }
    }
}
