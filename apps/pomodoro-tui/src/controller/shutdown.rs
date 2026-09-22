use super::{
    Clock, Command, Commit, CompletionNotifier, Controller, ControllerError, Mode, SaveKind,
    SaveStore,
};

/// Process-level result, distinct from a successful in-memory domain command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    Saved,
    /// Explicit exit without a confirmed final save, including uncertain commits.
    Unsaved,
    /// Startup/recovery was declined before attempting a save.
    Cancelled,
}

impl<S: SaveStore, C: Clock, N: CompletionNotifier> Controller<S, C, N> {
    /// Observes, applies `CloseApp` once, and saves before allowing a saved exit.
    /// Unlike session-scoped input, shutdown also applies after a completion/gap
    /// observed at this boundary. Notification still waits for the combined save.
    /// On failure use `retry()` or explicitly `exit()` with an unsaved outcome.
    ///
    /// # Errors
    /// Reports pending operations, closed state, clock/domain or save failure.
    /// An existing operation must finish retrying before shutdown can begin.
    pub fn shutdown(&mut self) -> Result<Commit, ControllerError> {
        self.require_operating()?;
        let observation = self.sample()?;
        let (_, mut effects) = self.apply_observation(observation)?;
        let events = self.domain.history().events.len();
        let before = self.domain.snapshot().clone();
        self.domain
            .apply(Command::CloseApp, observation.at)
            .map_err(ControllerError::Domain)?;
        self.dirty |=
            &before != self.domain.snapshot() || events != self.domain.history().events.len();
        effects.command = Some(Command::CloseApp);
        if !self.dirty {
            self.mode = Mode::Closed;
            return Ok(self.finish(effects));
        }
        // The candidate already stops timing. Retrying shutdown must neither
        // observe the wait nor generate another CloseApp or recovery gap.
        self.begin_save(observation.at, effects, SaveKind::Shutdown);
        self.flush()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        matches!(self.mode, Mode::Closed)
    }

    /// Releases the storage lock without attempting any further save. Call only
    /// after saved shutdown or an explicit user choice to leave without saving.
    /// A failed/uncertain save is never reported as a saved exit.
    #[must_use]
    pub fn exit(self) -> ExitOutcome {
        if self.is_closed() {
            ExitOutcome::Saved
        } else {
            ExitOutcome::Unsaved
        }
    }
}
