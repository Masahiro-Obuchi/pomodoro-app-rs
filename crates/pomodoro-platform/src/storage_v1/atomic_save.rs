use std::{io, path::Path};

use pomodoro_core::{DomainState, Timestamp};

use crate::schema_v1::codec::PersistedStateV1;

use super::{
    SaveError, SaveStage, SavedState, WritableStorage,
    file_io::{at_stage, read_optional, remaining_entries, replace_file, sync_directory},
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

#[cfg(test)]
#[path = "save_tests/mod.rs"]
mod tests;
