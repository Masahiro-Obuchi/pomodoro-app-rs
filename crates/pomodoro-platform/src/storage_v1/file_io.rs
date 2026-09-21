//! Concrete file access, atomic replacement, and ownership of temporary files.
//! Load/save policy and commit accounting stay in their respective modules.

use super::{LoadProblem, SaveError, SaveStage, StorageLocation};
use rustix::fs::{Mode, OFlags, open};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

// Never use Path::exists or a blanket NotFound fallback through symlinks. A
// dangling link is not an empty store, and a FIFO must not hang application startup.
pub(super) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, LoadProblem> {
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
    let [create, write, sync, rename, directory_sync] = kind.stages();
    let mut temporary = at_stage(create, target, before, || TemporaryFile::create(target))?;
    let result = (|| {
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
    })();
    if temporary.cleanup().is_err() {
        leftovers.0.push(temporary);
    }
    result
}

pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    fs::File::from(fd).sync_all()
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

    fn owns_path(&self, path: &Path) -> io::Result<bool> {
        let actual = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let expected = self.file.metadata()?;
        Ok(actual.is_file() && actual.dev() == expected.dev() && actual.ino() == expected.ino())
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

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        // Never delete a replacement file merely because its name matches.
        let _ = self.cleanup();
    }
}
