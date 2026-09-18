use serde::de::DeserializeOwned;

use super::*;

fn assert_json<T: Serialize + DeserializeOwned + fmt::Debug + PartialEq>(value: &T, json: &str) {
    assert_eq!(serde_json::to_string(value).unwrap(), json);
    assert_eq!(&serde_json::from_str::<T>(json).unwrap(), value);
}

fn reject_json<T: DeserializeOwned>(inputs: &[&str]) {
    for json in inputs {
        assert!(serde_json::from_str::<T>(json).is_err(), "accepted {json}");
    }
}

#[test]
fn timestamps_round_trip_at_millisecond_and_calendar_boundaries() {
    for (millis, text) in [
        (0, "1970-01-01T00:00:00.000Z"),
        (1, "1970-01-01T00:00:00.001Z"),
        (999, "1970-01-01T00:00:00.999Z"),
        (1_000, "1970-01-01T00:00:01.000Z"),
        (86_400_000, "1970-01-02T00:00:00.000Z"),
        (951_782_400_123, "2000-02-29T00:00:00.123Z"),
        (253_402_300_799_999, "9999-12-31T23:59:59.999Z"),
    ] {
        let wire = TimestampV1::try_from(Timestamp(millis)).unwrap();
        assert_json(&wire, &serde_json::to_string(text).unwrap());
        assert_eq!(Timestamp::from(wire), Timestamp(millis));
    }
}

#[test]
fn timestamps_reject_noncanonical_precision_calendar_and_offsets() {
    for text in [
        "1969-12-31T23:59:59.999Z",
        "10000-01-01T00:00:00.000Z",
        "+10000-01-01T00:00:00.000Z",
        "1970-01-01T00:00:00Z",
        "1970-01-01T00:00:00.0Z",
        "1970-01-01T00:00:00.00Z",
        "1970-01-01T00:00:00.0000Z",
        "1970-01-01T00:00:00.0001Z",
        "1970-01-01T00:00:00.123456789Z",
        "1970-01-01T00:00:00.000+00:00",
        "1970-01-01T00:00:00.000-00:00",
        "1970-01-01T09:00:00.000+09:00",
        "1970-01-01t00:00:00.000Z",
        "1970-01-01T00:00:00.000z",
        "1970-01-01 00:00:00.000Z",
        "1970-01-01T00:00:00,000Z",
        "2016-12-31T23:59:60.000Z",
        "2016-12-31T23:59:60.999Z",
        "2026-02-29T00:00:00.000Z",
        "2100-02-29T00:00:00.000Z",
        "2024-02-30T00:00:00.000Z",
        "2024-13-01T00:00:00.000Z",
        "2024-01-01T24:00:00.000Z",
        "1970-01-01T00:00:00.000Z ",
        " 1970-01-01T00:00:00.000Z",
        "",
    ] {
        reject_json::<TimestampV1>(&[&serde_json::to_string(text).unwrap()]);
    }
    reject_json::<TimestampV1>(&["0", "1.0", "null", "true", "[]", "{}"]);
}

#[test]
fn timestamps_reject_unrepresentable_domain_values_on_write() {
    for millis in [253_402_300_800_000, u64::MAX] {
        assert!(TimestampV1::try_from(Timestamp(millis)).is_err());
        // Even accidental construction inside this module cannot encode an invalid value.
        assert!(serde_json::to_string(&TimestampV1(Timestamp(millis))).is_err());
    }
}

#[test]
fn ids_preserve_full_u64_precision_and_domain_types() {
    for id in [1, 9_007_199_254_740_993, u64::MAX] {
        let session = SessionIdV1::try_from(SessionId(id)).unwrap();
        let interruption = InterruptionIdV1::try_from(InterruptionId(id)).unwrap();
        assert_json(&session, &id.to_string());
        assert_json(&interruption, &id.to_string());
        assert_eq!(SessionId::from(session), SessionId(id));
        assert_eq!(InterruptionId::from(interruption), InterruptionId(id));
    }
    assert!(SessionIdV1::try_from(SessionId(0)).is_err());
    assert!(InterruptionIdV1::try_from(InterruptionId(0)).is_err());
}

#[test]
fn ids_reject_zero_overflow_and_noninteger_json() {
    let invalid = [
        "0",
        "-0",
        "-1",
        "1.0",
        "1e0",
        "18446744073709551616",
        "\"1\"",
        "true",
        "null",
        "[]",
        "{}",
    ];
    reject_json::<SessionIdV1>(&invalid);
    reject_json::<InterruptionIdV1>(&invalid);
}

const DEFAULT_SETTINGS: &str = r#"{"focus_seconds":1500,"short_break_seconds":300,"long_break_seconds":900,"focuses_before_long_break":4}"#;

#[test]
fn settings_round_trip_defaults_and_valid_extremes() {
    let settings = SettingsV1::try_from(TimerConfig::default()).unwrap();
    assert_json(&settings, DEFAULT_SETTINGS);
    for config in [
        TimerConfig::new(1, 1, 1, 1).unwrap(),
        TimerConfig::new(86_400, 86_400, 86_400, u32::MAX).unwrap(),
    ] {
        let wire = SettingsV1::try_from(config).unwrap();
        let json = serde_json::to_string(&wire).unwrap();
        let loaded: SettingsV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(TimerConfig::from(loaded), config);
    }
}

#[test]
fn settings_reject_missing_unknown_duplicate_and_nonobject_fields() {
    reject_json::<SettingsV1>(&[
        "null",
        "[]",
        "[1500,300,900,4]",
        "{}",
        r#"{"focus_seconds":1500,"focus_seconds":1500,"short_break_seconds":300,"long_break_seconds":900,"focuses_before_long_break":4}"#,
        r#"{"focus_seconds":1500,"\u0066ocus_seconds":1500,"short_break_seconds":300,"long_break_seconds":900,"focuses_before_long_break":4}"#,
    ]);
    let defaults: serde_json::Value = serde_json::from_str(DEFAULT_SETTINGS).unwrap();
    for field in [
        "focus_seconds",
        "short_break_seconds",
        "long_break_seconds",
        "focuses_before_long_break",
    ] {
        let mut missing = defaults.clone();
        missing.as_object_mut().unwrap().remove(field);
        reject_json::<SettingsV1>(&[&missing.to_string()]);
        let mut null = defaults.clone();
        null[field] = serde_json::Value::Null;
        reject_json::<SettingsV1>(&[&null.to_string()]);
    }
    let mut extra = defaults;
    extra["future_setting"] = serde_json::Value::Null;
    reject_json::<SettingsV1>(&[&extra.to_string()]);
}

#[test]
fn settings_validate_ranges_on_read_and_domain_conversion() {
    let defaults: serde_json::Value = serde_json::from_str(DEFAULT_SETTINGS).unwrap();
    for field in ["focus_seconds", "short_break_seconds", "long_break_seconds"] {
        for invalid in [0, 86_401, u64::MAX] {
            let mut value = defaults.clone();
            value[field] = invalid.into();
            let json = value.to_string();
            reject_json::<SettingsV1>(&[&json]);
            // Legacy TimerConfig deserialization can bypass its constructor.
            let config: TimerConfig = serde_json::from_str(&json).unwrap();
            assert!(SettingsV1::try_from(config).is_err());
            assert!(serde_json::to_string(&SettingsV1(config)).is_err());
        }
    }
    for invalid in [0, u64::from(u32::MAX) + 1] {
        let mut value = defaults.clone();
        value["focuses_before_long_break"] = invalid.into();
        reject_json::<SettingsV1>(&[&value.to_string()]);
    }
    for token in ["-1", "1.0", "1e0", "\"1\"", "true"] {
        reject_json::<SettingsV1>(&[&DEFAULT_SETTINGS.replace("1500", token)]);
    }
}

#[test]
fn current_tasks_preserve_canonical_text_without_normalization() {
    for text in [
        "今回やること",
        "read chapter 1",
        "read\tchapter",
        "cafe\u{0301} 📖",
        "path \\\"quoted\\\"",
    ] {
        let task = CurrentTask::parse(text).unwrap().unwrap();
        let wire = CurrentTaskV1::from(task.clone());
        assert_json(&wire, &serde_json::to_string(text).unwrap());
        assert_eq!(CurrentTask::from(wire), task);
    }
}

#[test]
fn current_tasks_reject_values_that_would_be_normalized_or_multiline() {
    for text in [
        "",
        " ",
        "\t",
        "\n",
        " task",
        "task ",
        "\u{3000}task",
        "task\u{00a0}",
    ] {
        reject_json::<CurrentTaskV1>(&[&serde_json::to_string(text).unwrap()]);
    }
    for separator in [
        '\r', '\n', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
    ] {
        reject_json::<CurrentTaskV1>(&[
            &serde_json::to_string(&format!("first{separator}second")).unwrap()
        ]);
    }
    reject_json::<CurrentTaskV1>(&["null", "0", "false", "[]", "{}"]);
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NullableValues {
    #[serde(deserialize_with = "required_option")]
    task: Option<CurrentTaskV1>,
    #[serde(deserialize_with = "required_option")]
    source: Option<SessionIdV1>,
    #[serde(deserialize_with = "required_option")]
    uncertainty: Option<TimeUncertaintyV1>,
}

#[test]
fn nullable_fields_require_keys_and_preserve_explicit_null() {
    let empty = NullableValues {
        task: None,
        source: None,
        uncertainty: None,
    };
    assert_json(&empty, r#"{"task":null,"source":null,"uncertainty":null}"#);
    let value = NullableValues {
        task: Some(CurrentTask::parse("read").unwrap().unwrap().into()),
        source: Some(SessionIdV1::try_from(SessionId(1)).unwrap()),
        uncertainty: Some(TimeUncertainty::ClockDiscontinuity.into()),
    };
    assert_json(
        &value,
        r#"{"task":"read","source":1,"uncertainty":"clock_discontinuity"}"#,
    );
    reject_json::<NullableValues>(&[
        "{}",
        r#"{"source":null,"uncertainty":null}"#,
        r#"{"task":null,"uncertainty":null}"#,
        r#"{"task":null,"source":null}"#,
        r#"{"task":null,"task":null,"source":null,"uncertainty":null}"#,
        r#"{"task":" ","source":null,"uncertainty":null}"#,
        r#"{"task":null,"source":0,"uncertainty":null}"#,
        r#"{"task":null,"source":null,"uncertainty":"unknown"}"#,
    ]);
}

fn check_enum<D, W>(cases: &[(D, &str)])
where
    D: Copy + fmt::Debug + PartialEq + From<W>,
    W: Serialize + DeserializeOwned + fmt::Debug + PartialEq + From<D>,
{
    for &(domain, text) in cases {
        let wire = W::from(domain);
        assert_json(&wire, &serde_json::to_string(text).unwrap());
        assert_eq!(D::from(wire), domain);
        reject_json::<W>(&[&format!("{{\"{text}\":null}}")]);
        reject_json::<W>(&[&serde_json::to_string(&text.to_uppercase()).unwrap()]);
    }
    reject_json::<W>(&[
        "null",
        "0",
        "false",
        "[]",
        "{}",
        "\"future_variant\"",
        "\"\"",
    ]);
}

#[test]
fn scalar_enums_use_only_the_documented_string_variants() {
    check_enum::<SessionKind, SessionKindV1>(&[
        (SessionKind::Focus, "focus"),
        (SessionKind::QuickStart, "quick_start"),
        (SessionKind::ShortBreak, "short_break"),
        (SessionKind::LongBreak, "long_break"),
    ]);
    check_enum::<SessionOutcome, SessionOutcomeV1>(&[
        (SessionOutcome::Completed, "completed"),
        (SessionOutcome::Cancelled, "cancelled"),
        (SessionOutcome::Reset, "reset"),
        (SessionOutcome::Skipped, "skipped"),
    ]);
    check_enum::<InterruptionKind, InterruptionKindV1>(&[
        (InterruptionKind::Pause, "pause"),
        (InterruptionKind::Distraction, "distraction"),
        (InterruptionKind::AppExit, "app_exit"),
        (InterruptionKind::ObservationGap, "observation_gap"),
    ]);
    check_enum::<TimeUncertainty, TimeUncertaintyV1>(&[
        (TimeUncertainty::ClockMovedBackward, "clock_moved_backward"),
        (TimeUncertainty::ClockDiscontinuity, "clock_discontinuity"),
        (
            TimeUncertainty::InsufficientClockEvidence,
            "insufficient_clock_evidence",
        ),
    ]);
    check_enum::<GapReason, GapReasonV1>(&[
        (GapReason::Restart, "restart"),
        (
            GapReason::ObservationDiscontinuity,
            "observation_discontinuity",
        ),
        (GapReason::ClockAnomaly, "clock_anomaly"),
    ]);
}
