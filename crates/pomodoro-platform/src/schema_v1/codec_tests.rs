use pomodoro_core::{
    Command, CurrentTask, DomainState, Observation, ProgressState, QuickStartChoice, SessionId,
    SessionKind, SessionOutcome, TimerConfig, Timestamp,
};
use serde_json::{Value, json};

use super::{CodecError, PersistedStateV1};

fn candidate(domain: DomainState) -> PersistedStateV1 {
    PersistedStateV1 {
        save_generation: 7,
        saved_at: Timestamp(500_000),
        domain,
    }
}

fn ready(kind: SessionKind) -> DomainState {
    let state = DomainState::new(TimerConfig::new(2, 1, 3, 2).unwrap()).unwrap();
    let mut snapshot = state.snapshot().clone();
    snapshot.state = ProgressState::Ready {
        next_kind: kind,
        current_task_draft: if kind.is_work() {
            CurrentTask::parse("仕様を読む").unwrap()
        } else {
            None
        },
    };
    // Progress is authoritative, not reconstructed from a count of sessions.
    snapshot.round_progress.completed_focuses_in_round = 1;
    DomainState::from_parts(
        snapshot,
        state.history().clone(),
        state.id_allocators().clone(),
    )
    .unwrap()
}

fn apply(state: &mut DomainState, command: Command, at: u64) {
    state.apply(command, Timestamp(at)).unwrap();
    state.validate().unwrap();
}

fn advance(state: &mut DomainState, from: u64, duration: u64) {
    let mut at = from;
    while at < from + duration {
        let next = (at + 1_000).min(from + duration);
        state
            .observe(Observation {
                previous_at: Timestamp(at),
                at: Timestamp(next),
                monotonic_elapsed_ms: Some(next - at),
            })
            .unwrap();
        at = next;
    }
}

fn wire(state: &DomainState) -> Value {
    serde_json::from_slice(&candidate(state.clone()).encode().unwrap()).unwrap()
}

fn decode(value: &Value) -> Result<PersistedStateV1, CodecError> {
    PersistedStateV1::decode(&serde_json::to_vec(value).unwrap())
}

/// Valid transition products, covering all snapshot and payload variants.
fn corpus() -> Vec<DomainState> {
    let mut cases = vec![DomainState::new(TimerConfig::default()).unwrap()];
    for kind in [
        SessionKind::Focus,
        SessionKind::QuickStart,
        SessionKind::ShortBreak,
        SessionKind::LongBreak,
    ] {
        let mut state = ready(kind);
        cases.push(state.clone());
        apply(&mut state, Command::Start(kind), 0);
        advance(&mut state, 0, 500);
        cases.push(state.clone()); // includes an unclosed, partially credited run
        for outcome in [
            SessionOutcome::Cancelled,
            SessionOutcome::Reset,
            SessionOutcome::Skipped,
        ] {
            let mut ended = state.clone();
            apply(
                &mut ended,
                Command::End {
                    session_id: SessionId(1),
                    outcome,
                },
                500,
            );
            cases.push(ended);
            let mut interrupted = state.clone();
            apply(&mut interrupted, Command::Pause(SessionId(1)), 500);
            cases.push(interrupted.clone());
            apply(
                &mut interrupted,
                Command::End {
                    session_id: SessionId(1),
                    outcome,
                },
                600,
            );
            cases.push(interrupted);
        }
        let mut lifecycle = state.clone();
        apply(&mut lifecycle, Command::CloseApp, 500);
        cases.push(lifecycle.clone());
        apply(&mut lifecycle, Command::RestoreApp, 1_000);
        cases.push(lifecycle.clone());
        apply(&mut lifecycle, Command::Resume(SessionId(1)), 1_000);
        cases.push(lifecycle);

        let mut restart = state.clone();
        apply(&mut restart, Command::RestoreApp, 5_000);
        cases.push(restart);

        if kind.is_work() {
            let mut distracted = state.clone();
            apply(&mut distracted, Command::Distraction(SessionId(1)), 500);
            cases.push(distracted.clone());
            apply(&mut distracted, Command::CloseApp, 500);
            apply(&mut distracted, Command::RestoreApp, 10_000);
            apply(&mut distracted, Command::Return(SessionId(1)), 11_000);
            cases.push(distracted);
        }
        let ProgressState::Active { session, .. } = &state.snapshot().state else {
            panic!("expected active session");
        };
        let duration = session.planned_duration_ms;
        advance(&mut state, 500, duration - 500);
        cases.push(state.clone());
        if kind == SessionKind::QuickStart {
            apply(&mut state, Command::CloseApp, duration);
            apply(&mut state, Command::RestoreApp, duration + 1_000);
            cases.push(state.clone());
            for choice in [QuickStartChoice::Finish, QuickStartChoice::Continue] {
                let mut decided = state.clone();
                apply(
                    &mut decided,
                    Command::DecideQuickStart {
                        session_id: SessionId(1),
                        choice,
                    },
                    duration + 1_000,
                );
                cases.push(decided);
            }
        }
    }
    cases.extend(gap_cases());
    cases
}

fn gap_cases() -> Vec<DomainState> {
    let mut cases = vec![];
    // Clock rollback, forward discontinuity, missing evidence, and ordinary gap.
    for (at, elapsed) in [
        (400, Some(100)),
        (3_000, Some(100)),
        (600, None),
        (8_000, Some(7_500)),
    ] {
        let mut state = ready(SessionKind::Focus);
        apply(&mut state, Command::Start(SessionKind::Focus), 0);
        advance(&mut state, 0, 500);
        state
            .observe(Observation {
                previous_at: Timestamp(500),
                at: Timestamp(at),
                monotonic_elapsed_ms: elapsed,
            })
            .unwrap();
        cases.push(state.clone());
        apply(&mut state, Command::Resume(SessionId(1)), at + 100);
        cases.push(state);
    }
    cases
}

#[test]
fn round_trips_all_snapshots_sessions_interruptions_and_events_without_replay() {
    for domain in corpus() {
        let original = candidate(domain);
        let bytes = original.encode().unwrap();
        let loaded = PersistedStateV1::decode(&bytes).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.encode().unwrap(), bytes);
    }
}

#[test]
fn initial_json_matches_the_documented_contract() {
    let original = PersistedStateV1 {
        save_generation: 1,
        saved_at: Timestamp(0),
        domain: DomainState::new(TimerConfig::default()).unwrap(),
    };
    let expected = json!({
        "schema_version": 1,
        "save_generation": 1,
        "saved_at": "1970-01-01T00:00:00.000Z",
        "id_allocators": {"next_session_id": 1, "next_interruption_id": 1, "next_event_sequence": 1},
        "snapshot": {
            "settings": {"focus_seconds": 1500, "short_break_seconds": 300, "long_break_seconds": 900, "focuses_before_long_break": 4},
            "round_progress": {"completed_focuses_in_round": 0},
            "state": {"status": "ready", "next_kind": "focus", "current_task_draft": null}
        },
        "history": {"sessions": [], "events": []}
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&original.encode().unwrap()).unwrap(),
        expected
    );
    assert_eq!(decode(&expected).unwrap(), original);
}

#[test]
fn rejects_unversioned_unsupported_duplicate_and_malformed_envelopes() {
    assert!(matches!(
        PersistedStateV1::decode(br#"{"timer":{},"history":{}}"#),
        Err(CodecError::MissingVersion)
    ));
    for version in [0, 2, u32::MAX] {
        assert!(
            matches!(decode(&json!({"schema_version": version, "future": {"data": true}})), Err(CodecError::UnsupportedVersion(v)) if v == version)
        );
    }
    for bytes in [
        "null",
        "[]",
        "[1]",
        "{",
        "{}{}",
        r#"{"schema_version":null}"#,
        r#"{"schema_version":"1"}"#,
        r#"{"schema_version":1.0}"#,
        r#"{"schema_version":-1}"#,
        r#"{"schema_version":4294967296}"#,
        r#"{"schema_version":1,"schema_version":2}"#,
        r#"{"schema_version":2,"schema_version":1}"#,
        r#"{"schema_version":1,"schema_versi\u006fn":1}"#,
    ] {
        assert!(
            matches!(
                PersistedStateV1::decode(bytes.as_bytes()),
                Err(CodecError::Json(_))
            ),
            "{bytes}"
        );
    }
}

fn object_paths(value: &Value, path: &str, result: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            result.push(path.to_owned());
            for (key, value) in fields {
                object_paths(value, &format!("{path}/{key}"), result);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                object_paths(value, &format!("{path}/{index}"), result);
            }
        }
        _ => {}
    }
}

// Inject raw JSON at an exact path, so duplicate keys reach the decoder without
// passing through Value's key deduplication. Only test fixture mutation uses Value.
fn with_raw_object(value: &Value, path: &str, raw: &str) -> Vec<u8> {
    let mut root = value.clone();
    *root.pointer_mut(path).unwrap() = json!("__raw_object__");
    serde_json::to_string(&root)
        .unwrap()
        .replacen("\"__raw_object__\"", raw, 1)
        .into_bytes()
}

#[test]
fn every_object_rejects_unknown_missing_duplicate_fields_and_arrays() {
    for domain in corpus() {
        let value = wire(&domain);
        let mut paths = vec![];
        object_paths(&value, "", &mut paths);
        for path in paths {
            let fields = value.pointer(&path).unwrap().as_object().unwrap();
            let mut unknown = fields.clone();
            unknown.insert("unexpected".into(), json!(true));
            let raw = serde_json::to_string(&unknown).unwrap();
            assert!(
                PersistedStateV1::decode(&with_raw_object(&value, &path, &raw)).is_err(),
                "unknown at {path}: {raw}"
            );
            let array: Vec<_> = fields.values().collect();
            let raw = serde_json::to_string(&array).unwrap();
            assert!(
                PersistedStateV1::decode(&with_raw_object(&value, &path, &raw)).is_err(),
                "array at {path}"
            );
            for (key, item) in fields {
                let mut missing = fields.clone();
                missing.remove(key);
                let raw = serde_json::to_string(&missing).unwrap();
                assert!(
                    PersistedStateV1::decode(&with_raw_object(&value, &path, &raw)).is_err(),
                    "missing {path}/{key}"
                );
                let raw = format!(
                    "{{{}:{},{}",
                    serde_json::to_string(key).unwrap(),
                    item,
                    &serde_json::to_string(fields).unwrap()[1..]
                );
                assert!(
                    PersistedStateV1::decode(&with_raw_object(&value, &path, &raw)).is_err(),
                    "duplicate {path}/{key}: {raw}"
                );
            }
        }
    }
}

#[test]
fn rejects_unknown_tagged_variants() {
    for domain in corpus() {
        let value = wire(&domain);
        let mut paths = vec![];
        object_paths(&value, "", &mut paths);
        for path in paths {
            for tag in ["status", "type"] {
                if value.pointer(&path).unwrap().get(tag).is_some() {
                    let mut changed = value.clone();
                    changed.pointer_mut(&path).unwrap()[tag] = json!("future_variant");
                    assert!(
                        matches!(decode(&changed), Err(CodecError::Json(_))),
                        "{path}/{tag}"
                    );
                }
            }
        }
    }
}

fn rejects_mutation(value: &Value, path: &str, replacement: Value) {
    let mut changed = value.clone();
    *changed.pointer_mut(path).unwrap() = replacement;
    assert!(decode(&changed).is_err(), "accepted {path}: {changed}");
}

#[test]
fn validates_references_allocation_accounting_and_active_settings_after_decode() {
    let mut state = ready(SessionKind::Focus);
    apply(&mut state, Command::Start(SessionKind::Focus), 0);
    advance(&mut state, 0, 500);
    apply(&mut state, Command::Pause(SessionId(1)), 500);
    let value = wire(&state);
    for (path, replacement) in [
        ("/snapshot/state/session/planned_duration_ms", json!(3_000)),
        ("/snapshot/state/session/elapsed_ms", json!(501)),
        (
            "/snapshot/state/session/end",
            json!({"ended_at":"1970-01-01T00:00:00.500Z", "outcome":"cancelled"}),
        ),
        ("/snapshot/state/timer/interruption/id", json!(2)),
        (
            "/snapshot/state/timer/interruption/started_at",
            json!("1970-01-01T00:00:00.499Z"),
        ),
        ("/id_allocators/next_session_id", json!(1)),
        ("/id_allocators/next_interruption_id", json!(1)),
        ("/id_allocators/next_event_sequence", json!(10)),
        ("/history/events/0/session_id", json!(99)),
        ("/history/events/0/sequence", json!(2)),
        ("/history/events/0/payload/credited_ms", json!(501)),
        (
            "/snapshot/round_progress/completed_focuses_in_round",
            json!(3),
        ),
    ] {
        rejects_mutation(&value, path, replacement);
    }
    let mut duplicate = value.clone();
    duplicate["history"]["sessions"] = json!([duplicate["snapshot"]["state"]["session"].clone()]);
    duplicate["history"]["sessions"][0]["end"] =
        json!({"ended_at":"1970-01-01T00:00:00.500Z", "outcome":"cancelled"});
    assert!(matches!(
        decode(&duplicate),
        Err(CodecError::InvalidDomain(_))
    ));

    apply(
        &mut state,
        Command::End {
            session_id: SessionId(1),
            outcome: SessionOutcome::Cancelled,
        },
        600,
    );
    apply(
        &mut state,
        Command::Configure(TimerConfig::new(5, 4, 6, 2).unwrap()),
        600,
    );
    // Old settings remain valid for ended sessions.
    assert_eq!(decode(&wire(&state)).unwrap().domain, state);
}

#[test]
fn validates_quick_start_links_task_and_decision_consistency() {
    let mut state = ready(SessionKind::QuickStart);
    apply(&mut state, Command::Start(SessionKind::QuickStart), 0);
    advance(&mut state, 0, 120_000);
    let awaiting = wire(&state);
    rejects_mutation(
        &awaiting,
        "/snapshot/state/quick_start_session_id",
        json!(2),
    );
    rejects_mutation(
        &awaiting,
        "/snapshot/state/current_task",
        json!("different"),
    );
    apply(
        &mut state,
        Command::DecideQuickStart {
            session_id: SessionId(1),
            choice: QuickStartChoice::Continue,
        },
        120_000,
    );
    let continued = wire(&state);
    for (path, replacement) in [
        (
            "/snapshot/state/session/continued_from_quick_start",
            Value::Null,
        ),
        ("/snapshot/state/session/current_task", Value::Null),
        (
            "/history/events/1/payload/decision/focus_session_id",
            json!(3),
        ),
        ("/history/sessions/0/planned_duration_ms", json!(119_000)),
    ] {
        rejects_mutation(&continued, path, replacement);
    }
}

#[test]
fn preserves_clock_evidence_and_rejects_forged_known_recovery() {
    let mut state = ready(SessionKind::Focus);
    apply(&mut state, Command::Start(SessionKind::Focus), 1_000);
    apply(&mut state, Command::Distraction(SessionId(1)), 1_000);
    apply(&mut state, Command::CloseApp, 2_000);
    apply(&mut state, Command::RestoreApp, 1_500);
    let mut open = wire(&state);
    open["snapshot"]["state"]["timer"]["interruption"]["time_uncertainty"] = Value::Null;
    assert!(matches!(decode(&open), Err(CodecError::InvalidDomain(_))));
    apply(&mut state, Command::Return(SessionId(1)), 3_000);
    let mut closed = wire(&state);
    let last = closed["history"]["events"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    last["payload"]["end"]["duration"] = json!({"status":"known", "elapsed_ms":2_000});
    assert!(matches!(decode(&closed), Err(CodecError::InvalidDomain(_))));
}

#[test]
fn validates_metadata_and_rejects_unrepresentable_domain_times_on_write() {
    let mut value = candidate(ready(SessionKind::Focus));
    value.save_generation = 0;
    assert!(matches!(value.encode(), Err(CodecError::InvalidValue(_))));
    value.save_generation = u64::MAX;
    assert_eq!(
        PersistedStateV1::decode(&value.encode().unwrap()).unwrap(),
        value
    );
    // saved_at is metadata, not an ordering constraint against session times.
    value.saved_at = Timestamp(0);
    assert!(value.encode().is_ok());
    value.saved_at = Timestamp(u64::MAX);
    assert!(matches!(value.encode(), Err(CodecError::InvalidValue(_))));
    let mut domain = ready(SessionKind::Focus);
    apply(
        &mut domain,
        Command::Start(SessionKind::Focus),
        u64::MAX - 1,
    );
    assert!(candidate(domain.clone()).encode().is_err());
    apply(&mut domain, Command::Pause(SessionId(1)), u64::MAX - 1);
    assert!(candidate(domain.clone()).encode().is_err());
    apply(
        &mut domain,
        Command::End {
            session_id: SessionId(1),
            outcome: SessionOutcome::Reset,
        },
        u64::MAX,
    );
    assert!(candidate(domain).encode().is_err());

    let value = wire(&ready(SessionKind::Focus));
    for (path, bad) in [
        ("/save_generation", json!(0)),
        ("/save_generation", json!(-1)),
        ("/save_generation", json!(1.5)),
        ("/id_allocators/next_session_id", json!(0)),
        ("/saved_at", json!("1970-01-01T00:00:00Z")),
    ] {
        rejects_mutation(&value, path, bad);
    }
}
