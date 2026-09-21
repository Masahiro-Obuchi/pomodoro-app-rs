use std::{error::Error, fmt, io, path::PathBuf};

use super::LoadProblem;

/// The failed I/O operation. Commit uncertainty is separate from the operation:
/// an ancestry sync or read can fail while retrying an already-renamed primary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStage {
    SyncStorageAncestry,
    VerifyRecoverySources,
    CreateQuarantine,
    WriteQuarantine,
    SyncQuarantine,
    SyncQuarantineDirectory,
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
    /// Retains both the unconfirmed commit and the original failure diagnostics.
    CommitUncertain(Box<Self>),
}

impl SaveError {
    /// Whether the pending candidate may already be the primary without a
    /// confirmed durable commit, including failures during subsequent retries.
    #[must_use]
    pub fn is_commit_uncertain(&self) -> bool {
        matches!(
            self,
            Self::CommitUncertain(_)
                | Self::Io {
                    stage: SaveStage::SyncPrimaryDirectory,
                    ..
                }
        )
    }

    /// The underlying failure, without the retained commit-uncertainty wrapper.
    #[must_use]
    pub fn failure(&self) -> &Self {
        match self {
            Self::CommitUncertain(error) => error.failure(),
            error => error,
        }
    }

    pub(super) fn with_commit_uncertainty(self, uncertain: bool) -> Self {
        if uncertain && !self.is_commit_uncertain() {
            Self::CommitUncertain(Box::new(self))
        } else {
            self
        }
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
            Self::CommitUncertain(error) => write!(f, "save commit remains uncertain: {error}"),
        }
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::CommitUncertain(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}
