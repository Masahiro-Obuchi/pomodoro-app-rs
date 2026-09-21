use std::{
    error::Error,
    fmt, fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use pomodoro_core::{DomainState, Timestamp};
use rustix::fs::{Mode, OFlags, open};

use crate::schema_v1::codec::PersistedStateV1;

use super::{
    LoadProblem, SavedState, WritableStorage,
    load::{read_optional, remaining_entries},
};

/// One validated, encoded candidate. Its domain, IDs, generation, timestamp and
/// bytes stay fixed after a failed attempt; it is not a committed saved state.
#[derive(Debug)]
pub struct PendingSave {
    candidate: SavedState,
}

impl PendingSave {
    #[must_use]
    pub fn domain(&self) -> &DomainState {
        self.candidate.domain()
    }

    #[must_use]
    pub fn save_generation(&self) -> u64 {
        self.candidate.save_generation()
    }

    #[must_use]
    pub fn saved_at(&self) -> Timestamp {
        self.candidate.saved_at()
    }

    #[must_use]
    pub fn encoded_bytes(&self) -> &[u8] {
        self.candidate.original_bytes()
    }
}

impl WritableStorage {
    /// The immutable candidate retained after an unsuccessful write attempt.
    #[must_use]
    pub fn pending_save(&self) -> Option<&PendingSave> {
        self.pending.as_ref()
    }

    /// Saves a fixed candidate under the existing lock. The previous validated
    /// primary becomes the backup; success includes file and directory syncs.
    /// No domain transition, clock sampling or notification is performed here.
    ///
    /// A failed I/O attempt retains the candidate and the last committed baseline.
    /// Further normal saves are blocked while that candidate remains pending.
    ///
    /// # Errors
    /// Rejects a pending save, external primary changes, generation overflow and
    /// invalid candidates before writing. I/O failures report the save stage;
    /// failure after primary rename is an uncertain commit, not a rolled-back save.
    pub fn save(
        &mut self,
        domain: &DomainState,
        saved_at: Timestamp,
    ) -> Result<&SavedState, SaveError> {
        self.save_with_hook(domain, saved_at, &mut |_, _| Ok(()))
    }

    // A hook at save-specific I/O boundaries lets tests fail individual writes,
    // syncs and renames. It is private, with no runtime environment switches or
    // general-purpose filesystem abstraction.
    fn save_with_hook(
        &mut self,
        domain: &DomainState,
        saved_at: Timestamp,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<&SavedState, SaveError> {
        if self.pending.is_some() {
            return Err(SaveError::PendingSave);
        }
        self.check_baseline()?;
        let generation = self
            .baseline
            .as_ref()
            .map_or(0, SavedState::save_generation)
            .checked_add(1)
            .ok_or(SaveError::GenerationExhausted)?;
        let state = PersistedStateV1 {
            save_generation: generation,
            saved_at,
            domain: domain.clone(),
        };
        let bytes = state
            .encode()
            .map_err(|error| SaveError::InvalidCandidate(error.to_string()))?;
        self.pending = Some(PendingSave {
            candidate: SavedState {
                state,
                original_bytes: bytes,
            },
        });
        self.write_pending_from_old_primary(before)
    }

    /// Retries the one fixed candidate retained by an earlier failed save.
    ///
    /// If the primary already equals that candidate, this only repeats the
    /// required directory sync and commits the same generation. If the primary
    /// still equals the last committed baseline, it writes the same candidate.
    /// Any other primary bytes are a conflict and no backup is changed.
    ///
    /// # Errors
    /// Returns [`SaveError::NoPendingSave`] when there is no failed candidate to
    /// retry. I/O errors and conflicts retain the candidate unchanged, so a later
    /// retry cannot create new IDs, events, generations, or timestamps.
    pub fn retry_pending(&mut self) -> Result<&SavedState, SaveError> {
        self.retry_pending_with_hook(&mut |_, _| Ok(()))
    }

    fn retry_pending_with_hook(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<&SavedState, SaveError> {
        if self.pending.is_none() {
            return Err(SaveError::NoPendingSave);
        }
        let primary_path = self.location().state_path();
        let primary = read_optional(&primary_path).map_err(SaveError::Read)?;
        let pending = self.pending.as_ref().expect("checked above");
        if primary.as_deref() == Some(pending.encoded_bytes()) {
            self.sync_storage_ancestry(before)?;
            let directory = self.location().directory();
            at_stage(SaveStage::SyncPrimaryDirectory, directory, before, || {
                sync_directory(directory)
            })?;
            return Ok(self.commit_pending());
        }
        if primary.as_deref() == self.baseline.as_ref().map(SavedState::original_bytes) {
            return self.write_pending_from_old_primary(before);
        }
        Err(SaveError::Conflict { path: primary_path })
    }

    fn write_pending_from_old_primary(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<&SavedState, SaveError> {
        self.sync_storage_ancestry(before)?;
        let location = self.location();
        if let Some(baseline) = &self.baseline {
            replace_file(
                &location.backup_path(),
                baseline.original_bytes(),
                true,
                before,
            )?;
        }
        let pending = self.pending.as_ref().expect("candidate was fixed above");
        replace_file(
            &location.state_path(),
            pending.encoded_bytes(),
            false,
            before,
        )?;
        Ok(self.commit_pending())
    }

    fn sync_storage_ancestry(
        &self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        for path in &self.locked.directories_to_sync {
            at_stage(SaveStage::SyncStorageAncestry, path, before, || {
                sync_directory(path)
            })?;
        }
        Ok(())
    }

    fn commit_pending(&mut self) -> &SavedState {
        // Only the completed directory sync grants a committed generation.
        self.baseline = Some(
            self.pending
                .take()
                .expect("candidate exists until commit")
                .candidate,
        );
        self.locked.directories_to_sync.clear();
        self.baseline
            .as_ref()
            .expect("successful save establishes a baseline")
    }

    fn check_baseline(&self) -> Result<(), SaveError> {
        let path = self.location().state_path();
        let current = read_optional(&path).map_err(SaveError::Read)?;
        if current.as_deref() != self.baseline.as_ref().map(SavedState::original_bytes) {
            return Err(SaveError::Conflict { path });
        }
        // A new-store permit cannot discard a backup/remnant added after load.
        if self.baseline.is_none() {
            if let Some(path) = remaining_entries(self.location())
                .map_err(SaveError::Read)?
                .into_iter()
                .next()
            {
                return Err(SaveError::Conflict { path });
            }
        }
        Ok(())
    }
}

fn replace_file(
    target: &Path,
    bytes: &[u8],
    backup: bool,
    before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    let (create, write, sync, rename, directory_sync) = if backup {
        (
            SaveStage::CreateBackupTemp,
            SaveStage::WriteBackupTemp,
            SaveStage::SyncBackupTemp,
            SaveStage::RenameBackup,
            SaveStage::SyncBackupDirectory,
        )
    } else {
        (
            SaveStage::CreatePrimaryTemp,
            SaveStage::WritePrimaryTemp,
            SaveStage::SyncPrimaryTemp,
            SaveStage::RenamePrimary,
            SaveStage::SyncPrimaryDirectory,
        )
    };
    let mut temporary = at_stage(create, target, before, || TemporaryFile::create(target))?;
    let path = temporary
        .path
        .as_ref()
        .expect("temporary has not been renamed");
    at_stage(write, path, before, || temporary.file.write_all(bytes))?;
    at_stage(sync, path, before, || temporary.file.sync_all())?;
    at_stage(rename, target, before, || fs::rename(path, target))?;
    temporary.path = None;
    let parent = target.parent().expect("storage targets have a parent");
    at_stage(directory_sync, parent, before, || sync_directory(parent))
}

fn sync_directory(path: &Path) -> io::Result<()> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    fs::File::from(fd).sync_all()
}

fn at_stage<T>(
    stage: SaveStage,
    path: &Path,
    before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    operation: impl FnOnce() -> io::Result<T>,
) -> Result<T, SaveError> {
    before(stage, path)
        .and_then(|()| operation())
        .map_err(|source| SaveError::Io {
            stage,
            path: path.to_owned(),
            source,
        })
}

struct TemporaryFile {
    file: fs::File,
    path: Option<PathBuf>,
}

impl TemporaryFile {
    fn create(target: &Path) -> io::Result<Self> {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let sequence = NEXT_TEMP
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| io::Error::other("temporary file sequence exhausted"))?;
            let mut path = target.as_os_str().to_os_string();
            path.push(format!(".tmp-{}-{sequence}", std::process::id()));
            let path = PathBuf::from(path);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique save temporary file",
        ))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            // Cleanup only our exclusively created file. Failure leaves a remnant
            // which load will never mistake for an initial or committed state.
            let _ = fs::remove_file(path);
        }
    }
}

/// The I/O boundary that failed. Primary rename followed by a directory-sync
/// failure leaves commit durability unknown; all earlier failures preserve the
/// previous primary (a backup update may already have completed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStage {
    SyncStorageAncestry,
    CreateBackupTemp,
    WriteBackupTemp,
    SyncBackupTemp,
    RenameBackup,
    SyncBackupDirectory,
    CreatePrimaryTemp,
    WritePrimaryTemp,
    SyncPrimaryTemp,
    RenamePrimary,
    SyncPrimaryDirectory,
}

#[derive(Debug)]
pub enum SaveError {
    PendingSave,
    NoPendingSave,
    GenerationExhausted,
    InvalidCandidate(String),
    Conflict {
        path: PathBuf,
    },
    Read(LoadProblem),
    Io {
        stage: SaveStage,
        path: PathBuf,
        source: io::Error,
    },
}

impl SaveError {
    /// Whether this attempt replaced the primary without confirming durability.
    #[must_use]
    pub fn is_commit_uncertain(&self) -> bool {
        matches!(
            self,
            Self::Io {
                stage: SaveStage::SyncPrimaryDirectory,
                ..
            }
        )
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PendingSave => {
                f.write_str("a save candidate is already pending; normal saves are blocked")
            }
            Self::NoPendingSave => f.write_str("there is no pending save candidate to retry"),
            Self::GenerationExhausted => f.write_str("save generation exhausted"),
            Self::InvalidCandidate(detail) => write!(f, "invalid save candidate: {detail}"),
            Self::Conflict { path } => write!(
                f,
                "saved data changed outside this handle: {}",
                path.display()
            ),
            Self::Read(problem) => write!(f, "cannot verify save baseline: {problem}"),
            Self::Io {
                stage,
                path,
                source,
            } => write!(f, "save failed at {stage:?} ({}): {source}", path.display()),
        }
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "atomic_save_tests.rs"]
mod tests;
