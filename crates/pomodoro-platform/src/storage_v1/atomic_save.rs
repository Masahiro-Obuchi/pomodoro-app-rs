use std::{io, path::Path};

use super::{
    LoadProblem, SaveError, SaveStage, SavedState, WritableStorage,
    file_io::{
        SaveTarget, TemporaryFiles, at_stage, read_optional, remaining_entries, replace_file,
        sync_directory,
    },
};
use crate::schema_v1::codec::PersistedStateV1;
use pomodoro_core::{DomainState, Timestamp};

/// One fixed candidate, plus the I/O state required to retry it. Its domain,
/// IDs, generation, timestamp and encoded bytes never change between attempts.
#[derive(Debug)]
pub struct PendingSave {
    pub(super) candidate: SavedState,
    pub(super) commit_uncertain: bool,
    pub(super) temporary_files: TemporaryFiles,
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

    /// A primary rename has not yet been reconciled with a durable commit.
    #[must_use]
    pub fn is_commit_uncertain(&self) -> bool {
        self.commit_uncertain
    }
}

impl WritableStorage {
    #[must_use]
    pub fn pending_save(&self) -> Option<&PendingSave> {
        self.pending.as_ref()
    }

    /// Saves a fixed candidate under the existing lock. The previous validated
    /// primary becomes the backup; success includes file and directory syncs.
    /// No domain transition, clock sampling or notification is performed here.
    /// Failed attempts retain the candidate and block further normal saves.
    ///
    /// # Errors
    /// Rejects pending saves, external changes, generation overflow and invalid
    /// candidates before writing. Errors preserve an uncertain commit status.
    pub fn save(
        &mut self,
        domain: &DomainState,
        saved_at: Timestamp,
    ) -> Result<&SavedState, SaveError> {
        self.save_with_hook(domain, saved_at, &mut |_, _| Ok(()))
    }

    // Injection stays at concrete save I/O boundaries, without runtime switches
    // or a general-purpose filesystem abstraction.
    fn save_with_hook(
        &mut self,
        domain: &DomainState,
        saved_at: Timestamp,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<&SavedState, SaveError> {
        if let Some(pending) = &self.pending {
            return Err(SaveError::PendingSave.with_commit_uncertainty(pending.commit_uncertain));
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
            commit_uncertain: false,
            temporary_files: TemporaryFiles::default(),
        });
        if let Err(error) = self.write_pending_from_old_primary(before) {
            return Err(self.record_failure(error));
        }
        Ok(self.commit_pending())
    }

    /// Retries the fixed candidate. Matching candidate bytes require only sync;
    /// matching baseline bytes permit rewriting the same candidate. Any third
    /// primary is a conflict. Initial retries also check for external remnants.
    ///
    /// # Errors
    /// Rejects a missing candidate, conflicting files or I/O failures. The
    /// candidate stays fixed, including after read failures and repeated retries.
    /// Errors retain commit uncertainty until primary reconciliation succeeds.
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
        if let Err(error) = self.retry_candidate(before) {
            return Err(self.record_failure(error));
        }
        Ok(self.commit_pending())
    }

    fn retry_candidate(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        let primary_path = self.location().state_path();
        let primary = read_optional(&primary_path).map_err(SaveError::Read)?;
        let pending = self.pending.as_mut().expect("retry requires a candidate");
        if primary.as_deref() == Some(pending.encoded_bytes()) {
            // Record this before any operation can fail, including ancestry sync.
            pending.commit_uncertain = true;
            self.sync_storage_ancestry(before)?;
            let directory = self.location().directory();
            return at_stage(SaveStage::SyncPrimaryDirectory, directory, before, || {
                sync_directory(directory)
            });
        }
        if primary.as_deref() != self.baseline.as_ref().map(SavedState::original_bytes) {
            return Err(SaveError::Conflict { path: primary_path });
        }
        self.check_new_store_remnants()?;
        // A successful comparison establishes that the old primary is current.
        self.pending
            .as_mut()
            .expect("retry requires a candidate")
            .commit_uncertain = false;
        self.write_pending_from_old_primary(before)
    }

    fn write_pending_from_old_primary(
        &mut self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        self.sync_storage_ancestry(before)?;
        let location = self.locked.location();
        let pending = self.pending.as_mut().expect("write requires a candidate");
        pending.temporary_files.cleanup();
        if let Some(baseline) = &self.baseline {
            replace_file(
                &location.backup_path(),
                baseline.original_bytes(),
                SaveTarget::Backup,
                &mut pending.temporary_files,
                before,
            )?;
        }
        replace_file(
            &location.state_path(),
            pending.candidate.original_bytes(),
            SaveTarget::Primary,
            &mut pending.temporary_files,
            before,
        )
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

    fn record_failure(&mut self, error: SaveError) -> SaveError {
        let pending = self.pending.as_mut().expect("attempt requires a candidate");
        pending.commit_uncertain |= error.is_commit_uncertain();
        error.with_commit_uncertainty(pending.commit_uncertain)
    }

    fn commit_pending(&mut self) -> &SavedState {
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
        self.check_new_store_remnants()
    }

    fn check_new_store_remnants(&self) -> Result<(), SaveError> {
        if self.baseline.is_some() {
            return Ok(());
        }
        for path in remaining_entries(self.location()).map_err(SaveError::Read)? {
            let owned = match &self.pending {
                Some(pending) => pending.temporary_files.owns(&path).map_err(|source| {
                    SaveError::Read(LoadProblem::Io {
                        path: path.clone(),
                        source,
                    })
                })?,
                None => false,
            };
            if !owned {
                return Err(SaveError::Conflict { path });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "save_tests/mod.rs"]
mod tests;
