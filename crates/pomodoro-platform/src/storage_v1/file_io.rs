//! Concrete file access, atomic replacement, and ownership of temporary files.
//! Load/save policy and commit accounting stay in their respective modules.

use super::{LoadProblem, SaveError, SaveStage, StorageLocation};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

mod native_unix;
use native_unix::{create_new_private, owns_path, sync_file};
pub(super) use native_unix::{read_optional, sync_directory};

// Inspect names only. Never parse or adopt temporary/quarantined files. This
// directory belongs to the application: unknown entries also prevent implicit
// initialization, including non-UTF-8 names and remnants from future versions.
pub(super) fn remaining_entries(location: &StorageLocation) -> Result<Vec<PathBuf>, LoadProblem> {
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

#[derive(Clone, Copy)]
pub(super) enum SaveTarget {
    Backup,
    Primary,
}

impl SaveTarget {
    fn stages(self) -> [SaveStage; 5] {
        match self {
            Self::Backup => [
                SaveStage::CreateBackupTemp,
                SaveStage::WriteBackupTemp,
                SaveStage::SyncBackupTemp,
                SaveStage::RenameBackup,
                SaveStage::SyncBackupDirectory,
            ],
            Self::Primary => [
                SaveStage::CreatePrimaryTemp,
                SaveStage::WritePrimaryTemp,
                SaveStage::SyncPrimaryTemp,
                SaveStage::RenamePrimary,
                SaveStage::SyncPrimaryDirectory,
            ],
        }
    }
}

/// Failed cleanups retain open descriptors to preserve inode identity.
/// An external replacement at the same pathname never becomes our file.
#[derive(Debug, Default)]
pub(super) struct TemporaryFiles(Vec<TemporaryFile>);

impl TemporaryFiles {
    pub(super) fn owns(&self, path: &Path) -> io::Result<bool> {
        for temporary in &self.0 {
            if temporary.path.as_deref() == Some(path) {
                return temporary.owns_path(path);
            }
        }
        Ok(false)
    }

    pub(super) fn cleanup(&mut self) {
        self.0.retain_mut(|temporary| temporary.cleanup().is_err());
    }
}

pub(super) fn replace_file(
    target: &Path,
    bytes: &[u8],
    kind: SaveTarget,
    leftovers: &mut TemporaryFiles,
    before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    replace_file_with_check(target, bytes, kind, leftovers, before, || Ok(()))
}

/// Run the caller's final policy check after the temporary file is synced and
/// immediately before rename. A rejected check uses the same temporary cleanup
/// path as an I/O failure and never replaces the target.
pub(super) fn replace_file_with_check(
    target: &Path,
    bytes: &[u8],
    kind: SaveTarget,
    leftovers: &mut TemporaryFiles,
    before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    check: impl FnOnce() -> Result<(), SaveError>,
) -> Result<(), SaveError> {
    let [create, write, sync, rename, directory_sync] = kind.stages();
    let mut temporary = at_stage(create, target, before, || TemporaryFile::create(target))?;
    let result = (|| {
        let path = temporary
            .path
            .as_ref()
            .expect("temporary has not been renamed");
        at_stage(write, path, before, || temporary.file.write_all(bytes))?;
        at_stage(sync, path, before, || sync_file(&temporary.file))?;
        let rename_error = |source| SaveError::Io {
            stage: rename,
            path: target.to_owned(),
            source,
        };
        before(rename, target).map_err(rename_error)?;
        check()?;
        fs::rename(path, target).map_err(rename_error)?;
        temporary.path = None;
        let parent = target.parent().expect("storage targets have a parent");
        at_stage(directory_sync, parent, before, || sync_directory(parent))
    })();
    if temporary.cleanup().is_err() {
        leftovers.0.push(temporary);
    }
    result
}

pub(super) fn at_stage<T>(
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

#[derive(Debug)]
struct TemporaryFile {
    file: fs::File,
    path: Option<PathBuf>,
}

impl TemporaryFile {
    fn create(target: &Path) -> io::Result<Self> {
        Self::create_with_suffix(target, "tmp")
    }

    fn create_with_suffix(target: &Path, suffix: &str) -> io::Result<Self> {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let sequence = NEXT_TEMP
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| io::Error::other("temporary file sequence exhausted"))?;
            let mut path = target.as_os_str().to_os_string();
            path.push(format!(".{suffix}-{}-{sequence}", std::process::id()));
            let path = PathBuf::from(path);
            match create_new_private(&path) {
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

    fn owns_path(&self, path: &Path) -> io::Result<bool> {
        owns_path(&self.file, path)
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if let Some(path) = &self.path {
            if self.owns_path(path)? {
                fs::remove_file(path)?;
            }
            self.path = None;
        }
        Ok(())
    }
}

/// A recovery archive is never deleted on drop, even if a later sync fails.
/// Keep its descriptor to detect a replacement at the same pathname on retry.
#[derive(Debug)]
pub(super) struct QuarantinedFile {
    file: fs::File,
    path: PathBuf,
}

impl QuarantinedFile {
    pub(super) fn create(
        primary: &Path,
        bytes: &[u8],
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<Self, SaveError> {
        let mut temporary = at_stage(SaveStage::CreateQuarantine, primary, before, || {
            TemporaryFile::create_with_suffix(primary, "quarantine")
        })?;
        let path = temporary.path.as_ref().expect("new quarantine has a path");
        at_stage(SaveStage::WriteQuarantine, path, before, || {
            temporary.file.write_all(bytes)
        })?;
        // Preserve the complete copy from here onward, including sync failures.
        let file = temporary.file.try_clone().map_err(|source| SaveError::Io {
            stage: SaveStage::WriteQuarantine,
            path: path.clone(),
            source,
        })?;
        Ok(Self {
            file,
            path: temporary.path.take().expect("quarantine has not moved"),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn verify(&self, bytes: &[u8]) -> Result<(), SaveError> {
        if !owns_path(&self.file, &self.path)
            .map_err(|source| SaveError::Read(LoadProblem::io(&self.path, source)))?
            || read_optional(&self.path)
                .map_err(SaveError::Read)?
                .as_deref()
                != Some(bytes)
        {
            return Err(SaveError::Conflict {
                path: self.path.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn sync(
        &self,
        before: &mut impl FnMut(SaveStage, &Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        at_stage(SaveStage::SyncQuarantine, &self.path, before, || {
            sync_file(&self.file)
        })?;
        let parent = self.path.parent().expect("quarantine has a parent");
        at_stage(SaveStage::SyncQuarantineDirectory, parent, before, || {
            sync_directory(parent)
        })
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        // Never delete a replacement file merely because its name matches.
        let _ = self.cleanup();
    }
}
