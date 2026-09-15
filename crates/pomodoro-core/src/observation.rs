use crate::{GapReason, TimeUncertainty, Timestamp};

// Internal policy, not persisted settings. The TUI normally observes every 100ms.
// A gap over 5s is deliberately conservative; changing policy needs tests, not a
// schema change. A wall/monotonic discrepancy over 1s indicates clock uncertainty.
const MAX_OBSERVATION_GAP_MS: u64 = 5_000;
const MAX_CLOCK_DRIFT_MS: u64 = 1_000;

/// Runtime-only clock evidence supplied by the platform. Never serialize this
/// or reconstruct monotonic elapsed time from saved UTC timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub previous_at: Timestamp,
    pub at: Timestamp,
    pub monotonic_elapsed_ms: Option<u64>,
}

impl Observation {
    pub(crate) fn classify(self) -> Result<u64, (GapReason, Option<TimeUncertainty>)> {
        let Some(wall_delta) = self.at.0.checked_sub(self.previous_at.0) else {
            return Err((
                GapReason::ClockAnomaly,
                Some(TimeUncertainty::ClockMovedBackward),
            ));
        };
        let Some(elapsed) = self.monotonic_elapsed_ms else {
            return Err((
                GapReason::ObservationDiscontinuity,
                Some(TimeUncertainty::InsufficientClockEvidence),
            ));
        };
        if wall_delta.abs_diff(elapsed) > MAX_CLOCK_DRIFT_MS {
            return Err((
                GapReason::ClockAnomaly,
                Some(TimeUncertainty::ClockDiscontinuity),
            ));
        }
        if elapsed > MAX_OBSERVATION_GAP_MS || wall_delta > MAX_OBSERVATION_GAP_MS {
            return Err((GapReason::ObservationDiscontinuity, None));
        }
        Ok(elapsed)
    }
}
