//! Validated scalar representations for the V1 JSON contract.

use std::{fmt, marker::PhantomData, num::NonZeroU64};

use chrono::{DateTime, SecondsFormat, Utc};
use pomodoro_core::{
    ConfigError, CurrentTask, GapReason, InterruptionId, InterruptionKind, SessionId, SessionKind,
    SessionOutcome, TimeUncertainty, TimerConfig, Timestamp,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, MapAccess, Visitor, value::MapAccessDeserializer},
    ser::Error as _,
};

const MAX_TIMESTAMP_MS: u64 = 253_402_300_799_999;
const TIMESTAMP_RANGE_ERROR: &str = "V1 timestamp must be between Unix epoch and year 9999";
const TIMESTAMP_FORMAT_ERROR: &str =
    "V1 timestamp must use YYYY-MM-DDTHH:MM:SS.sssZ without leap seconds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TimestampV1(Timestamp);

fn utc_datetime(timestamp: Timestamp) -> Result<DateTime<Utc>, &'static str> {
    if timestamp.0 > MAX_TIMESTAMP_MS {
        return Err(TIMESTAMP_RANGE_ERROR);
    }
    let millis = i64::try_from(timestamp.0).map_err(|_| TIMESTAMP_RANGE_ERROR)?;
    DateTime::from_timestamp_millis(millis).ok_or(TIMESTAMP_RANGE_ERROR)
}

impl TryFrom<Timestamp> for TimestampV1 {
    type Error = &'static str;

    fn try_from(value: Timestamp) -> Result<Self, Self::Error> {
        utc_datetime(value)?;
        Ok(Self(value))
    }
}

impl From<TimestampV1> for Timestamp {
    fn from(value: TimestampV1) -> Self {
        value.0
    }
}

impl Serialize for TimestampV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let utc = utc_datetime(self.0).map_err(S::Error::custom)?;
        serializer.serialize_str(&utc.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

impl<'de> Deserialize<'de> for TimestampV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 24 || !text.ends_with('Z') {
            return Err(D::Error::custom(TIMESTAMP_FORMAT_ERROR));
        }
        let parsed = DateTime::parse_from_rfc3339(&text).map_err(D::Error::custom)?;
        // Chrono supports leap seconds; V1's integer Unix milliseconds do not.
        if parsed.timestamp_subsec_nanos() >= 1_000_000_000 {
            return Err(D::Error::custom(TIMESTAMP_FORMAT_ERROR));
        }
        let millis = u64::try_from(parsed.timestamp_millis())
            .map_err(|_| D::Error::custom(TIMESTAMP_RANGE_ERROR))?;
        let value = Self::try_from(Timestamp(millis)).map_err(D::Error::custom)?;
        let utc = utc_datetime(value.0).map_err(D::Error::custom)?;
        if utc.to_rfc3339_opts(SecondsFormat::Millis, true) != text {
            return Err(D::Error::custom(TIMESTAMP_FORMAT_ERROR));
        }
        Ok(value)
    }
}

macro_rules! id_value {
    ($wire:ident, $domain:ident, $message:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub(super) struct $wire(NonZeroU64);

        impl TryFrom<$domain> for $wire {
            type Error = &'static str;

            fn try_from(value: $domain) -> Result<Self, Self::Error> {
                NonZeroU64::new(value.0).map(Self).ok_or($message)
            }
        }

        impl From<$wire> for $domain {
            fn from(value: $wire) -> Self {
                Self(value.0.get())
            }
        }
    };
}

id_value!(
    SessionIdV1,
    SessionId,
    "session ID must be greater than zero"
);
id_value!(
    InterruptionIdV1,
    InterruptionId,
    "interruption ID must be greater than zero"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SettingsV1(TimerConfig);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFields {
    focus_seconds: u64,
    short_break_seconds: u64,
    long_break_seconds: u64,
    focuses_before_long_break: u32,
}

impl TryFrom<TimerConfig> for SettingsV1 {
    type Error = ConfigError;

    fn try_from(value: TimerConfig) -> Result<Self, Self::Error> {
        value.validate()?;
        Ok(Self(value))
    }
}

impl From<SettingsV1> for TimerConfig {
    fn from(value: SettingsV1) -> Self {
        value.0
    }
}

impl Serialize for SettingsV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.validate().map_err(S::Error::custom)?;
        SettingsFields {
            focus_seconds: self.0.focus_seconds(),
            short_break_seconds: self.0.short_break_seconds(),
            long_break_seconds: self.0.long_break_seconds(),
            focuses_before_long_break: self.0.focuses_before_long_break(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SettingsV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields: SettingsFields = deserialize_object(deserializer)?;
        TimerConfig::new(
            fields.focus_seconds,
            fields.short_break_seconds,
            fields.long_break_seconds,
            fields.focuses_before_long_break,
        )
        .map(Self)
        .map_err(D::Error::custom)
    }
}

// Derived struct deserializers also accept positional arrays. V1 requires JSON
// objects; retain the original MapAccess so duplicate keys remain observable.
pub(super) fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectVisitor<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a V1 JSON object")
        }

        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(ObjectVisitor(PhantomData))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CurrentTaskV1(CurrentTask);

impl From<CurrentTask> for CurrentTaskV1 {
    fn from(value: CurrentTask) -> Self {
        Self(value)
    }
}

impl From<CurrentTaskV1> for CurrentTask {
    fn from(value: CurrentTaskV1) -> Self {
        value.0
    }
}

impl Serialize for CurrentTaskV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for CurrentTaskV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let task = CurrentTask::parse(&text)
            .map_err(D::Error::custom)?
            .filter(|task| task.as_str() == text)
            .ok_or_else(|| D::Error::custom("current task must be nonempty and already trimmed"))?;
        Ok(Self(task))
    }
}

/// Use as `#[serde(deserialize_with = "required_option")]` on every nullable
/// DTO field. The attribute makes missing keys an error; explicit null is None.
pub(super) fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

// Scalar enums have only string representations. Derived externally tagged
// enum deserializers also accept objects such as {"focus": null}, which V1 rejects.
macro_rules! enum_value {
    ($wire:ident, $domain:ident, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) struct $wire($domain);

        impl From<$domain> for $wire {
            fn from(value: $domain) -> Self {
                Self(value)
            }
        }

        impl From<$wire> for $domain {
            fn from(value: $wire) -> Self {
                value.0
            }
        }

        impl Serialize for $wire {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(match self.0 {
                    $($domain::$variant => $text,)+
                })
            }
        }

        impl<'de> Deserialize<'de> for $wire {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                match text.as_str() {
                    $($text => Ok(Self($domain::$variant)),)+
                    _ => Err(D::Error::unknown_variant(&text, &[$($text),+])),
                }
            }
        }
    };
}

enum_value!(SessionKindV1, SessionKind, {
    Focus => "focus",
    QuickStart => "quick_start",
    ShortBreak => "short_break",
    LongBreak => "long_break",
});

enum_value!(SessionOutcomeV1, SessionOutcome, {
    Completed => "completed",
    Cancelled => "cancelled",
    Reset => "reset",
    Skipped => "skipped",
});

enum_value!(InterruptionKindV1, InterruptionKind, {
    Pause => "pause",
    Distraction => "distraction",
    AppExit => "app_exit",
    ObservationGap => "observation_gap",
});

enum_value!(TimeUncertaintyV1, TimeUncertainty, {
    ClockMovedBackward => "clock_moved_backward",
    ClockDiscontinuity => "clock_discontinuity",
    InsufficientClockEvidence => "insufficient_clock_evidence",
});

enum_value!(GapReasonV1, GapReason, {
    Restart => "restart",
    ObservationDiscontinuity => "observation_discontinuity",
    ClockAnomaly => "clock_anomaly",
});

#[cfg(test)]
#[path = "values_tests.rs"]
mod tests;
