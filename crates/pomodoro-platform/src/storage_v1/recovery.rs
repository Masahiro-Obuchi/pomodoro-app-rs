use std::{io, path::Path};

use pomodoro_core::{Command, Timestamp};

use super::{
    PendingSave, RecoveryCandidate, SaveError, SaveStage, SavedState, StorageLocation,
    WritableStorage,
    file_io::{
        QuarantinedFile, SaveTarget, TemporaryFiles, at_stage, read_optional, replace_file,
        sync_directory,
    },
};
use crate::schema_v1::codec::PersistedStateV1;

/// Explicitly confirmed recovery, retaining the lock and one restored candidate.
/// Only a durable save yields normal-write permission. Failed attempts can only
/// retry this candidate; dropping the handle never undoes a possible rename.
#[derive(Debug)]
#[must_use = "retain the recovery handle while saving or retrying"]
pub struct RecoverySave {
    source: Option<RecoveryCandidate>,
    pending: Option<PendingSave>,
    quarantine: Option<QuarantinedFile>,
}

impl RecoveryCandidate {
    /// Call only after the user explicitly accepts this backup's timestamp and
    /// possible loss of later data. Declining/cancelling means dropping `self`.
    /// Fixes `RestoreApp`, IDs, generation, timestamp and JSON exactly once, with
    /// no disk writes. `RecoverySave::save` archives the primary before replacing
    /// it. Callers must not apply another `RestoreApp` after recovery succeeds.
    ///
    /// # Errors
    /// Rejects generation/ID overflow or an invalid restored candidate before I/O.
    pub fn confirm(self, restored_at: Timestamp) -> Result<RecoverySave, SaveError> {
        let generation = self
            .backup
            .save_generation()
            .checked_add(1)
            .ok_or(SaveError::GenerationExhausted)?;
        let mut domain = self.backup.domain().clone();
        domain
            .apply(Command::RestoreApp, restored_at)
            .map_err(|error| SaveError::InvalidCandidate(error.to_string()))?;
        let state = PersistedStateV1 {
            domain,
            save_generation: generation,
            saved_at: restored_at,
        };
        let original_bytes = state
            .encode()
            .map_err(|error| SaveError::InvalidCandidate(error.to_string()))?;
        Ok(RecoverySave {
            source: Some(self),
            pending: Some(PendingSave {
                candidate: SavedState {
                    state,
                    original_bytes,
                },
                commit_uncertain: false,
                temporary_files: TemporaryFiles::default(),
            }),
            quarantine: None,
        })
    }
}

impl RecoverySave {
    #[must_use]
    pub fn pending_save(&self) -> Option<&PendingSave> {
        self.pending.as_ref()
    }

    /// The retained original primary, including after a failed directory sync.
    #[must_use]
    pub fn quarantine_path(&self) -> Option<&Path> {
        self.quarantine.as_ref().map(QuarantinedFile::path)
    }

    /// Saves or retries the same recovery candidate without updating `.bak`.
    /// Rechecks both source files and the archived primary before replacement.
    /// Matching candidate bytes are resynced; matching original bytes are
    /// rewritten. A third value stops recovery. On success the returned storage
    /// owns the lock and uses the restored candidate as its normal save baseline.
    ///
    /// # Errors
    /// Reports source conflicts and I/O failures, retaining the fixed candidate
    /// and commit uncertainty. Calling again after success returns `NoPendingSave`.
    pub fn save(&mut self) -> Result<WritableStorage, SaveError> {
        self.save_with_hook(&mut |_, _| Ok(()))
    }

    fn save_with_hook(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<WritableStorage, SaveError> {
        if self.pending.is_none() {
            return Err(SaveError::NoPendingSave);
        }
        if let Err(error) = self.attempt(before) {
            let pending = self
                .pending
                .as_mut()
                .expect("candidate retained on failure");
            pending.commit_uncertain |= error.is_commit_uncertain();
            return Err(error.with_commit_uncertainty(pending.commit_uncertain));
        }
        let mut source = self.source.take().expect("source retained until success");
        source.locked.directories_to_sync.clear();
        Ok(WritableStorage {
            locked: source.locked,
            baseline: Some(self.pending.take().expect("successful candidate").candidate),
            pending: None,
        })
    }

    fn attempt(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        let source = self.source.as_ref().expect("pending recovery has a source");
        let pending = self
            .pending
            .as_mut()
            .expect("pending recovery has a candidate");
        let location = source.location();
        at_stage(
            SaveStage::VerifyRecoverySources,
            location.directory(),
            before,
            || Ok(()),
        )?;
        let primary = read_optional(&location.state_path()).map_err(SaveError::Read)?;
        let matches_candidate = primary.as_deref() == Some(pending.encoded_bytes());
        if matches_candidate {
            pending.commit_uncertain = true;
        } else if primary != source.original_primary {
            return Err(SaveError::Conflict {
                path: location.state_path(),
            });
        }
        check_backup(source)?;
        if !matches_candidate {
            pending.commit_uncertain = false;
        }
        for path in &source.locked.directories_to_sync {
            at_stage(SaveStage::SyncStorageAncestry, path, before, || {
                sync_directory(path)
            })?;
        }
        if let Some(bytes) = &source.original_primary {
            if self.quarantine.is_none() {
                // Never archive cached bytes after the primary has already changed.
                if matches_candidate {
                    return Err(SaveError::Conflict {
                        path: location.state_path(),
                    });
                }
                self.quarantine = Some(QuarantinedFile::create(
                    &location.state_path(),
                    bytes,
                    before,
                )?);
            }
            self.quarantine
                .as_ref()
                .expect("original primary archived")
                .sync(bytes, before)?;
        }
        at_stage(
            SaveStage::VerifyRecoverySources,
            location.directory(),
            before,
            || Ok(()),
        )?;
        check_backup(source)?;
        check_primary(location, primary.as_deref())?;
        if matches_candidate {
            return at_stage(
                SaveStage::SyncPrimaryDirectory,
                location.directory(),
                before,
                || sync_directory(location.directory()),
            );
        }
        pending.temporary_files.cleanup();
        replace_file(
            &location.state_path(),
            pending.candidate.original_bytes(),
            SaveTarget::Primary,
            &mut pending.temporary_files,
            before,
        )
    }
}

fn check_backup(source: &RecoveryCandidate) -> Result<(), SaveError> {
    let path = source.location().backup_path();
    if read_optional(&path).map_err(SaveError::Read)?.as_deref()
        != Some(source.backup.original_bytes())
    {
        return Err(SaveError::Conflict { path });
    }
    Ok(())
}

fn check_primary(location: &StorageLocation, expected: Option<&[u8]>) -> Result<(), SaveError> {
    let path = location.state_path();
    if read_optional(&path).map_err(SaveError::Read)?.as_deref() != expected {
        return Err(SaveError::Conflict { path });
    }
    Ok(())
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
