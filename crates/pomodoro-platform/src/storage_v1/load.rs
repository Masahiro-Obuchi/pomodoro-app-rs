use std::{
    error::Error,
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use pomodoro_core::{DomainState, Timestamp};
use rustix::fs::{Mode, OFlags, open};

use crate::schema_v1::codec::{CodecError, PersistedStateV1};

use super::{LockedStorage, StorageLocation};

/// The only successful paths out of a locked load. Recovery is never a normal
/// write permit; neither loading nor offering recovery applies `RestoreApp`.
#[derive(Debug)]
pub enum LoadOutcome {
    New(WritableStorage),
    Loaded(WritableStorage),
    RecoveryRequired(RecoveryCandidate),
}

/// Validated saved data and the exact bytes used as the save baseline. The bytes
/// are retained because generation alone cannot detect external modifications.
#[derive(Debug)]
pub struct SavedState {
    state: PersistedStateV1,
    original_bytes: Vec<u8>,
}

impl SavedState {
    #[must_use]
    pub fn domain(&self) -> &DomainState {
        &self.state.domain
    }

    #[must_use]
    pub fn save_generation(&self) -> u64 {
        self.state.save_generation
    }

    #[must_use]
    pub fn saved_at(&self) -> Timestamp {
        self.state.saved_at
    }

    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }
}

/// Normal-write permission and its lock. Only a successful load/new-store check
/// constructs this handle. Atomic save will be added on this type in Phase 2-6.
/// No default/new constructor, Clone, or conversion from `LockedStorage` exists.
///
/// ```compile_fail
/// use pomodoro_platform::{LockedStorage, WritableStorage};
/// fn bypass_load(locked: LockedStorage) -> WritableStorage {
///     WritableStorage { locked, baseline: None }
/// }
/// ```
#[derive(Debug)]
#[must_use = "keep the write-permitted handle alive through the final save"]
pub struct WritableStorage {
    locked: LockedStorage,
    baseline: Option<SavedState>,
}

impl WritableStorage {
    #[must_use]
    pub fn location(&self) -> &StorageLocation {
        self.locked.location()
    }

    /// None means the load verified a new store. It is never a load-error fallback.
    #[must_use]
    pub fn saved_state(&self) -> Option<&SavedState> {
        self.baseline.as_ref()
    }
}

/// A validated .bak offered for explicit recovery, with no normal-write permit.
/// Retains the lock and original primary bytes for subsequent quarantine and
/// conflict checks. Phase 2-8 adds the separate recovery operation.
#[derive(Debug)]
#[must_use = "keep the recovery candidate alive while deciding whether to recover"]
pub struct RecoveryCandidate {
    locked: LockedStorage,
    backup: SavedState,
    primary_problem: LoadProblem,
    original_primary: Option<Vec<u8>>,
}

impl RecoveryCandidate {
    #[must_use]
    pub fn location(&self) -> &StorageLocation {
        self.locked.location()
    }

    #[must_use]
    pub fn backup(&self) -> &SavedState {
        &self.backup
    }

    #[must_use]
    pub fn primary_problem(&self) -> &LoadProblem {
        &self.primary_problem
    }

    /// None means the primary was absent, not unreadable.
    #[must_use]
    pub fn original_primary_bytes(&self) -> Option<&[u8]> {
        self.original_primary.as_deref()
    }
}

impl LockedStorage {
    /// Validates the primary before authorizing normal writes. A missing primary
    /// is new only when no backup or other remnants exist. Valid primary data wins
    /// over backups/temp files; unversioned/unsupported files and I/O errors stop
    /// the load without downgrading to a backup. No file is changed by this method.
    ///
    /// # Errors
    /// Returns [`LoadError`] for unsupported data, I/O failure, or corruption or
    /// remnants without a valid .bak. Errors release the lock and grant no writes.
    pub fn load(self) -> Result<LoadOutcome, LoadError> {
        let primary_path = self.location().state_path();
        let primary = read_optional(&primary_path).map_err(LoadError::primary)?;
        match primary {
            Some(bytes) => match PersistedStateV1::decode(&bytes) {
                Ok(state) => Ok(LoadOutcome::Loaded(WritableStorage {
                    locked: self,
                    baseline: Some(SavedState {
                        state,
                        original_bytes: bytes,
                    }),
                })),
                Err(error @ (CodecError::MissingVersion | CodecError::UnsupportedVersion(_))) => {
                    Err(LoadError::primary(format_problem(primary_path, error)))
                }
                Err(error) => self.check_backup(Some(bytes), format_problem(primary_path, error)),
            },
            None => self.check_backup(None, LoadProblem::Missing { path: primary_path }),
        }
    }

    fn check_backup(
        self,
        primary: Option<Vec<u8>>,
        problem: LoadProblem,
    ) -> Result<LoadOutcome, LoadError> {
        let path = self.location().backup_path();
        match read_optional(&path) {
            Ok(Some(bytes)) => match PersistedStateV1::decode(&bytes) {
                Ok(state) => Ok(LoadOutcome::RecoveryRequired(RecoveryCandidate {
                    locked: self,
                    backup: SavedState {
                        state,
                        original_bytes: bytes,
                    },
                    primary_problem: problem,
                    original_primary: primary,
                })),
                Err(error) => Err(LoadError::with_backup(problem, format_problem(path, error))),
            },
            Err(error) => Err(LoadError::with_backup(problem, error)),
            Ok(None) if primary.is_some() => Err(LoadError::primary(problem)),
            Ok(None) => {
                let remnants = remaining_entries(self.location()).map_err(LoadError::primary)?;
                if remnants.is_empty() {
                    Ok(LoadOutcome::New(WritableStorage {
                        locked: self,
                        baseline: None,
                    }))
                } else {
                    Err(LoadError {
                        primary: problem,
                        backup: None,
                        remnants,
                    })
                }
            }
        }
    }
}

// Never use Path::exists or a blanket NotFound fallback through symlinks. A
// dangling link is not an empty store, and a FIFO must not hang application startup.
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, LoadProblem> {
    let fd = match open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(LoadProblem::io(path, error.into())),
    };
    let mut file = fs::File::from(fd);
    if !file
        .metadata()
        .map_err(|error| LoadProblem::io(path, error))?
        .is_file()
    {
        return Err(LoadProblem::io(
            path,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "saved state must be a regular file",
            ),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| LoadProblem::io(path, error))?;
    Ok(Some(bytes))
}

// Inspect names only. Never parse or adopt temporary/quarantined files. This
// directory belongs to the application: unknown entries also prevent implicit
// initialization, including non-UTF-8 names and remnants from future versions.
fn remaining_entries(location: &StorageLocation) -> Result<Vec<PathBuf>, LoadProblem> {
    let entries = fs::read_dir(location.directory())
        .map_err(|error| LoadProblem::io(location.directory(), error))?;
    let mut remnants = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| LoadProblem::io(location.directory(), error))?;
        if entry.file_name() != "state.lock" {
            remnants.push(entry.path());
        }
    }
    remnants.sort();
    Ok(remnants)
}

fn format_problem(path: PathBuf, error: CodecError) -> LoadProblem {
    match error {
        CodecError::MissingVersion => LoadProblem::Unsupported {
            path,
            schema_version: None,
        },
        CodecError::UnsupportedVersion(version) => LoadProblem::Unsupported {
            path,
            schema_version: Some(version),
        },
        error => LoadProblem::Invalid {
            path,
            detail: error.to_string(),
        },
    }
}

#[derive(Debug)]
pub enum LoadProblem {
    Missing {
        path: PathBuf,
    },
    Unsupported {
        path: PathBuf,
        schema_version: Option<u32>,
    },
    Invalid {
        path: PathBuf,
        detail: String,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl LoadProblem {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for LoadProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(f, "saved state is missing: {}", path.display()),
            Self::Unsupported {
                path,
                schema_version: None,
            } => write!(
                f,
                "unversioned saved state is unsupported: {}",
                path.display()
            ),
            Self::Unsupported {
                path,
                schema_version: Some(version),
            } => write!(
                f,
                "unsupported schema version {version}: {}",
                path.display()
            ),
            Self::Invalid { path, detail } => {
                write!(f, "invalid saved state at {}: {detail}", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "saved state I/O failed at {}: {source}", path.display())
            }
        }
    }
}

impl Error for LoadProblem {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A failed load, retaining both primary and backup diagnostics when applicable.
/// No state or writable handle can be recovered from this error.
#[derive(Debug)]
pub struct LoadError {
    primary: LoadProblem,
    backup: Option<Box<LoadProblem>>,
    remnants: Vec<PathBuf>,
}

impl LoadError {
    fn primary(primary: LoadProblem) -> Self {
        Self {
            primary,
            backup: None,
            remnants: Vec::new(),
        }
    }

    fn with_backup(primary: LoadProblem, backup: LoadProblem) -> Self {
        Self {
            primary,
            backup: Some(Box::new(backup)),
            remnants: Vec::new(),
        }
    }

    #[must_use]
    pub fn primary_problem(&self) -> &LoadProblem {
        &self.primary
    }

    #[must_use]
    pub fn backup_problem(&self) -> Option<&LoadProblem> {
        self.backup.as_deref()
    }

    #[must_use]
    pub fn remnants(&self) -> &[PathBuf] {
        &self.remnants
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.primary)?;
        if let Some(backup) = &self.backup {
            write!(f, "; backup: {backup}")?;
        }
        if !self.remnants.is_empty() {
            write!(
                f,
                "; remaining files prevent initialization: {:?}",
                self.remnants
            )?;
        }
        Ok(())
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.primary)
    }
}
