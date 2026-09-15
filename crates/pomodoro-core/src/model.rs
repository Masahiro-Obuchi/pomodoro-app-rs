//! Concrete, I/O-free domain values. Serialization is a platform concern; these
//! values are not themselves the V1 JSON wire contract.

use std::{error::Error, fmt};

use crate::{ConfigError, TimerConfig};

/// UTC milliseconds since the Unix epoch. Not a monotonic runtime anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InterruptionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Focus,
    QuickStart,
    ShortBreak,
    LongBreak,
}

impl SessionKind {
    #[must_use]
    pub const fn is_work(self) -> bool {
        matches!(self, Self::Focus | Self::QuickStart)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Completed,
    Cancelled,
    Reset,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTask(String);

impl CurrentTask {
    /// Normalizes optional one-line input. Blank input means no task.
    ///
    /// # Errors
    /// Returns an error for embedded line separators.
    pub fn parse(input: &str) -> Result<Option<Self>, DomainError> {
        if input.trim().is_empty() {
            return Ok(None);
        }
        if input.contains(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']) {
            return Err(DomainError::InvalidTask);
        }
        let value = input.trim();
        Ok((!value.is_empty()).then(|| Self(value.to_owned())))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,
    pub current_task: Option<CurrentTask>,
    pub planned_duration_ms: u64,
    pub started_at: Timestamp,
    pub elapsed_ms: u64,
    pub continued_from_quick_start: Option<SessionId>,
    pub end: Option<SessionEnd>,
}

impl Session {
    #[must_use]
    pub const fn remaining_ms(&self) -> u64 {
        self.planned_duration_ms.saturating_sub(self.elapsed_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEnd {
    pub ended_at: Timestamp,
    pub outcome: SessionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerState {
    Running { run: RunState },
    Interrupted { interruption: Interruption },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunState {
    pub started_at: Timestamp,
    pub elapsed_ms_at_start: u64,
    pub last_confirmed_at: Timestamp,
    pub time_uncertainty: Option<TimeUncertainty>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interruption {
    pub id: InterruptionId,
    pub kind: InterruptionKind,
    pub started_at: Timestamp,
    pub recorded_at: Timestamp,
    pub time_uncertainty: Option<TimeUncertainty>,
    pub end: Option<InterruptionEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionKind {
    Pause,
    Distraction,
    AppExit,
    ObservationGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionOutcome {
    Resumed,
    Returned,
    SessionEnded { session_outcome: SessionOutcome },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUncertainty {
    ClockMovedBackward,
    ClockDiscontinuity,
    InsufficientClockEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasuredDuration {
    Known { elapsed_ms: u64 },
    Unknown { reason: TimeUncertainty },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptionEnd {
    pub ended_at: Timestamp,
    pub outcome: InterruptionOutcome,
    pub duration: MeasuredDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoundProgress {
    pub completed_focuses_in_round: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PomodoroState {
    pub settings: TimerConfig,
    pub round_progress: RoundProgress,
    pub state: ProgressState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressState {
    Ready {
        next_kind: SessionKind,
        current_task_draft: Option<CurrentTask>,
    },
    Active {
        session: Session,
        timer: TimerState,
    },
    AwaitingQuickStartDecision {
        quick_start_session_id: SessionId,
        current_task: Option<CurrentTask>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickStartChoice {
    Finish,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickStartDecision {
    Finish,
    Continue { focus_session_id: SessionId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    pub sessions: Vec<Session>,
    pub events: Vec<HistoryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEvent {
    pub sequence: u64,
    pub session_id: SessionId,
    pub effective_at: Timestamp,
    pub recorded_at: Timestamp,
    pub payload: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    RunIntervalRecorded {
        started_at: Timestamp,
        ended_at: Timestamp,
        credited_ms: u64,
        time_uncertainty: Option<TimeUncertainty>,
    },
    InterruptionStarted {
        interruption_id: InterruptionId,
        interruption_kind: InterruptionKind,
    },
    InterruptionEnded {
        interruption_id: InterruptionId,
        end: InterruptionEnd,
    },
    QuickStartDecisionMade {
        decision: QuickStartDecision,
    },
    AppClosing,
    AppRestored,
    ObservationGapDetected {
        last_confirmed_at: Timestamp,
        detected_at: Timestamp,
        reason: GapReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapReason {
    Restart,
    ObservationDiscontinuity,
    ClockAnomaly,
}

/// Allocated atomically with the domain change, within the adopted save lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdAllocators {
    pub next_session_id: u64,
    pub next_interruption_id: u64,
    pub next_event_sequence: u64,
}

impl Default for IdAllocators {
    fn default() -> Self {
        Self {
            next_session_id: 1,
            next_interruption_id: 1,
            next_event_sequence: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReflectionSummary {
    pub work_ms: u64,
    pub completed_focus_sessions: u64,
    pub distractions: u64,
    pub returns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidConfig(ConfigError),
    InvalidTask,
    InvalidOperation,
    WrongSession,
    ObservationRequired,
    Overflow,
    InvalidState(&'static str),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(error) => write!(f, "invalid settings: {error}"),
            Self::InvalidTask => f.write_str("current task must be a single line"),
            Self::InvalidOperation => f.write_str("operation not allowed in this state"),
            Self::WrongSession => f.write_str("operation targets a different or ended session"),
            Self::ObservationRequired => {
                f.write_str("observe time before operating on a running session")
            }
            Self::Overflow => f.write_str("domain counter overflow"),
            Self::InvalidState(reason) => write!(f, "invalid domain state: {reason}"),
        }
    }
}

impl Error for DomainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidConfig(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ConfigError> for DomainError {
    fn from(value: ConfigError) -> Self {
        Self::InvalidConfig(value)
    }
}
