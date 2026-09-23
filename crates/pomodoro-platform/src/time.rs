use std::{
    error::Error,
    fmt,
    time::{Duration, Instant, SystemTime},
};

use pomodoro_core::{Observation, Timestamp};

/// Runtime-only clock evidence. Construct a fresh clock on application startup;
/// never reconstruct its monotonic anchor from persisted UTC timestamps.
/// Gap/completion policy and domain transitions belong to `pomodoro-core`.
#[derive(Debug)]
pub struct ObservationClock {
    previous_at: Timestamp,
    previous_monotonic: Option<Instant>,
    fractional_elapsed: Duration,
}

impl ObservationClock {
    /// Samples the clocks to establish a fresh runtime boundary. Use `at()` for
    /// the initial domain operation (such as `RestoreApp`); subsequent samples
    /// can only measure time since this process's new anchor.
    ///
    /// # Errors
    /// Returns [`TimeError`] if the UTC clock cannot be represented.
    pub fn new() -> Result<Self, TimeError> {
        Self::from_reading(read_clocks())
    }

    /// UTC time of the last successful sample, for commands at that same boundary.
    /// It does not imply that the application has applied or saved an observation.
    #[must_use]
    pub fn at(&self) -> Timestamp {
        self.previous_at
    }

    /// Drops continuity evidence while retaining the last UTC boundary. Call
    /// when observations cannot be applied, including on a failed save. Resume
    /// sampling only when the application can pass the next observation to core;
    /// that first sample has no elapsed-time evidence, even for a short gap.
    pub fn break_continuity(&mut self) {
        self.previous_monotonic = None;
        self.fractional_elapsed = Duration::ZERO;
    }

    /// Samples UTC and monotonic time without applying domain or storage policy.
    /// A lost/backward monotonic anchor yields `None`, never a wall-clock delta.
    /// The successful sample becomes the anchor for the following observation.
    ///
    /// # Errors
    /// Returns [`TimeError`] for an unrepresentable UTC reading. A failed reading
    /// breaks continuity; the next valid sample cannot credit the failed interval.
    pub fn observe(&mut self) -> Result<Observation, TimeError> {
        self.observe_reading(read_clocks())
    }

    fn from_reading((wall, monotonic): (SystemTime, Instant)) -> Result<Self, TimeError> {
        Ok(Self {
            previous_at: timestamp_at(wall)?,
            previous_monotonic: Some(monotonic),
            fractional_elapsed: Duration::ZERO,
        })
    }

    fn observe_reading(
        &mut self,
        (wall, monotonic): (SystemTime, Instant),
    ) -> Result<Observation, TimeError> {
        let at = match timestamp_at(wall) {
            Ok(at) => at,
            Err(error) => {
                self.break_continuity();
                return Err(error);
            }
        };
        let elapsed = self
            .previous_monotonic
            .and_then(|previous| monotonic.checked_duration_since(previous))
            .and_then(|duration| duration.checked_add(self.fractional_elapsed))
            .and_then(split_milliseconds);
        let observation = Observation {
            previous_at: self.previous_at,
            at,
            monotonic_elapsed_ms: elapsed.map(|(millis, _)| millis),
        };
        self.previous_at = at;
        self.previous_monotonic = Some(monotonic);
        self.fractional_elapsed = elapsed.map_or(Duration::ZERO, |(_, remainder)| remainder);
        Ok(observation)
    }
}

fn read_clocks() -> (SystemTime, Instant) {
    let monotonic = Instant::now();
    let wall = SystemTime::now();
    (wall, monotonic)
}

// Keep fractions across continuous samples rather than losing up to a millisecond
// on every tick. Overflow is missing evidence, not a saturated elapsed duration.
fn split_milliseconds(duration: Duration) -> Option<(u64, Duration)> {
    let millis = u64::try_from(duration.as_millis()).ok()?;
    Some((millis, duration.checked_sub(Duration::from_millis(millis))?))
}

fn timestamp_at(wall: SystemTime) -> Result<Timestamp, TimeError> {
    let millis = wall
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| TimeError::BeforeUnixEpoch)?
        .as_millis();
    u64::try_from(millis)
        .map(Timestamp)
        .map_err(|_| TimeError::OutOfRange)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    BeforeUnixEpoch,
    OutOfRange,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BeforeUnixEpoch => "system time is before the Unix epoch",
            Self::OutOfRange => "timestamp is outside the supported range",
        })
    }
}

impl Error for TimeError {}

#[cfg(test)]
#[path = "time_tests.rs"]
mod tests;
