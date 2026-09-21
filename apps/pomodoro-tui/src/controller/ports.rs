use pomodoro_core::{DomainState, Observation, SessionKind, Timestamp};
use pomodoro_platform::{
    NotificationError, ObservationClock, SaveError, TimeError, WritableStorage,
};

/// The V1 durable-save boundary, including fixed-candidate retry. Implementations
/// must retain their baseline on failure and advance it only after durable success.
pub trait SaveStore {
    fn saved_domain(&self) -> Option<&DomainState>;
    fn has_pending_save(&self) -> bool;

    /// # Errors
    /// Returns the save failure without treating an uncertain commit as success.
    fn save(&mut self, domain: &DomainState, at: Timestamp) -> Result<(), SaveError>;

    /// # Errors
    /// Retries only the store's fixed candidate; errors retain that candidate.
    fn retry_pending(&mut self) -> Result<(), SaveError>;
}

impl SaveStore for WritableStorage {
    fn saved_domain(&self) -> Option<&DomainState> {
        self.saved_state()
            .map(pomodoro_platform::SavedState::domain)
    }

    fn has_pending_save(&self) -> bool {
        self.pending_save().is_some()
    }

    fn save(&mut self, domain: &DomainState, at: Timestamp) -> Result<(), SaveError> {
        Self::save(self, domain, at).map(|_| ())
    }

    fn retry_pending(&mut self) -> Result<(), SaveError> {
        Self::retry_pending(self).map(|_| ())
    }
}

/// Runtime observations, never stored as clock anchors in the V1 snapshot.
pub trait Clock {
    /// # Errors
    /// Returns a clock-read error instead of inventing an observation.
    fn observe(&mut self) -> Result<Observation, TimeError>;
    fn break_continuity(&mut self);
}

impl Clock for ObservationClock {
    fn observe(&mut self) -> Result<Observation, TimeError> {
        Self::observe(self)
    }

    fn break_continuity(&mut self) {
        Self::break_continuity(self);
    }
}

/// Receives completion only after the corresponding save succeeds.
pub trait CompletionNotifier {
    /// # Errors
    /// Notification failure is a warning; it cannot roll back a saved completion.
    fn session_completed(&mut self, kind: SessionKind) -> Result<(), NotificationError>;
}

impl<F> CompletionNotifier for F
where
    F: FnMut(SessionKind) -> Result<(), NotificationError>,
{
    fn session_completed(&mut self, kind: SessionKind) -> Result<(), NotificationError> {
        self(kind)
    }
}
