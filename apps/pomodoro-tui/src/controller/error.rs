use std::{error::Error, fmt};

use pomodoro_core::DomainError;
use pomodoro_platform::{SaveError, TimeError};

#[derive(Debug)]
pub enum ControllerError {
    MissingSavedState,
    StorageAlreadyPending,
    SavePending,
    NoPendingSave,
    Closed,
    Domain(DomainError),
    Save(SaveError),
    Time(TimeError),
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSavedState => f.write_str("controller requires a saved startup state"),
            Self::StorageAlreadyPending => f.write_str("storage already has a pending candidate"),
            Self::SavePending => f.write_str("save recovery is pending; normal input is blocked"),
            Self::NoPendingSave => f.write_str("there is no pending save to retry"),
            Self::Closed => f.write_str("application shutdown has already been saved"),
            Self::Domain(error) => write!(f, "domain operation failed: {error}"),
            Self::Save(error) => write!(f, "save failed: {error}"),
            Self::Time(error) => write!(f, "clock observation failed: {error}"),
        }
    }
}

impl Error for ControllerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Domain(error) => Some(error),
            Self::Save(error) => Some(error),
            Self::Time(error) => Some(error),
            _ => None,
        }
    }
}
