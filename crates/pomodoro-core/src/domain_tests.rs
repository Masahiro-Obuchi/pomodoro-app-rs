use crate::*;

fn state() -> DomainState {
    DomainState::new(TimerConfig::new(10, 5, 20, 2).unwrap()).unwrap()
}

fn apply(state: &mut DomainState, command: Command, at: u64) {
    state.apply(command, Timestamp(at)).unwrap();
    state.validate().unwrap();
}

fn observe(state: &mut DomainState, from: u64, to: u64) {
    state
        .observe(Observation {
            previous_at: Timestamp(from),
            at: Timestamp(to),
            monotonic_elapsed_ms: Some(to - from),
        })
        .unwrap();
    state.validate().unwrap();
}

fn advance(state: &mut DomainState, from: u64, duration: u64) {
    let mut at = from;
    while at < from + duration {
        let next = (at + 1_000).min(from + duration);
        observe(state, at, next);
        at = next;
    }
}

fn active(state: &DomainState) -> (&Session, &TimerState) {
    let ProgressState::Active { session, timer } = &state.snapshot().state else {
        panic!("expected active")
    };
    (session, timer)
}

fn interruption(state: &DomainState) -> &Interruption {
    let TimerState::Interrupted { interruption } = active(state).1 else {
        panic!("expected interruption")
    };
    interruption
}

fn ready_kind(state: &DomainState) -> SessionKind {
    let ProgressState::Ready { next_kind, .. } = state.snapshot().state else {
        panic!("expected ready")
    };
    next_kind
}

fn rejected(state: &mut DomainState, command: Command, at: u64) {
    let before = state.clone();
    assert!(state.apply(command, Timestamp(at)).is_err());
    assert_eq!(*state, before);
    state.validate().unwrap();
}

fn restored(state: &DomainState) -> DomainState {
    DomainState::from_parts(
        state.snapshot().clone(),
        state.history().clone(),
        state.id_allocators().clone(),
    )
    .unwrap()
}

#[test]
fn current_task_is_optional_normalized_and_single_line() {
    for empty in ["", " ", "\t", "\n", "\u{000B}", "\u{000C}"] {
        assert_eq!(CurrentTask::parse(empty).unwrap(), None);
    }
    assert_eq!(
        CurrentTask::parse(" read chapter 1 ")
            .unwrap()
            .unwrap()
            .as_str(),
        "read chapter 1"
    );
    for invalid in [
        "first\nsecond",
        "first\rsecond",
        "first\u{000B}second",
        "first\u{000C}second",
        "first\u{0085}second",
        "first\u{2028}second",
        "first\u{2029}second",
    ] {
        assert_eq!(CurrentTask::parse(invalid), Err(DomainError::InvalidTask));
    }
    let mut state = state();
    assert!(state.history().sessions.is_empty());
    assert!(state.history().events.is_empty());
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    assert!(active(&state).0.current_task.is_none());
    rejected(
        &mut state,
        Command::SetCurrentTask(CurrentTask::parse("changed").unwrap()),
        0,
    );
}

#[test]
fn ordinary_ticks_only_update_the_snapshot_and_completion_is_recorded_once() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    advance(&mut state, 0, 9_000);
    assert_eq!(active(&state).0.elapsed_ms, 9_000);
    assert!(state.history().events.is_empty());
    observe(&mut state, 9_000, 10_000);
    assert_eq!(ready_kind(&state), SessionKind::ShortBreak);
    assert_eq!(state.history().sessions.len(), 1);
    assert_eq!(state.history().events.len(), 1);
    assert_eq!(
        state.history().sessions[0].end.unwrap().outcome,
        SessionOutcome::Completed
    );
    observe(&mut state, 10_000, 11_000);
    assert_eq!(state.history().sessions.len(), 1);
    assert_eq!(
        state.reflection().unwrap(),
        ReflectionSummary {
            work_ms: 10_000,
            completed_focus_sessions: 1,
            distractions: 0,
            returns: 0
        }
    );
}

#[test]
fn completion_before_an_input_cannot_retarget_that_input() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let old_id = active(&state).0.id;
    advance(&mut state, 0, 10_000);
    let completed = state.clone();
    rejected(&mut state, Command::Pause(old_id), 10_000);
    rejected(
        &mut state,
        Command::End {
            session_id: old_id,
            outcome: SessionOutcome::Skipped,
        },
        10_000,
    );
    assert_eq!(state, completed);
    apply(&mut state, Command::Start(SessionKind::ShortBreak), 10_000);
    rejected(&mut state, Command::Pause(old_id), 10_000);
    assert_eq!(active(&state).0.kind, SessionKind::ShortBreak);
}

#[test]
fn quick_start_continues_as_a_new_full_focus_with_task_and_one_link() {
    let mut state = state();
    let task = CurrentTask::parse("read chapter 1").unwrap();
    apply(&mut state, Command::SetCurrentTask(task.clone()), 0);
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    let quick_id = active(&state).0.id;
    advance(&mut state, 0, 120_000);
    assert!(matches!(
        state.snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    assert_eq!(
        state.snapshot().round_progress.completed_focuses_in_round,
        0
    );
    assert_eq!(state.reflection().unwrap().completed_focus_sessions, 0);
    rejected(&mut state, Command::Start(SessionKind::Focus), 120_000);
    apply(&mut state, Command::CloseApp, 120_000);
    state = restored(&state);
    apply(&mut state, Command::RestoreApp, 180_000);
    assert!(matches!(
        state.snapshot().state,
        ProgressState::AwaitingQuickStartDecision { .. }
    ));
    apply(
        &mut state,
        Command::DecideQuickStart {
            session_id: quick_id,
            choice: QuickStartChoice::Continue,
        },
        180_000,
    );
    let focus = active(&state).0;
    assert_ne!(focus.id, quick_id);
    assert_eq!(focus.planned_duration_ms, 10_000);
    assert_eq!(focus.elapsed_ms, 0);
    assert_eq!(focus.current_task, task);
    assert_eq!(focus.continued_from_quick_start, Some(quick_id));
    let focus_id = focus.id;
    rejected(
        &mut state,
        Command::DecideQuickStart {
            session_id: quick_id,
            choice: QuickStartChoice::Continue,
        },
        180_000,
    );
    advance(&mut state, 180_000, 10_000);
    assert_eq!(state.reflection().unwrap().work_ms, 130_000);
    assert_eq!(state.reflection().unwrap().completed_focus_sessions, 1);
    assert_eq!(state.history().sessions[1].id, focus_id);
}

#[test]
fn quick_start_finish_does_not_create_another_session_or_break() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    let id = active(&state).0.id;
    advance(&mut state, 0, 120_000);
    apply(
        &mut state,
        Command::DecideQuickStart {
            session_id: id,
            choice: QuickStartChoice::Finish,
        },
        123_000,
    );
    assert_eq!(ready_kind(&state), SessionKind::Focus);
    assert_eq!(state.id_allocators().next_session_id, 2);
    rejected(
        &mut state,
        Command::DecideQuickStart {
            session_id: id,
            choice: QuickStartChoice::Continue,
        },
        123_000,
    );
}

#[test]
fn reset_of_continued_focus_does_not_reuse_the_quick_start_link() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    let quick = active(&state).0.id;
    advance(&mut state, 0, 120_000);
    apply(
        &mut state,
        Command::DecideQuickStart {
            session_id: quick,
            choice: QuickStartChoice::Continue,
        },
        120_000,
    );
    let first_focus = active(&state).0.id;
    apply(
        &mut state,
        Command::End {
            session_id: first_focus,
            outcome: SessionOutcome::Reset,
        },
        120_000,
    );
    apply(&mut state, Command::Start(SessionKind::Focus), 120_000);
    assert_ne!(active(&state).0.id, first_focus);
    assert_eq!(active(&state).0.continued_from_quick_start, None);
}

#[test]
fn pause_and_distraction_have_distinct_operations_and_results_for_both_work_kinds() {
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        let mut state = state();
        apply(&mut state, Command::Start(kind), 0);
        let id = active(&state).0.id;
        observe(&mut state, 0, 1_000);
        apply(&mut state, Command::Pause(id), 1_000);
        rejected(&mut state, Command::Return(id), 2_000);
        rejected(&mut state, Command::Distraction(id), 2_000);
        apply(&mut state, Command::Resume(id), 2_000);
        observe(&mut state, 2_000, 3_000);
        apply(&mut state, Command::Distraction(id), 3_000);
        rejected(&mut state, Command::Resume(id), 4_000);
        rejected(&mut state, Command::Pause(id), 4_000);
        apply(&mut state, Command::Return(id), 4_000);
        assert_eq!(active(&state).0.elapsed_ms, 2_000);
        assert_eq!(
            state.reflection().unwrap(),
            ReflectionSummary {
                work_ms: 2_000,
                completed_focus_sessions: 0,
                distractions: 1,
                returns: 1
            }
        );
    }
}

#[test]
fn offline_time_counts_toward_recovery_but_not_work() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Distraction(id), 1_000);
    let interruption_id = interruption(&state).id;
    apply(&mut state, Command::CloseApp, 2_000);
    state = restored(&state);
    apply(&mut state, Command::RestoreApp, 90_000);
    assert_eq!(interruption(&state).id, interruption_id);
    assert_eq!(interruption(&state).kind, InterruptionKind::Distraction);
    apply(&mut state, Command::Return(id), 91_000);
    assert_eq!(active(&state).0.elapsed_ms, 1_000);
    assert!(matches!(
        state.history().events.last().unwrap().payload,
        EventKind::InterruptionEnded {
            end: InterruptionEnd {
                outcome: InterruptionOutcome::Returned,
                duration: MeasuredDuration::Known { elapsed_ms: 90_000 },
                ..
            },
            ..
        }
    ));
}

#[test]
fn clock_anomaly_keeps_return_fact_but_marks_recovery_unknown() {
    for (at, monotonic) in [(500, 1_000), (9_000, 100)] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        let id = active(&state).0.id;
        observe(&mut state, 0, 1_000);
        apply(&mut state, Command::Distraction(id), 1_000);
        state
            .observe(Observation {
                previous_at: Timestamp(1_000),
                at: Timestamp(at),
                monotonic_elapsed_ms: Some(monotonic),
            })
            .unwrap();
        assert_eq!(interruption(&state).kind, InterruptionKind::Distraction);
        apply(&mut state, Command::Return(id), 10_000);
        assert!(matches!(
            state.history().events.last().unwrap().payload,
            EventKind::InterruptionEnded {
                end: InterruptionEnd {
                    outcome: InterruptionOutcome::Returned,
                    duration: MeasuredDuration::Unknown { .. },
                    ..
                },
                ..
            }
        ));
        assert_eq!(state.reflection().unwrap().returns, 1);
    }
}

#[test]
fn backward_clock_on_restore_marks_the_existing_distraction_unknown() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Distraction(id), 1_000);
    apply(&mut state, Command::CloseApp, 2_000);
    state = restored(&state);
    apply(&mut state, Command::RestoreApp, 1_500);
    apply(&mut state, Command::Return(id), 3_000);
    assert!(matches!(
        state.history().events.last().unwrap().payload,
        EventKind::InterruptionEnded {
            end: InterruptionEnd {
                duration: MeasuredDuration::Unknown {
                    reason: TimeUncertainty::ClockMovedBackward
                },
                ..
            },
            ..
        }
    ));
}

fn reject_known_interruption_duration(state: &DomainState, elapsed_ms: u64) {
    assert_eq!(restored(state), *state);
    let mut bad = state.clone();
    let EventKind::InterruptionEnded { end, .. } =
        &mut bad.history.events.last_mut().unwrap().payload
    else {
        panic!("expected interruption end")
    };
    assert!(matches!(end.duration, MeasuredDuration::Unknown { .. }));
    end.duration = MeasuredDuration::Known { elapsed_ms };
    assert!(DomainState::from_parts(bad.snapshot, bad.history, bad.ids).is_err());
}

#[test]
fn from_parts_rejects_known_duration_after_lifecycle_clock_rollback() {
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        for interruption_kind in [
            InterruptionKind::Pause,
            InterruptionKind::Distraction,
            InterruptionKind::AppExit,
            InterruptionKind::ObservationGap,
        ] {
            for outcome in [
                None,
                Some(SessionOutcome::Cancelled),
                Some(SessionOutcome::Reset),
                Some(SessionOutcome::Skipped),
            ] {
                let mut state = state();
                apply(&mut state, Command::Start(kind), 0);
                let id = active(&state).0.id;
                observe(&mut state, 0, 1_000);
                let command = match interruption_kind {
                    InterruptionKind::Pause => Command::Pause(id),
                    InterruptionKind::Distraction => Command::Distraction(id),
                    InterruptionKind::AppExit => Command::CloseApp,
                    InterruptionKind::ObservationGap => Command::RestoreApp,
                };
                apply(&mut state, command, 1_000);
                apply(&mut state, Command::CloseApp, 2_000);
                state = restored(&state);
                apply(&mut state, Command::RestoreApp, 1_500);
                let command = match outcome {
                    Some(outcome) => Command::End {
                        session_id: id,
                        outcome,
                    },
                    None if interruption_kind == InterruptionKind::Distraction => {
                        Command::Return(id)
                    }
                    None => Command::Resume(id),
                };
                apply(&mut state, command, 3_000);
                // The endpoints have recovered, but the intervening rollback remains evidence.
                reject_known_interruption_duration(&state, 2_000);
            }
        }
    }
}

#[test]
fn from_parts_rejects_known_duration_when_return_predates_the_last_event() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Distraction(id), 1_000);
    apply(&mut state, Command::CloseApp, 3_000);
    apply(&mut state, Command::RestoreApp, 4_000);
    apply(&mut state, Command::Return(id), 2_000);
    reject_known_interruption_duration(&state, 1_000);
}

#[test]
fn from_parts_rejects_known_duration_after_a_backward_observation_pair() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Distraction(id), 1_000);
    state
        .observe(Observation {
            previous_at: Timestamp(3_000),
            at: Timestamp(2_500),
            monotonic_elapsed_ms: Some(500),
        })
        .unwrap();
    apply(&mut state, Command::Return(id), 4_000);
    // Event recording times alone are increasing; the gap's endpoints prove the rollback.
    reject_known_interruption_duration(&state, 3_000);
}

#[test]
fn from_parts_rejects_known_duration_when_gap_detection_predates_its_boundary() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::RestoreApp, 500);
    apply(&mut state, Command::Resume(id), 2_000);
    reject_known_interruption_duration(&state, 1_000);
}

#[test]
fn from_parts_rejects_lost_clock_evidence_in_an_open_interruption() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Distraction(id), 1_000);
    apply(&mut state, Command::CloseApp, 2_000);
    apply(&mut state, Command::RestoreApp, 1_500);
    assert_eq!(restored(&state), state);
    let ProgressState::Active {
        timer: TimerState::Interrupted { interruption },
        ..
    } = &mut state.snapshot.state
    else {
        panic!("expected interruption")
    };
    interruption.time_uncertainty = None;
    assert!(DomainState::from_parts(state.snapshot, state.history, state.ids).is_err());
}

#[test]
fn from_parts_keeps_clock_evidence_scoped_to_each_interruption() {
    for new_session in [false, true] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        let id = active(&state).0.id;
        observe(&mut state, 0, 1_000);
        apply(&mut state, Command::Pause(id), 1_000);
        apply(&mut state, Command::CloseApp, 4_000);
        apply(&mut state, Command::RestoreApp, 1_500);
        if new_session {
            apply(
                &mut state,
                Command::End {
                    session_id: id,
                    outcome: SessionOutcome::Cancelled,
                },
                1_500,
            );
            apply(&mut state, Command::Start(SessionKind::Focus), 1_500);
        } else {
            apply(&mut state, Command::Resume(id), 1_500);
        }
        let id = active(&state).0.id;
        observe(&mut state, 1_500, 2_000);
        apply(&mut state, Command::Distraction(id), 2_000);
        apply(&mut state, Command::CloseApp, 2_000);
        apply(&mut state, Command::RestoreApp, 2_000);
        apply(&mut state, Command::Return(id), 3_000);
        assert!(matches!(
            state.history().events.last().unwrap().payload,
            EventKind::InterruptionEnded {
                end: InterruptionEnd {
                    duration: MeasuredDuration::Known { elapsed_ms: 1_000 },
                    ..
                },
                ..
            }
        ));
        assert_eq!(restored(&state), state);
    }
}

#[test]
fn interrupted_observation_checks_recorded_times_even_when_the_pair_is_consistent() {
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        for prior_lifecycle_event in [false, true] {
            let mut state = state();
            apply(&mut state, Command::Start(kind), 0);
            let id = active(&state).0.id;
            observe(&mut state, 0, 1_000);
            apply(&mut state, Command::Distraction(id), 1_000);
            if prior_lifecycle_event {
                apply(&mut state, Command::CloseApp, 2_000);
                apply(&mut state, Command::RestoreApp, 3_000);
            }
            let original_id = interruption(&state).id;
            let events = state.history().events.clone();
            let to = if prior_lifecycle_event { 2_500 } else { 500 };
            observe(&mut state, to - 100, to);
            assert_eq!(interruption(&state).id, original_id);
            assert_eq!(interruption(&state).kind, InterruptionKind::Distraction);
            assert_eq!(
                interruption(&state).time_uncertainty,
                Some(TimeUncertainty::ClockMovedBackward)
            );
            assert_eq!(state.history().events, events);
            assert_eq!(active(&state).0.elapsed_ms, 1_000);
            // The evidence must survive a snapshot round trip before Return.
            state = restored(&state);
            apply(&mut state, Command::Return(id), 4_000);
            assert!(matches!(
                state.history().events.last().unwrap().payload,
                EventKind::InterruptionEnded {
                    end: InterruptionEnd {
                        outcome: InterruptionOutcome::Returned,
                        duration: MeasuredDuration::Unknown {
                            reason: TimeUncertainty::ClockMovedBackward
                        },
                        ..
                    },
                    ..
                }
            ));
            assert_eq!(state.reflection().unwrap().returns, 1);
        }
    }
}

#[test]
fn unlinked_running_observations_preserve_the_strongest_clock_evidence() {
    for (previous, at, monotonic, reason, uncertainty) in [
        // Internally consistent, but earlier than the last confirmed timestamp.
        (
            400,
            500,
            Some(100),
            GapReason::ClockAnomaly,
            Some(TimeUncertainty::ClockMovedBackward),
        ),
        // Pair-level rollback even though its end is later than the snapshot.
        (
            2_000,
            1_500,
            Some(100),
            GapReason::ClockAnomaly,
            Some(TimeUncertainty::ClockMovedBackward),
        ),
        (
            2_000,
            5_000,
            Some(100),
            GapReason::ClockAnomaly,
            Some(TimeUncertainty::ClockDiscontinuity),
        ),
        (
            2_000,
            2_100,
            None,
            GapReason::ObservationDiscontinuity,
            Some(TimeUncertainty::InsufficientClockEvidence),
        ),
        (
            2_000,
            8_000,
            Some(6_000),
            GapReason::ObservationDiscontinuity,
            None,
        ),
        // A healthy pair still cannot credit time across a missing link.
        (
            2_000,
            2_100,
            Some(100),
            GapReason::ObservationDiscontinuity,
            Some(TimeUncertainty::InsufficientClockEvidence),
        ),
    ] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        observe(&mut state, 0, 1_000);
        state
            .observe(Observation {
                previous_at: Timestamp(previous),
                at: Timestamp(at),
                monotonic_elapsed_ms: monotonic,
            })
            .unwrap();
        assert_eq!(interruption(&state).kind, InterruptionKind::ObservationGap);
        assert_eq!(interruption(&state).started_at, Timestamp(1_000));
        assert_eq!(interruption(&state).recorded_at, Timestamp(at));
        assert_eq!(interruption(&state).time_uncertainty, uncertainty);
        assert_eq!(active(&state).0.elapsed_ms, 1_000);
        assert!(matches!(
            state.history().events[0].payload,
            EventKind::ObservationGapDetected { reason: actual, .. } if actual == reason
        ));
        assert_eq!(restored(&state), state);
    }
}

fn ready_for(kind: SessionKind) -> DomainState {
    let mut state = state();
    match kind {
        SessionKind::ShortBreak => apply(&mut state, Command::SkipReady, 0),
        SessionKind::LongBreak => {
            for index in 0..2 {
                let at = index * 10_000;
                apply(&mut state, Command::Start(SessionKind::Focus), at);
                advance(&mut state, at, 10_000);
                if index == 0 {
                    apply(&mut state, Command::SkipReady, at + 10_000);
                }
            }
        }
        SessionKind::Focus | SessionKind::QuickStart => {}
    }
    state
}

#[test]
fn all_session_kinds_stop_at_observation_gaps_before_completion() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut state = ready_for(kind);
        apply(&mut state, Command::Start(kind), 30_000);
        observe(&mut state, 30_000, 31_000);
        let count = state.history().sessions.len();
        observe(&mut state, 31_000, 300_000);
        assert_eq!(active(&state).0.elapsed_ms, 1_000);
        assert_eq!(interruption(&state).kind, InterruptionKind::ObservationGap);
        assert_eq!(interruption(&state).started_at, Timestamp(31_000));
        assert_eq!(interruption(&state).recorded_at, Timestamp(300_000));
        assert_eq!(state.history().sessions.len(), count);
        let id = active(&state).0.id;
        rejected(&mut state, Command::Return(id), 300_000);
        apply(&mut state, Command::Resume(id), 300_000);
        observe(&mut state, 300_000, 301_000);
        assert_eq!(active(&state).0.elapsed_ms, 2_000);
    }
}

#[test]
fn all_session_kinds_stop_on_exit_and_on_running_snapshot_restore() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        for normal_exit in [false, true] {
            let mut state = ready_for(kind);
            apply(&mut state, Command::Start(kind), 30_000);
            observe(&mut state, 30_000, 31_000);
            if normal_exit {
                apply(&mut state, Command::CloseApp, 31_000);
            }
            state = restored(&state);
            apply(&mut state, Command::RestoreApp, 900_000);
            assert_eq!(active(&state).0.elapsed_ms, 1_000);
            assert_eq!(
                interruption(&state).kind,
                if normal_exit {
                    InterruptionKind::AppExit
                } else {
                    InterruptionKind::ObservationGap
                }
            );
            assert_eq!(interruption(&state).started_at, Timestamp(31_000));
        }
    }
}

#[test]
fn long_pause_gap_and_app_boundaries_do_not_change_interruption_kind() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    apply(&mut state, Command::Pause(id), 0);
    let original = interruption(&state).clone();
    observe(&mut state, 0, 100_000);
    apply(&mut state, Command::CloseApp, 100_000);
    apply(&mut state, Command::RestoreApp, 200_000);
    assert_eq!(interruption(&state), &original);
    assert_eq!(state.id_allocators().next_interruption_id, 2);
    apply(&mut state, Command::Resume(id), 200_000);
    assert_eq!(state.reflection().unwrap().distractions, 0);
}

#[test]
fn observation_policy_has_tested_boundaries_and_requires_clock_evidence() {
    for (elapsed, interrupted) in [(5_000, false), (5_001, true)] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        observe(&mut state, 0, elapsed);
        assert_eq!(
            matches!(active(&state).1, TimerState::Interrupted { .. }),
            interrupted
        );
    }
    for evidence in [
        Observation {
            previous_at: Timestamp(0),
            at: Timestamp(100),
            monotonic_elapsed_ms: None,
        },
        Observation {
            previous_at: Timestamp(10),
            at: Timestamp(100),
            monotonic_elapsed_ms: Some(90),
        },
    ] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        state.observe(evidence).unwrap();
        assert_eq!(interruption(&state).kind, InterruptionKind::ObservationGap);
        assert_eq!(active(&state).0.elapsed_ms, 0);
        state.validate().unwrap();
    }
}

#[test]
fn continuous_overshoot_credits_only_the_planned_time() {
    let mut state = DomainState::new(TimerConfig::new(1, 1, 1, 2).unwrap()).unwrap();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    observe(&mut state, 0, 5_000);
    assert_eq!(state.history().sessions[0].elapsed_ms, 1_000);
    assert_eq!(state.reflection().unwrap().work_ms, 1_000);
}

#[test]
fn ready_does_not_manufacture_sessions_or_events() {
    let mut state = state();
    for command in [
        Command::ResetReady,
        Command::CloseApp,
        Command::RestoreApp,
        Command::SkipReady,
    ] {
        apply(&mut state, command, 100_000);
    }
    observe(&mut state, 100_000, 900_000);
    assert!(state.history().sessions.is_empty());
    assert!(state.history().events.is_empty());
    assert_eq!(state.id_allocators(), &IdAllocators::default());
    assert_eq!(ready_kind(&state), SessionKind::ShortBreak);
    rejected(&mut state, Command::Start(SessionKind::QuickStart), 900_000);
    rejected(
        &mut state,
        Command::SetCurrentTask(CurrentTask::parse("work").unwrap()),
        900_000,
    );
}

#[test]
fn breaks_allow_pause_but_never_distraction_or_task() {
    for kind in [SessionKind::ShortBreak, SessionKind::LongBreak] {
        let mut state = ready_for(kind);
        apply(&mut state, Command::Start(kind), 30_000);
        let id = active(&state).0.id;
        rejected(&mut state, Command::Distraction(id), 30_000);
        rejected(
            &mut state,
            Command::SetCurrentTask(CurrentTask::parse("work").unwrap()),
            30_000,
        );
        apply(&mut state, Command::Pause(id), 30_000);
        apply(&mut state, Command::Resume(id), 31_000);
        assert!(active(&state).0.current_task.is_none());
    }
}

#[test]
fn every_noncompletion_outcome_is_retained_for_focus_and_quick_start() {
    for kind in [SessionKind::Focus, SessionKind::QuickStart] {
        for outcome in [
            SessionOutcome::Cancelled,
            SessionOutcome::Reset,
            SessionOutcome::Skipped,
        ] {
            for distracted in [false, true] {
                let mut state = state();
                let task = CurrentTask::parse("work").unwrap();
                apply(&mut state, Command::SetCurrentTask(task.clone()), 0);
                apply(&mut state, Command::Start(kind), 0);
                let id = active(&state).0.id;
                observe(&mut state, 0, 1_000);
                if distracted {
                    apply(&mut state, Command::Distraction(id), 1_000);
                }
                apply(
                    &mut state,
                    Command::End {
                        session_id: id,
                        outcome,
                    },
                    1_000,
                );
                assert_eq!(state.history().sessions[0].end.unwrap().outcome, outcome);
                assert_eq!(state.reflection().unwrap().work_ms, 1_000);
                assert_eq!(state.reflection().unwrap().completed_focus_sessions, 0);
                assert_eq!(state.reflection().unwrap().returns, 0);
                assert_eq!(
                    state.snapshot().round_progress.completed_focuses_in_round,
                    0
                );
                if outcome == SessionOutcome::Reset {
                    assert_eq!(
                        state.snapshot().state,
                        ProgressState::Ready {
                            next_kind: kind,
                            current_task_draft: task
                        }
                    );
                    apply(&mut state, Command::Start(kind), 1_000);
                    assert_ne!(active(&state).0.id, id);
                }
            }
        }
    }
}

#[test]
fn long_break_resets_round_on_completion_cancel_or_skip_but_not_reset() {
    for outcome in [
        SessionOutcome::Completed,
        SessionOutcome::Cancelled,
        SessionOutcome::Reset,
        SessionOutcome::Skipped,
    ] {
        let mut state = ready_for(SessionKind::LongBreak);
        assert_eq!(
            state.snapshot().round_progress.completed_focuses_in_round,
            2
        );
        apply(&mut state, Command::Start(SessionKind::LongBreak), 30_000);
        let id = active(&state).0.id;
        if outcome == SessionOutcome::Completed {
            advance(&mut state, 30_000, 20_000);
        } else {
            apply(
                &mut state,
                Command::End {
                    session_id: id,
                    outcome,
                },
                30_000,
            );
        }
        assert_eq!(
            state.snapshot().round_progress.completed_focuses_in_round,
            if outcome == SessionOutcome::Reset {
                2
            } else {
                0
            }
        );
        assert_eq!(
            ready_kind(&state),
            if outcome == SessionOutcome::Reset {
                SessionKind::LongBreak
            } else {
                SessionKind::Focus
            }
        );
        assert_eq!(state.reflection().unwrap().work_ms, 20_000);
    }
    let mut state = ready_for(SessionKind::LongBreak);
    apply(&mut state, Command::SkipReady, 30_000);
    assert_eq!(
        state.snapshot().round_progress.completed_focuses_in_round,
        0
    );
    assert_eq!(state.history().sessions.len(), 2);
}

#[test]
fn settings_change_only_in_ready_and_do_not_rewrite_history() {
    let mut state = ready_for(SessionKind::LongBreak);
    let history = state.history().clone();
    let settings = TimerConfig::new(20, 10, 30, 3).unwrap();
    apply(&mut state, Command::Configure(settings), 30_000);
    assert_eq!(
        state.snapshot().round_progress.completed_focuses_in_round,
        0
    );
    assert_eq!(ready_kind(&state), SessionKind::LongBreak);
    assert_eq!(state.history(), &history);
    apply(&mut state, Command::Start(SessionKind::LongBreak), 30_000);
    rejected(
        &mut state,
        Command::Configure(TimerConfig::default()),
        30_000,
    );
    assert_eq!(active(&state).0.planned_duration_ms, 30_000);
}

#[test]
fn from_parts_requires_active_duration_to_match_snapshot_settings() {
    for kind in [
        SessionKind::Focus,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        for paused in [false, true] {
            let mut state = ready_for(kind);
            apply(&mut state, Command::Start(kind), 30_000);
            observe(&mut state, 30_000, 31_000);
            if paused {
                let id = active(&state).0.id;
                apply(&mut state, Command::Pause(id), 31_000);
            }
            assert_eq!(restored(&state), state);

            let expected_ms = state.snapshot().settings.duration_millis(kind);
            for duration_ms in [expected_ms - 1_000, expected_ms + 1_000] {
                let mut snapshot = state.snapshot().clone();
                let ProgressState::Active { session, .. } = &mut snapshot.state else {
                    unreachable!();
                };
                session.planned_duration_ms = duration_ms;

                let loaded = DomainState::from_parts(
                    snapshot,
                    state.history().clone(),
                    state.id_allocators().clone(),
                );
                assert_eq!(
                    loaded.err(),
                    Some(DomainError::InvalidState("active session duration")),
                    "{kind:?}, paused={paused}, duration_ms={duration_ms}",
                );
            }
        }
    }
}

#[test]
fn from_parts_preserves_historical_durations_after_settings_change() {
    for kind in [
        SessionKind::Focus,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut state = ready_for(kind);
        let duration_ms = state.snapshot().settings.duration_millis(kind);
        apply(&mut state, Command::Start(kind), 30_000);
        advance(&mut state, 30_000, duration_ms);
        let history = state.history().clone();

        let settings = TimerConfig::new(11, 6, 21, 2).unwrap();
        let at = 30_000 + duration_ms;
        apply(&mut state, Command::Configure(settings), at);
        assert_ne!(duration_ms, settings.duration_millis(kind));
        assert_eq!(restored(&state), state);

        let next_kind = ready_kind(&state);
        apply(&mut state, Command::Start(next_kind), at);
        assert_eq!(
            active(&state).0.planned_duration_ms,
            settings.duration_millis(next_kind),
        );
        assert_eq!(state.history(), &history);
        assert_eq!(restored(&state), state);
    }
}

#[test]
fn invalid_commands_and_missing_observation_do_not_allocate_ids_or_emit_events() {
    let mut state = state();
    for command in [
        Command::Pause(SessionId(1)),
        Command::Resume(SessionId(1)),
        Command::Return(SessionId(1)),
        Command::Distraction(SessionId(1)),
    ] {
        rejected(&mut state, command, 0);
    }
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    for command in [
        Command::Start(SessionKind::QuickStart),
        Command::Resume(id),
        Command::Return(id),
        Command::SkipReady,
        Command::ResetReady,
        Command::End {
            session_id: id,
            outcome: SessionOutcome::Completed,
        },
    ] {
        rejected(&mut state, command, 0);
    }
    assert_eq!(
        state.apply(Command::Pause(id), Timestamp(1_000)),
        Err(DomainError::ObservationRequired)
    );
    assert!(state.history().events.is_empty());
    assert_eq!(active(&state).0.elapsed_ms, 0);
}

#[test]
fn overflow_rolls_back_even_after_an_interval_was_appended() {
    let mut state = state();
    state.ids.next_session_id = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.apply(Command::Start(SessionKind::Focus), Timestamp(0)),
        Err(DomainError::Overflow)
    );
    assert_eq!(state, before);
    state.ids.next_session_id = 1;
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    state.ids.next_interruption_id = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.apply(Command::Pause(SessionId(1)), Timestamp(0)),
        Err(DomainError::Overflow)
    );
    assert_eq!(state, before);
    state.ids.next_interruption_id = 1;
    state.ids.next_event_sequence = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.apply(Command::CloseApp, Timestamp(0)),
        Err(DomainError::Overflow)
    );
    assert_eq!(state, before);
}

#[test]
fn cross_object_validation_rejects_corruption_without_replay_or_repair() {
    let mut original = state();
    apply(&mut original, Command::Start(SessionKind::Focus), 0);
    observe(&mut original, 0, 1_000);
    apply(&mut original, Command::Pause(SessionId(1)), 1_000);
    let mut corruptions = Vec::new();
    let mut bad = original.clone();
    bad.history.events[0].session_id = SessionId(999);
    corruptions.push(bad);
    let mut bad = original.clone();
    bad.history.events[0].sequence = 2;
    corruptions.push(bad);
    let mut bad = original.clone();
    if let EventKind::RunIntervalRecorded { credited_ms, .. } = &mut bad.history.events[0].payload {
        *credited_ms += 1;
    }
    corruptions.push(bad);
    let mut bad = original.clone();
    if let ProgressState::Active { session, .. } = &mut bad.snapshot.state {
        session.elapsed_ms += 1;
    }
    corruptions.push(bad);
    let mut bad = original.clone();
    if let ProgressState::Active {
        timer: TimerState::Interrupted { interruption },
        ..
    } = &mut bad.snapshot.state
    {
        interruption.id = InterruptionId(999);
    }
    corruptions.push(bad);
    let mut bad = original.clone();
    bad.ids.next_session_id = 1;
    corruptions.push(bad);
    for bad in corruptions {
        assert!(DomainState::from_parts(bad.snapshot, bad.history, bad.ids).is_err());
    }
    assert_eq!(restored(&original), original);
}

fn reject_inserted_event(
    state: &DomainState,
    index: usize,
    session_id: SessionId,
    payload: EventKind,
) {
    let mut bad = state.clone();
    let at = Timestamp(200_000);
    bad.history.events.insert(
        index,
        HistoryEvent {
            sequence: u64::try_from(index).unwrap() + 1,
            session_id,
            effective_at: at,
            recorded_at: at,
            payload,
        },
    );
    for event in &mut bad.history.events[index + 1..] {
        event.sequence += 1;
    }
    bad.ids.next_event_sequence += 1;
    assert!(DomainState::from_parts(bad.snapshot, bad.history, bad.ids).is_err());
}

fn gap_event() -> EventKind {
    EventKind::ObservationGapDetected {
        last_confirmed_at: Timestamp(200_000),
        detected_at: Timestamp(200_000),
        reason: GapReason::ObservationDiscontinuity,
    }
}

#[test]
fn history_rejects_lifecycle_and_gap_events_after_session_end() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        for outcome in [
            SessionOutcome::Completed,
            SessionOutcome::Cancelled,
            SessionOutcome::Reset,
            SessionOutcome::Skipped,
        ] {
            for interrupted in [false, true] {
                if interrupted && outcome == SessionOutcome::Completed {
                    continue;
                }
                let mut state = ready_for(kind);
                apply(&mut state, Command::Start(kind), 30_000);
                let id = active(&state).0.id;
                if outcome == SessionOutcome::Completed {
                    let duration = active(&state).0.planned_duration_ms;
                    advance(&mut state, 30_000, duration);
                } else {
                    observe(&mut state, 30_000, 31_000);
                    if interrupted {
                        apply(&mut state, Command::Pause(id), 31_000);
                    }
                    apply(
                        &mut state,
                        Command::End {
                            session_id: id,
                            outcome,
                        },
                        31_000,
                    );
                }
                assert_eq!(restored(&state), state);
                let index = state.history().events.len();
                reject_inserted_event(&state, index, id, gap_event());
                // Completed Quick Start still permits lifecycle events while awaiting a decision.
                if kind != SessionKind::QuickStart || outcome != SessionOutcome::Completed {
                    reject_inserted_event(&state, index, id, EventKind::AppClosing);
                    reject_inserted_event(&state, index, id, EventKind::AppRestored);
                }
            }
        }
    }
}

#[test]
fn lifecycle_events_require_the_running_session_to_have_been_interrupted() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    let id = active(&state).0.id;
    observe(&mut state, 0, 1_000);
    apply(&mut state, Command::Pause(id), 1_000);
    // Lifecycle events are emitted after both RunIntervalRecorded and InterruptionStarted.
    for index in [0, 1] {
        reject_inserted_event(&state, index, id, EventKind::AppClosing);
        reject_inserted_event(&state, index, id, EventKind::AppRestored);
    }
}

#[test]
fn quick_start_lifecycle_events_are_valid_only_before_the_decision() {
    for choice in [QuickStartChoice::Finish, QuickStartChoice::Continue] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
        let id = active(&state).0.id;
        advance(&mut state, 0, 120_000);
        apply(&mut state, Command::CloseApp, 121_000);
        apply(&mut state, Command::RestoreApp, 122_000);
        assert_eq!(restored(&state), state);
        apply(
            &mut state,
            Command::DecideQuickStart {
                session_id: id,
                choice,
            },
            123_000,
        );
        // Past lifecycle events remain valid in both Ready and a different Active session.
        assert_eq!(restored(&state), state);
        let index = state.history().events.len();
        reject_inserted_event(&state, index, id, EventKind::AppClosing);
        reject_inserted_event(&state, index, id, EventKind::AppRestored);
        reject_inserted_event(&state, index, id, gap_event());
    }
}

#[test]
fn ended_sessions_keep_valid_gap_and_lifecycle_events_from_their_interruptions() {
    for kind in [
        InterruptionKind::Pause,
        InterruptionKind::Distraction,
        InterruptionKind::AppExit,
        InterruptionKind::ObservationGap,
    ] {
        let mut state = state();
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        let id = active(&state).0.id;
        observe(&mut state, 0, 1_000);
        let command = match kind {
            InterruptionKind::Pause => Command::Pause(id),
            InterruptionKind::Distraction => Command::Distraction(id),
            InterruptionKind::AppExit => Command::CloseApp,
            InterruptionKind::ObservationGap => Command::RestoreApp,
        };
        apply(&mut state, command, 1_000);
        let interruption_id = interruption(&state).id;
        observe(&mut state, 1_000, 10_000);
        assert_eq!(interruption(&state).id, interruption_id);
        assert_eq!(interruption(&state).kind, kind);
        assert_eq!(restored(&state), state);
        apply(&mut state, Command::CloseApp, 10_000);
        // Wall-clock order cannot be used as a substitute for event sequence.
        apply(&mut state, Command::RestoreApp, 9_000);
        let command = if kind == InterruptionKind::Distraction {
            Command::Return(id)
        } else {
            Command::Resume(id)
        };
        apply(&mut state, command, 12_000);
        assert_eq!(restored(&state), state);
        advance(&mut state, 12_000, 9_000);
        assert_eq!(ready_kind(&state), SessionKind::ShortBreak);
        assert_eq!(restored(&state), state);
        apply(&mut state, Command::Start(SessionKind::ShortBreak), 21_000);
        assert_eq!(restored(&state), state);
    }
}

#[test]
fn candidate_application_does_not_mean_the_committed_state_has_changed() {
    let committed = state();
    let mut candidate = committed.clone();
    apply(&mut candidate, Command::Start(SessionKind::Focus), 0);
    assert!(matches!(
        committed.snapshot().state,
        ProgressState::Ready { .. }
    ));
    assert!(matches!(
        candidate.snapshot().state,
        ProgressState::Active { .. }
    ));
    assert_eq!(committed.id_allocators().next_session_id, 1);
    assert_eq!(candidate.id_allocators().next_session_id, 2);
    // Saving/retrying this exact candidate is an application/platform concern.
}

#[test]
fn rejected_quick_start_continuation_rolls_back_the_new_focus_and_id() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    let id = active(&state).0.id;
    advance(&mut state, 0, 120_000);
    state.ids.next_event_sequence = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.apply(
            Command::DecideQuickStart {
                session_id: id,
                choice: QuickStartChoice::Continue
            },
            Timestamp(120_000)
        ),
        Err(DomainError::Overflow)
    );
    assert_eq!(state, before);
}

#[test]
fn validation_rejects_missing_or_mismatched_quick_start_decisions() {
    let mut state = state();
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    let id = active(&state).0.id;
    advance(&mut state, 0, 120_000);
    apply(
        &mut state,
        Command::DecideQuickStart {
            session_id: id,
            choice: QuickStartChoice::Continue,
        },
        120_000,
    );
    let mut bad = state.clone();
    bad.history.events.pop();
    bad.ids.next_event_sequence -= 1;
    assert!(bad.validate().is_err());
    let mut bad = state.clone();
    bad.history.events.swap(0, 1);
    bad.history.events[0].sequence = 1;
    bad.history.events[1].sequence = 2;
    assert!(bad.validate().is_err());
    let mut bad = state;
    if let ProgressState::Active { session, .. } = &mut bad.snapshot.state {
        session.continued_from_quick_start = None;
    }
    assert!(bad.validate().is_err());
}

#[test]
fn deterministic_mixed_operations_preserve_invariants_and_atomic_rejection() {
    for seed in 1..=8_u64 {
        let mut random = seed;
        let mut state = DomainState::new(TimerConfig::new(1, 1, 1, 2).unwrap()).unwrap();
        let mut at = 1_000_u64;
        for _ in 0..400 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let previous_at = at;
            at = match random % 13 {
                0 => at.saturating_sub(500),
                1 => at + 60_000,
                _ => at + 250,
            };
            state
                .observe(Observation {
                    previous_at: Timestamp(previous_at),
                    at: Timestamp(at),
                    monotonic_elapsed_ms: Some(250),
                })
                .unwrap();
            state.validate().unwrap();
            let id = match &state.snapshot().state {
                ProgressState::Active { session, .. } => session.id,
                ProgressState::AwaitingQuickStartDecision {
                    quick_start_session_id,
                    ..
                } => *quick_start_session_id,
                ProgressState::Ready { .. } => SessionId(1),
            };
            let command = match (random >> 32) % 19 {
                0 => Command::Start(SessionKind::Focus),
                1 => Command::Start(SessionKind::QuickStart),
                2 => Command::Start(SessionKind::ShortBreak),
                3 => Command::Start(SessionKind::LongBreak),
                4 => Command::Pause(id),
                5 => Command::Distraction(id),
                6 => Command::Resume(id),
                7 => Command::Return(id),
                8 => Command::End {
                    session_id: id,
                    outcome: SessionOutcome::Cancelled,
                },
                9 => Command::End {
                    session_id: id,
                    outcome: SessionOutcome::Reset,
                },
                10 => Command::End {
                    session_id: id,
                    outcome: SessionOutcome::Skipped,
                },
                11 => Command::SkipReady,
                12 => Command::ResetReady,
                13 => Command::DecideQuickStart {
                    session_id: id,
                    choice: QuickStartChoice::Continue,
                },
                14 => Command::DecideQuickStart {
                    session_id: id,
                    choice: QuickStartChoice::Finish,
                },
                15 => Command::CloseApp,
                16 => Command::RestoreApp,
                17 => Command::SetCurrentTask(CurrentTask::parse("read").unwrap()),
                _ => Command::Configure(TimerConfig::new(1, 1, 1, 2).unwrap()),
            };
            let before = state.clone();
            if state.apply(command, Timestamp(at)).is_err() {
                assert_eq!(state, before);
            }
            state.validate().unwrap();
        }
    }
}
