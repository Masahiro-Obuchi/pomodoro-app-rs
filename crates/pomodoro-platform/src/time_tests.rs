use super::*;
use pomodoro_core::{
    Command, DomainState, EventKind, GapReason, Interruption, InterruptionKind, ProgressState,
    Session, SessionId, SessionKind, TimeUncertainty, TimerConfig, TimerState,
};

fn reading(wall_ms: u64, monotonic: Instant) -> (SystemTime, Instant) {
    (
        SystemTime::UNIX_EPOCH + Duration::from_millis(wall_ms),
        monotonic,
    )
}

fn started(kind: SessionKind, at: Timestamp) -> DomainState {
    let initial = DomainState::new(TimerConfig::new(2, 2, 2, 1).unwrap()).unwrap();
    let mut snapshot = initial.snapshot().clone();
    snapshot.state = ProgressState::Ready {
        next_kind: kind,
        current_task_draft: None,
    };
    let mut domain = DomainState::from_parts(
        snapshot,
        initial.history().clone(),
        initial.id_allocators().clone(),
    )
    .unwrap();
    domain.apply(Command::Start(kind), at).unwrap();
    domain
}

fn active(domain: &DomainState) -> (&Session, &TimerState) {
    let ProgressState::Active { session, timer } = &domain.snapshot().state else {
        panic!("expected an active session");
    };
    (session, timer)
}

fn interruption(domain: &DomainState) -> &Interruption {
    let TimerState::Interrupted { interruption } = active(domain).1 else {
        panic!("expected an interruption");
    };
    interruption
}

#[test]
fn continuous_observations_use_monotonic_elapsed_and_preserve_utc_boundaries() {
    let base = Instant::now();
    let mut clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
    let mut domain = started(SessionKind::Focus, clock.at());
    let observation = clock
        .observe_reading(reading(1_125, base + Duration::from_millis(100)))
        .unwrap();
    assert_eq!(
        observation,
        Observation {
            previous_at: Timestamp(1_000),
            at: Timestamp(1_125),
            monotonic_elapsed_ms: Some(100),
        }
    );
    domain.observe(observation).unwrap();
    assert_eq!(active(&domain).0.elapsed_ms, 100);
    let observation = clock
        .observe_reading(reading(1_200, base + Duration::from_millis(200)))
        .unwrap();
    assert_eq!(observation.previous_at, Timestamp(1_125));
    assert_eq!(observation.monotonic_elapsed_ms, Some(100));
    domain.observe(observation).unwrap();
    assert_eq!(active(&domain).0.elapsed_ms, 200);
    assert_eq!(clock.at(), Timestamp(1_200));
    assert!(domain.history().events.is_empty());
}

#[test]
fn submillisecond_ticks_do_not_accumulate_rounding_loss() {
    let base = Instant::now();
    let mut clock = ObservationClock::from_reading(reading(0, base)).unwrap();
    let mut elapsed = 0;
    for tick in 1..=10 {
        let nanos = tick * 600_000;
        let observation = clock
            .observe_reading(reading(
                nanos / 1_000_000,
                base + Duration::from_nanos(nanos),
            ))
            .unwrap();
        elapsed += observation.monotonic_elapsed_ms.unwrap();
        assert_eq!(elapsed, nanos / 1_000_000);
    }
    assert_eq!(elapsed, 6);
}

#[test]
fn wall_clock_anomalies_are_forwarded_to_core_without_clamping() {
    for (wall, uncertainty) in [
        (900, TimeUncertainty::ClockMovedBackward),
        (10_000, TimeUncertainty::ClockDiscontinuity),
    ] {
        let base = Instant::now();
        let mut clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
        let mut domain = started(SessionKind::Focus, clock.at());
        let observation = clock
            .observe_reading(reading(wall, base + Duration::from_millis(100)))
            .unwrap();
        assert_eq!(observation.at, Timestamp(wall));
        assert_eq!(observation.monotonic_elapsed_ms, Some(100));
        domain.observe(observation).unwrap();
        assert_eq!(active(&domain).0.elapsed_ms, 0);
        assert_eq!(interruption(&domain).time_uncertainty, Some(uncertainty));
        assert!(domain.history().events.iter().any(|event| matches!(
            event.payload,
            EventKind::ObservationGapDetected {
                reason: GapReason::ClockAnomaly,
                ..
            }
        )));
    }
}

#[test]
fn long_gaps_are_forwarded_to_core_before_completion_for_every_session_kind() {
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let base = Instant::now();
        let mut clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
        let mut domain = started(kind, clock.at());
        let observation = clock
            .observe_reading(reading(181_000, base + Duration::from_secs(180)))
            .unwrap();
        assert_eq!(observation.monotonic_elapsed_ms, Some(180_000));
        domain.observe(observation).unwrap();
        assert_eq!(active(&domain).0.elapsed_ms, 0);
        assert_eq!(interruption(&domain).kind, InterruptionKind::ObservationGap);
        assert_eq!(interruption(&domain).time_uncertainty, None);
        assert!(domain.history().sessions.is_empty());
    }
}

#[test]
fn short_save_failure_breaks_continuity_without_app_restored_or_elapsed_credit() {
    let base = Instant::now();
    let mut clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
    let mut domain = started(SessionKind::Focus, clock.at());
    domain
        .observe(
            clock
                .observe_reading(reading(1_100, base + Duration::from_millis(100)))
                .unwrap(),
        )
        .unwrap();
    clock.break_continuity();
    clock.break_continuity(); // Retrying the save does not move its UTC boundary.
    assert_eq!(clock.at(), Timestamp(1_100));
    let observation = clock
        .observe_reading(reading(1_150, base + Duration::from_millis(150)))
        .unwrap();
    assert_eq!(observation.previous_at, Timestamp(1_100));
    assert_eq!(observation.monotonic_elapsed_ms, None);
    domain.observe(observation).unwrap();
    assert_eq!(active(&domain).0.elapsed_ms, 100);
    assert_eq!(interruption(&domain).started_at, Timestamp(1_100));
    assert_eq!(interruption(&domain).recorded_at, Timestamp(1_150));
    assert_eq!(
        interruption(&domain).time_uncertainty,
        Some(TimeUncertainty::InsufficientClockEvidence)
    );
    assert!(
        !domain
            .history()
            .events
            .iter()
            .any(|event| matches!(event.payload, EventKind::AppRestored))
    );
    // Applying and saving the gap precedes any application-level permission to resume.
    domain
        .apply(Command::Resume(SessionId(1)), clock.at())
        .unwrap();
    domain
        .observe(
            clock
                .observe_reading(reading(1_250, base + Duration::from_millis(250)))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(active(&domain).0.elapsed_ms, 200);
}

#[test]
fn losing_continuity_discards_uncredited_fractions() {
    let base = Instant::now();
    let mut clock = ObservationClock::from_reading(reading(0, base)).unwrap();
    assert_eq!(
        clock
            .observe_reading(reading(0, base + Duration::from_micros(900)))
            .unwrap()
            .monotonic_elapsed_ms,
        Some(0)
    );
    clock.break_continuity();
    assert_eq!(
        clock
            .observe_reading(reading(1, base + Duration::from_micros(1_000)))
            .unwrap()
            .monotonic_elapsed_ms,
        None
    );
    assert_eq!(
        clock
            .observe_reading(reading(1, base + Duration::from_micros(1_200)))
            .unwrap()
            .monotonic_elapsed_ms,
        Some(0)
    );
}

#[test]
fn backward_monotonic_reading_is_missing_evidence_not_zero_elapsed() {
    let base = Instant::now();
    let mut clock =
        ObservationClock::from_reading(reading(1_000, base + Duration::from_secs(1))).unwrap();
    let mut domain = started(SessionKind::Focus, clock.at());
    let observation = clock.observe_reading(reading(1_100, base)).unwrap();
    assert_eq!(observation.monotonic_elapsed_ms, None);
    domain.observe(observation).unwrap();
    assert_eq!(active(&domain).0.elapsed_ms, 0);
    assert_eq!(
        interruption(&domain).time_uncertainty,
        Some(TimeUncertainty::InsufficientClockEvidence)
    );
}

#[test]
fn failed_utc_reading_invalidates_anchor_and_retains_the_previous_boundary() {
    let base = Instant::now();
    let invalid_wall = SystemTime::UNIX_EPOCH - Duration::from_millis(1);
    assert!(matches!(
        ObservationClock::from_reading((invalid_wall, base)),
        Err(TimeError::BeforeUnixEpoch)
    ));
    let mut clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
    assert_eq!(
        clock.observe_reading((invalid_wall, base + Duration::from_millis(50))),
        Err(TimeError::BeforeUnixEpoch)
    );
    assert_eq!(clock.at(), Timestamp(1_000));
    assert_eq!(
        clock
            .observe_reading(reading(1_100, base + Duration::from_millis(100)))
            .unwrap(),
        Observation {
            previous_at: Timestamp(1_000),
            at: Timestamp(1_100),
            monotonic_elapsed_ms: None,
        }
    );
}

#[test]
fn elapsed_conversion_overflow_does_not_saturate_or_wrap() {
    assert_eq!(
        split_milliseconds(Duration::from_millis(u64::MAX)),
        Some((u64::MAX, Duration::ZERO))
    );
    assert_eq!(
        split_milliseconds(Duration::from_millis(u64::MAX) + Duration::from_millis(1)),
        None
    );
    assert_eq!(split_milliseconds(Duration::MAX), None);
}

#[test]
fn restart_uses_a_fresh_anchor_and_never_credits_offline_time() {
    let base = Instant::now();
    let mut old_clock = ObservationClock::from_reading(reading(1_000, base)).unwrap();
    let mut saved_domain = started(SessionKind::Focus, old_clock.at());
    saved_domain
        .observe(
            old_clock
                .observe_reading(reading(1_100, base + Duration::from_millis(100)))
                .unwrap(),
        )
        .unwrap();
    let restarted_at = base + Duration::from_secs(60);
    let mut clock = ObservationClock::from_reading(reading(61_000, restarted_at)).unwrap();
    saved_domain.apply(Command::RestoreApp, clock.at()).unwrap();
    assert_eq!(active(&saved_domain).0.elapsed_ms, 100);
    assert_eq!(interruption(&saved_domain).started_at, Timestamp(1_100));
    let observation = clock
        .observe_reading(reading(61_100, restarted_at + Duration::from_millis(100)))
        .unwrap();
    assert_eq!(observation.previous_at, Timestamp(61_000));
    assert_eq!(observation.monotonic_elapsed_ms, Some(100));
    saved_domain.observe(observation).unwrap();
    assert_eq!(active(&saved_domain).0.elapsed_ms, 100);
    saved_domain
        .apply(Command::Resume(SessionId(1)), clock.at())
        .unwrap();
    saved_domain
        .observe(
            clock
                .observe_reading(reading(61_200, restarted_at + Duration::from_millis(200)))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(active(&saved_domain).0.elapsed_ms, 200);
}
