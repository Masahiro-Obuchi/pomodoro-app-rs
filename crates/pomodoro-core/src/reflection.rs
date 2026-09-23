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
    use crate::{
        Command, DomainState, Observation, ProgressState, SessionEnd, SessionId, TimerConfig,
        Timestamp,
    };

    #[test]
    fn completed_focuses_accumulate_while_breaks_and_skips_do_not() {
        let mut domain = DomainState::new(TimerConfig::new(1, 1, 1, 4).unwrap()).unwrap();
        for (kind, started_at) in [
            (SessionKind::Focus, 0),
            (SessionKind::ShortBreak, 1_000),
            (SessionKind::Focus, 2_000),
        ] {
            domain
                .apply(Command::Start(kind), Timestamp(started_at))
                .unwrap();
            domain
                .observe(Observation {
                    previous_at: Timestamp(started_at),
                    at: Timestamp(started_at + 1_000),
                    monotonic_elapsed_ms: Some(1_000),
                })
                .unwrap();
        }
        domain.apply(Command::SkipReady, Timestamp(3_000)).unwrap();
        domain
            .apply(Command::Start(SessionKind::Focus), Timestamp(3_000))
            .unwrap();
        let ProgressState::Active { session, .. } = &domain.snapshot().state else {
            panic!("expected active focus");
        };
        domain
            .apply(
                Command::End {
                    session_id: session.id,
                    outcome: SessionOutcome::Skipped,
                },
                Timestamp(3_000),
            )
            .unwrap();

        assert_eq!(domain.history().sessions.len(), 4);
        assert_eq!(
            domain.history().reflection().unwrap(),
            ReflectionSummary {
                work_ms: 2_000,
                completed_focus_sessions: 2,
                distractions: 0,
                returns: 0,
            }
        );
    }

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
