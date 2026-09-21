use std::{error::Error, fmt, io, path::PathBuf};

use super::LoadProblem;

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
