//! V1 objects, separate from core values. All objects require named fields,
//! including nullable ones, and reject unknown fields and duplicate keys.

// Field names are the documented V1 wire contract, including next_* counters.
#![allow(clippy::struct_field_names)]

use serde::{Deserialize, Deserializer, Serialize};

use super::values::{
    CurrentTaskV1, GapReasonV1, InterruptionIdV1, InterruptionKindV1, SessionIdV1, SessionKindV1,
    SessionOutcomeV1, SettingsV1, TimeUncertaintyV1, TimestampV1, deserialize_object,
    required_option,
};

// Serde's derived struct and internally tagged enum readers also accept arrays.
// These small wrappers require a map and pass its original entries to Serde,
// retaining duplicate-key evidence (including inside tagged enum content).
macro_rules! object {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Serialize)]
        pub(super) struct $name { $(pub $field: $ty),* }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fields { $($(#[$attr])* $field: $ty),* }
                let fields: Fields = deserialize_object(deserializer)?;
                Ok(Self { $($field: fields.$field),* })
            }
        }
    };
}

macro_rules! tagged {
    ($name:ident, $tag:literal, {
        $($variant:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }),* $(,)?
    }) => {
        #[derive(Serialize)]
        #[serde(tag = $tag, rename_all = "snake_case")]
        pub(super) enum $name { $($variant { $($field: $ty),* }),* }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(tag = $tag, rename_all = "snake_case", deny_unknown_fields)]
                enum Fields { $($variant { $($(#[$attr])* $field: $ty),* }),* }
                let fields: Fields = deserialize_object(deserializer)?;
                Ok(match fields {
                    $(Fields::$variant { $($field),* } => Self::$variant { $($field),* }),*
                })
            }
        }
    };
}

object!(Envelope {
    schema_version: u32,
    save_generation: u64,
    saved_at: TimestampV1,
    id_allocators: IdAllocatorsV1,
    snapshot: SnapshotV1,
    history: HistoryV1,
});

object!(IdAllocatorsV1 {
    next_session_id: u64,
    next_interruption_id: u64,
    next_event_sequence: u64,
});

object!(SnapshotV1 {
    settings: SettingsV1,
    round_progress: RoundProgressV1,
    state: ProgressV1,
});

object!(RoundProgressV1 {
    completed_focuses_in_round: u32
});

tagged!(ProgressV1, "status", {
    Ready {
        next_kind: SessionKindV1,
        #[serde(deserialize_with = "required_option")]
        current_task_draft: Option<CurrentTaskV1>,
    },
    Active { session: SessionV1, timer: TimerV1 },
    AwaitingQuickStartDecision {
        quick_start_session_id: SessionIdV1,
        #[serde(deserialize_with = "required_option")]
        current_task: Option<CurrentTaskV1>,
    },
});

object!(SessionV1 {
    id: SessionIdV1,
    kind: SessionKindV1,
    #[serde(deserialize_with = "required_option")]
    current_task: Option<CurrentTaskV1>,
    planned_duration_ms: u64,
    started_at: TimestampV1,
    elapsed_ms: u64,
    #[serde(deserialize_with = "required_option")]
    continued_from_quick_start: Option<SessionIdV1>,
    #[serde(deserialize_with = "required_option")]
    end: Option<SessionEndV1>,
});

object!(SessionEndV1 {
    ended_at: TimestampV1,
    outcome: SessionOutcomeV1
});

tagged!(TimerV1, "status", {
    Running { run: RunV1 },
    Interrupted { interruption: InterruptionV1 },
});

object!(RunV1 {
    started_at: TimestampV1,
    elapsed_ms_at_start: u64,
    last_confirmed_at: TimestampV1,
    #[serde(deserialize_with = "required_option")]
    time_uncertainty: Option<TimeUncertaintyV1>,
});

object!(InterruptionV1 {
    id: InterruptionIdV1,
    kind: InterruptionKindV1,
    started_at: TimestampV1,
    recorded_at: TimestampV1,
    #[serde(deserialize_with = "required_option")]
    time_uncertainty: Option<TimeUncertaintyV1>,
    #[serde(deserialize_with = "required_option")]
    end: Option<InterruptionEndV1>,
});

object!(InterruptionEndV1 {
    ended_at: TimestampV1,
    outcome: InterruptionOutcomeV1,
    duration: MeasuredDurationV1,
});

tagged!(InterruptionOutcomeV1, "type", {
    Resumed {},
    Returned {},
    SessionEnded { session_outcome: SessionOutcomeV1 },
});

tagged!(MeasuredDurationV1, "status", {
    Known { elapsed_ms: u64 },
    Unknown { reason: TimeUncertaintyV1 },
});

object!(HistoryV1 { sessions: Vec<SessionV1>, events: Vec<EventV1> });

object!(EventV1 {
    sequence: u64,
    session_id: SessionIdV1,
    effective_at: TimestampV1,
    recorded_at: TimestampV1,
    payload: EventKindV1,
});

tagged!(EventKindV1, "type", {
    RunIntervalRecorded {
        started_at: TimestampV1,
        ended_at: TimestampV1,
        credited_ms: u64,
        #[serde(deserialize_with = "required_option")]
        time_uncertainty: Option<TimeUncertaintyV1>,
    },
    InterruptionStarted {
        interruption_id: InterruptionIdV1,
        interruption_kind: InterruptionKindV1,
    },
    InterruptionEnded { interruption_id: InterruptionIdV1, end: InterruptionEndV1 },
    QuickStartDecisionMade { decision: QuickStartDecisionV1 },
    AppClosing {},
    AppRestored {},
    ObservationGapDetected {
        last_confirmed_at: TimestampV1,
        detected_at: TimestampV1,
        reason: GapReasonV1,
    },
});

tagged!(QuickStartDecisionV1, "type", {
    Finish {},
    Continue { focus_session_id: SessionIdV1 },
});
