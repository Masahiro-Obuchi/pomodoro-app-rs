//! Unix V1 storage boundary. Acquire a lock before load/initialization/recovery;
//! owning a lock alone does not authorize overwriting an unread or invalid file.
//! Load policy grants normal writes only after a valid V1/new-store check.
//! Normal atomic save and retry retain one fixed candidate and its commit status.
//! Explicit backup recovery protects the source and quarantines the old primary.
//!
//! `load` owns read policy and write authorization; `atomic_save` owns candidates
//! and retry decisions. `file_io` implements concrete file access and temporary
//! ownership; `save_error` keeps failure diagnostics separate from commit status.

mod atomic_save;
mod file_io;
mod load;
mod lock;
mod recovery;
mod save_error;

pub use crate::{StorageLocation, StorageLockError};

#[cfg(test)]
pub(crate) fn test_tempdir() -> tempfile::TempDir {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("canonical test temp root");
    tempfile::Builder::new()
        .tempdir_in(root)
        .expect("test temp directory")
}
pub use atomic_save::PendingSave;
pub use load::{
    LoadError, LoadOutcome, LoadProblem, RecoveryCandidate, SavedState, WritableStorage,
};
pub use lock::LockedStorage;
pub use recovery::RecoverySave;
pub use save_error::{SaveError, SaveStage};
