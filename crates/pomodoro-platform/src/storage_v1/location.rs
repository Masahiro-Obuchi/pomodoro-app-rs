use std::path::{Path, PathBuf};

use directories::BaseDirs;

use super::StorageLockError;

/// A directory containing V1 state, backup, and a dedicated lock file.
/// Constructing a location neither reads nor creates files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLocation {
    directory: PathBuf,
}

impl StorageLocation {
    /// Resolves the same XDG state directory as the existing TUI.
    ///
    /// # Errors
    /// Returns [`StorageLockError::NoBaseDirectory`] if no base directory is available.
    pub fn discover() -> Result<Self, StorageLockError> {
        let base_dirs = BaseDirs::new().ok_or(StorageLockError::NoBaseDirectory)?;
        let base = base_dirs
            .state_dir()
            .unwrap_or_else(|| base_dirs.data_local_dir());
        Ok(Self::at(base.join("pomodoro-app-rs")))
    }

    /// Selects a storage directory, not a state file. Relative paths are resolved
    /// when acquiring the lock; the locked handle retains a canonical directory.
    #[must_use]
    pub fn at(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.directory.join("state.json")
    }

    #[must_use]
    pub fn backup_path(&self) -> PathBuf {
        self.directory.join("state.json.bak")
    }

    #[must_use]
    pub fn lock_path(&self) -> PathBuf {
        self.directory.join("state.lock")
    }
}
