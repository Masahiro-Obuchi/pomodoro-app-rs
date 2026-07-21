use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use directories::BaseDirs;
use pomodoro_core::{History, PomodoroTimer, TimerConfig};
use serde::{Deserialize, Serialize};

const APPLICATION_DIRECTORY: &str = "pomodoro-app-rs";
const STATE_FILE: &str = "state.json";

/// A persistence unit containing timer state and daily history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedState {
    pub timer: PomodoroTimer,
    pub history: History,
}

impl PersistedState {
    /// Creates empty persisted state from an initial configuration.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidTimer`] if the configuration is invalid.
    pub fn new(config: TimerConfig) -> Result<Self, StorageError> {
        Ok(Self {
            timer: PomodoroTimer::new(config).map_err(StorageError::InvalidTimer)?,
            history: History::new(),
        })
    }

    /// Validates the timer and history after deserialization.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the timer or history is inconsistent with the
    /// current format.
    pub fn validate(&self) -> Result<(), StorageError> {
        self.timer.validate().map_err(StorageError::InvalidTimer)?;
        self.history
            .validate()
            .map_err(StorageError::InvalidHistory)
    }
}

/// Native JSON storage under the XDG state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStorage {
    state_path: PathBuf,
}

impl NativeStorage {
    /// Resolves the storage path from the current user's XDG directories.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NoBaseDirectory`] if a user base directory cannot be
    /// determined.
    pub fn discover() -> Result<Self, StorageError> {
        let base_dirs = BaseDirs::new().ok_or(StorageError::NoBaseDirectory)?;
        let base = base_dirs
            .state_dir()
            .unwrap_or_else(|| base_dirs.data_local_dir());
        Ok(Self::at(base.join(APPLICATION_DIRECTORY).join(STATE_FILE)))
    }

    #[must_use]
    pub fn at(state_path: PathBuf) -> Self {
        Self { state_path }
    }

    #[must_use]
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Loads persisted state, returning `Ok(None)` when the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the file cannot be read, the JSON is malformed, or
    /// validation fails.
    pub fn load(&self) -> Result<Option<PersistedState>, StorageError> {
        let bytes = match fs::read(&self.state_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StorageError::Io(error)),
        };
        let state: PersistedState =
            serde_json::from_slice(&bytes).map_err(StorageError::InvalidJson)?;
        state.validate()?;
        Ok(Some(state))
    }

    /// Writes to a temporary file in the same directory before replacing the state file.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if directory creation, JSON serialization, writing,
    /// synchronization, or replacement fails.
    pub fn save(&self, state: &PersistedState) -> Result<(), StorageError> {
        state.validate()?;
        let parent = self
            .state_path
            .parent()
            .ok_or(StorageError::InvalidStoragePath)?;
        fs::create_dir_all(parent).map_err(StorageError::Io)?;

        let json = serde_json::to_vec_pretty(state).map_err(StorageError::InvalidJson)?;
        let temporary = self
            .state_path
            .with_extension(format!("json.tmp-{}", std::process::id()));
        let mut file = fs::File::create(&temporary).map_err(StorageError::Io)?;
        io::Write::write_all(&mut file, &json).map_err(StorageError::Io)?;
        file.sync_all().map_err(StorageError::Io)?;
        fs::rename(temporary, &self.state_path).map_err(StorageError::Io)
    }
}

#[derive(Debug)]
pub enum StorageError {
    NoBaseDirectory,
    InvalidStoragePath,
    Io(io::Error),
    InvalidJson(serde_json::Error),
    InvalidTimer(pomodoro_core::TimerError),
    InvalidHistory(pomodoro_core::HistoryError),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseDirectory => {
                formatter.write_str("could not determine a user data directory")
            }
            Self::InvalidStoragePath => formatter.write_str("storage path has no parent directory"),
            Self::Io(error) => write!(formatter, "storage I/O failed: {error}"),
            Self::InvalidJson(error) => write!(formatter, "stored JSON is invalid: {error}"),
            Self::InvalidTimer(error) => write!(formatter, "stored timer is invalid: {error}"),
            Self::InvalidHistory(error) => write!(formatter, "stored history is invalid: {error}"),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidJson(error) => Some(error),
            Self::InvalidTimer(error) => Some(error),
            Self::InvalidHistory(error) => Some(error),
            Self::NoBaseDirectory | Self::InvalidStoragePath => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_state_atomically() {
        let unique = format!(
            "pomodoro-platform-test-{}-{}",
            std::process::id(),
            crate::unix_time_millis().unwrap()
        );
        let directory = std::env::temp_dir().join(unique);
        let storage = NativeStorage::at(directory.join(STATE_FILE));
        let state = PersistedState::new(TimerConfig::default()).unwrap();

        storage.save(&state).unwrap();
        let loaded = storage.load().unwrap();

        assert_eq!(loaded, Some(state));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_state_is_not_an_error() {
        let storage = NativeStorage::at(
            std::env::temp_dir().join(format!("pomodoro-missing-{}", std::process::id())),
        );

        assert_eq!(storage.load().unwrap(), None);
    }
}
