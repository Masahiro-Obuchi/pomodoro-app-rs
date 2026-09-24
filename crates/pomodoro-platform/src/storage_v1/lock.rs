use std::{fs, io, path::PathBuf};

#[cfg(windows)]
use fs4::{FileExt, TryLockError};
#[cfg(unix)]
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
#[cfg(windows)]
use std::{
    fs::OpenOptions,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use super::{StorageLocation, StorageLockError};

/// Exclusive access to a local V1 storage directory until this value is dropped.
/// No clone or raw descriptor is exposed. Future load/write handles must retain
/// this value through the final save. Dropping it closes the descriptor but never
/// removes the lock file, whose inode must stay stable across state-file renames.
#[derive(Debug)]
#[must_use = "dropping the storage handle releases its lock"]
pub struct LockedStorage {
    location: StorageLocation,
    _lock: fs::File,
    // The full canonical ancestry, deepest first, for every new handle. Existing
    // entries may belong to an earlier process whose directory sync failed.
    // Atomic save clears this only after a successful durable commit.
    pub(super) directories_to_sync: Vec<PathBuf>,
}

impl LockedStorage {
    #[must_use]
    pub fn location(&self) -> &StorageLocation {
        &self.location
    }
}

impl StorageLocation {
    /// Acquires a nonblocking kernel lock before any state-file access.
    /// Only the directory and `state.lock` may be created here. Directory-entry
    /// durability is part of the later atomic-save boundary, not lock acquisition.
    ///
    /// # Errors
    /// Returns [`StorageLockError::InUse`] if another handle owns the lock, or
    /// [`StorageLockError::Io`] if the directory/lock file cannot be accessed.
    pub fn lock(self) -> Result<LockedStorage, StorageLockError> {
        let directory = self.directory();
        fs::create_dir_all(directory).map_err(|source| StorageLockError::Io {
            path: directory.to_owned(),
            source,
        })?;
        let canonical = fs::canonicalize(directory).map_err(|source| StorageLockError::Io {
            path: directory.to_owned(),
            source,
        })?;
        // No durable marker identifies where a previous process stopped creating
        // or syncing directories. Reconstruct the entire chain on each open,
        // including parents which already existed before this invocation.
        let directories_to_sync = canonical
            .ancestors()
            .map(std::path::Path::to_owned)
            .collect();
        let location = Self::at(canonical);
        let path = location.lock_path();
        let file = open_lock_file(&path).map_err(|source| StorageLockError::Io {
            path: path.clone(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| StorageLockError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(StorageLockError::Io {
                path,
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "storage lock must be a regular file",
                ),
            });
        }
        lock_file(&file, &path)?;
        Ok(LockedStorage {
            location,
            _lock: file,
            directories_to_sync,
        })
    }
}

#[cfg(unix)]
fn open_lock_file(path: &std::path::Path) -> io::Result<fs::File> {
    // CLOEXEC is atomic with open, including for child processes started by
    // another thread. Never truncate or replace this dedicated lock file.
    let fd = open(
        path,
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::RUSR | Mode::WUSR,
    )?;
    Ok(fs::File::from(fd))
}

#[cfg(unix)]
fn lock_file(file: &fs::File, path: &std::path::Path) -> Result<(), StorageLockError> {
    flock(file, FlockOperation::NonBlockingLockExclusive).map_err(|source| {
        if source == rustix::io::Errno::WOULDBLOCK {
            StorageLockError::InUse {
                path: path.to_owned(),
            }
        } else {
            StorageLockError::Io {
                path: path.to_owned(),
                source: source.into(),
            }
        }
    })
}

#[cfg(windows)]
fn open_lock_file(path: &std::path::Path) -> io::Result<fs::File> {
    // Keep the lock pathname stable while the handle is live. Other instances
    // may open it but cannot replace it; the kernel lock serializes access.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage lock must not be a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn lock_file(file: &fs::File, path: &std::path::Path) -> Result<(), StorageLockError> {
    match FileExt::try_lock(file) {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(StorageLockError::InUse {
            path: path.to_owned(),
        }),
        Err(TryLockError::Error(source)) => Err(StorageLockError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}
