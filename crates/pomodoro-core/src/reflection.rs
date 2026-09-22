use crate::{
    DomainError, EventKind, History, InterruptionEnd, InterruptionKind, InterruptionOutcome,
    ReflectionSummary, Session, SessionKind, SessionOutcome,
};

impl History {
    /// Summarizes recorded sessions and events, excluding the active snapshot.
    /// Callers may cache this derived value while the history is unchanged.
    ///
    /// # Errors
    /// Returns an error if the total work duration exceeds its integer range.
    pub fn reflection(&self) -> Result<ReflectionSummary, DomainError> {
        let mut summary = ReflectionSummary::default();
        for session in &self.sessions {
            summary = summary.including_session(session)?;
        }
        for event in &self.events {
            match event.payload {
                EventKind::InterruptionStarted {
                    interruption_kind: InterruptionKind::Distraction,
                    ..
                } => summary.distractions += 1,
                EventKind::InterruptionEnded {
                    end:
                        InterruptionEnd {
                            outcome: InterruptionOutcome::Returned,
                            ..
                        },
                    ..
                } => summary.returns += 1,
                _ => {}
            }
        }
        Ok(summary)
    }
}

impl ReflectionSummary {
    /// Adds one session's work time and natural Focus completion, preserving
    /// interruption counts derived from events. Add the active snapshot to a
    /// cached history summary to display live work without rescanning history.
    ///
    /// # Errors
    /// Returns an error if total work duration or completion count overflows.
    pub fn including_session(mut self, session: &Session) -> Result<Self, DomainError> {
        if session.kind.is_work() {
            self.work_ms = self
                .work_ms
                .checked_add(session.elapsed_ms)
                .ok_or(DomainError::Overflow)?;
        }
        if session.kind == SessionKind::Focus
            && session
                .end
                .is_some_and(|end| end.outcome == SessionOutcome::Completed)
        {
            self.completed_focus_sessions = self
                .completed_focus_sessions
                .checked_add(1)
                .ok_or(DomainError::Overflow)?;
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionEnd, SessionId, Timestamp};

    #[test]
    fn including_a_session_checks_both_overflows_without_changing_the_summary() {
        let session = Session {
            id: SessionId(1),
            kind: SessionKind::Focus,
            current_task: None,
            planned_duration_ms: 1,
            started_at: Timestamp(0),
            elapsed_ms: 1,
            continued_from_quick_start: None,
            end: Some(SessionEnd {
                ended_at: Timestamp(1),
                outcome: SessionOutcome::Completed,
            }),
        };
        for summary in [
            ReflectionSummary {
                work_ms: u64::MAX,
                ..ReflectionSummary::default()
            },
            ReflectionSummary {
                completed_focus_sessions: u64::MAX,
                ..ReflectionSummary::default()
            },
        ] {
            let before = summary;
            assert_eq!(
                summary.including_session(&session),
                Err(DomainError::Overflow)
            );
            assert_eq!(summary, before);
        }
    }
}
