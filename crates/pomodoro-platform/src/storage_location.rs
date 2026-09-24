use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::StorageLockError;

/// A directory containing V1 state, backup, and a dedicated lock file.
/// Constructing a location neither reads nor creates files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLocation {
    directory: PathBuf,
}

impl StorageLocation {
    /// Resolves the platform's local user storage directory.
    ///
    /// Linux retains the XDG state directory. macOS and Windows use the
    /// platform's local application-data directory.
    ///
    /// # Errors
    /// Returns [`StorageLockError::NoBaseDirectory`] if no base is available.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_location_uses_native_local_base() {
        let base = BaseDirs::new().expect("user base directory");
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(
            base.state_dir().is_none(),
            "macOS and Windows storage should use the local data directory"
        );
        let expected_base = base.state_dir().unwrap_or_else(|| base.data_local_dir());
        let location = StorageLocation::discover().expect("storage location");
        assert_eq!(location.directory(), expected_base.join("pomodoro-app-rs"));
        assert_eq!(
            location.state_path(),
            location.directory().join("state.json")
        );
        assert_eq!(
            location.backup_path(),
            location.directory().join("state.json.bak")
        );
        assert_eq!(
            location.lock_path(),
            location.directory().join("state.lock")
        );
    }
}
