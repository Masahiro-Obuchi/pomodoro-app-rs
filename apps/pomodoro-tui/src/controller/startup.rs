use std::{error::Error, fmt};

use pomodoro_core::{Command, DomainError, DomainState, TimerConfig, Timestamp};
use pomodoro_platform::{
    LoadError, LoadOutcome, RecoveryCandidate, RecoverySave, SaveError, StorageLocation,
    StorageLockError, WritableStorage,
};

use super::ExitOutcome;

/// No normal controller exists until startup's fixed candidate is durably saved.
/// A recovery offer retains its lock but grants no normal write permission.
#[derive(Debug)]
pub enum Startup {
    Saving(StartupSave),
    RecoveryRequired(Box<RecoveryCandidate>),
}

impl Startup {
    /// Locks, loads and validates before deciding whether initialization is safe.
    /// Pass the boundary of a fresh runtime clock as `at`; never credit downtime.
    /// `settings` applies only to a verified new store. No file is saved here.
    ///
    /// # Errors
    /// Lock, load and domain errors stop startup without a default-state fallback.
    ///
    /// # Panics
    /// Panics if the native loader returns `Loaded` without a validated baseline.
    pub fn open(
        location: StorageLocation,
        settings: TimerConfig,
        at: Timestamp,
    ) -> Result<Self, StartupError> {
        let loaded = location
            .lock()
            .map_err(StartupError::Lock)?
            .load()
            .map_err(StartupError::Load)?;
        let (store, mut domain) = match loaded {
            LoadOutcome::New(store) => {
                let domain = DomainState::new(settings).map_err(StartupError::Domain)?;
                (store, domain)
            }
            LoadOutcome::Loaded(store) => {
                let domain = store
                    .saved_state()
                    .expect("validated load")
                    .domain()
                    .clone();
                (store, domain)
            }
            LoadOutcome::RecoveryRequired(candidate) => {
                return Ok(Self::RecoveryRequired(Box::new(candidate)));
            }
        };
        domain
            .apply(Command::RestoreApp, at)
            .map_err(StartupError::Domain)?;
        Ok(Self::Saving(StartupSave {
            pending: Some(Box::new(StartupCandidate::Normal { store, domain, at })),
        }))
    }

    /// Declines a recovery offer without modifying files. Abandoning a prepared
    /// save is conservatively reported as unsaved, even before its first attempt.
    #[must_use]
    pub fn cancel(self) -> ExitOutcome {
        match self {
            Self::RecoveryRequired(_) => ExitOutcome::Cancelled,
            Self::Saving(save) => save.exit_without_saving(),
        }
    }
}

#[derive(Debug)]
enum StartupCandidate {
    Normal {
        store: WritableStorage,
        domain: DomainState,
        at: Timestamp,
    },
    Recovery(RecoverySave),
}

/// One prepared startup operation, retained across failed or uncertain saves.
/// Neither retry nor adopting its result applies `RestoreApp` a second time.
#[derive(Debug)]
#[must_use = "retain the startup candidate and its lock until saved or explicitly abandoned"]
pub struct StartupSave {
    pending: Option<Box<StartupCandidate>>,
}

impl StartupSave {
    /// Call only after showing the candidate's backup timestamp and possible
    /// loss of later data, and receiving explicit acceptance from the user.
    /// `RecoveryCandidate::confirm` applies the one required `RestoreApp`.
    ///
    /// # Errors
    /// Returns an invalid-candidate/overflow error without writing any file.
    pub fn confirm_recovery(
        candidate: RecoveryCandidate,
        at: Timestamp,
    ) -> Result<Self, SaveError> {
        Ok(Self {
            pending: Some(Box::new(StartupCandidate::Recovery(candidate.confirm(at)?))),
        })
    }

    /// The fixed startup candidate for a pending-save display, not normal input.
    #[must_use]
    pub fn pending_state(&self) -> Option<&DomainState> {
        match self.pending.as_deref()? {
            StartupCandidate::Normal { domain, .. } => Some(domain),
            StartupCandidate::Recovery(recovery) => recovery
                .pending_save()
                .map(pomodoro_platform::PendingSave::domain),
        }
    }

    /// Saves/retries without changing IDs, domain data or the candidate timestamp.
    /// Only success returns the locked store for `Controller::from_saved`; use a
    /// fresh runtime clock for that controller. No startup completion is notified.
    ///
    /// # Errors
    /// Save errors retain the candidate and lock. After success, further calls
    /// return `NoPendingSave` and cannot create another startup generation.
    pub fn save(&mut self) -> Result<WritableStorage, SaveError> {
        let pending = self
            .pending
            .as_deref_mut()
            .ok_or(SaveError::NoPendingSave)?;
        match pending {
            StartupCandidate::Normal { store, domain, at } => {
                if store.pending_save().is_some() {
                    store.retry_pending()?;
                } else {
                    // Failures before native candidate creation still retry the
                    // application's original RestoreApp and timestamp.
                    store.save(domain, *at)?;
                }
            }
            StartupCandidate::Recovery(recovery) => {
                let store = recovery.save()?;
                self.pending = None;
                return Ok(store);
            }
        }
        let Some(pending) = self.pending.take() else {
            unreachable!("normal startup was saved")
        };
        let StartupCandidate::Normal { store, .. } = *pending else {
            unreachable!("normal startup was saved")
        };
        Ok(store)
    }

    /// Explicitly abandons startup, releasing its lock. An uncertain rename is
    /// not rolled back, and the result never claims a confirmed saved exit.
    #[must_use]
    pub fn exit_without_saving(self) -> ExitOutcome {
        ExitOutcome::Unsaved
    }
}

#[derive(Debug)]
pub enum StartupError {
    Lock(StorageLockError),
    Load(LoadError),
    Domain(DomainError),
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => write!(f, "startup lock failed: {error}"),
            Self::Load(error) => write!(f, "startup load failed: {error}"),
            Self::Domain(error) => write!(f, "startup restore failed: {error}"),
        }
    }
}

impl Error for StartupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            Self::Load(error) => Some(error),
            Self::Domain(error) => Some(error),
        }
    }
}
