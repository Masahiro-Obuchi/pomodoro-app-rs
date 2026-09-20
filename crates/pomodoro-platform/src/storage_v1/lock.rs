use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};

use super::StorageLocation;

/// Exclusive access to a local V1 storage directory until this value is dropped.
/// No clone or raw descriptor is exposed. Future load/write handles must retain
/// this value through the final save. Dropping it closes the descriptor but never
/// removes the lock file, whose inode must stay stable across state-file renames.
#[derive(Debug)]
#[must_use = "dropping the storage handle releases its lock"]
pub struct LockedStorage {
    location: StorageLocation,
    _lock: fs::File,
    // New directories and their containing directory, deepest first. Atomic
    // save must sync these before it can report the first durable commit.
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
        let directories_to_sync =
            create_directory(directory).map_err(|source| StorageLockError::Io {
                path: directory.to_owned(),
                source,
            })?;
        let canonical = fs::canonicalize(directory).map_err(|source| StorageLockError::Io {
            path: directory.to_owned(),
            source,
        })?;
        let location = Self::at(canonical);
        let path = location.lock_path();
        // CLOEXEC is set atomically with open, including for notification commands
        // spawned by other threads. Never truncate or replace the lock file.
        let fd = open(
            &path,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|source| StorageLockError::Io {
            path: path.clone(),
            source: source.into(),
        })?;
        let file = fs::File::from(fd);
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
        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|source| {
            if source == rustix::io::Errno::WOULDBLOCK {
                StorageLockError::InUse { path: path.clone() }
            } else {
                StorageLockError::Io {
                    path: path.clone(),
                    source: source.into(),
                }
            }
        })?;
        Ok(LockedStorage {
            location,
            _lock: file,
            directories_to_sync,
        })
    }
}

fn create_directory(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut cursor = std::path::absolute(path)?;
    let mut missing = Vec::new();
    loop {
        match fs::metadata(&cursor) {
            Ok(metadata) if metadata.is_dir() => break,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "storage path is not a directory",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor.clone());
                if !cursor.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    fs::create_dir_all(path)?;
    // The last existing ancestor owns the outermost newly created entry.
    missing.push(cursor);
    let mut directories = Vec::new();
    for path in missing {
        let canonical = fs::canonicalize(path)?;
        if !directories.contains(&canonical) {
            directories.push(canonical);
        }
    }
    Ok(directories)
}

#[derive(Debug)]
pub enum StorageLockError {
    NoBaseDirectory,
    InUse { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for StorageLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseDirectory => f.write_str("could not determine a user state directory"),
            Self::InUse { path } => write!(f, "storage is already in use: {}", path.display()),
            Self::Io { path, source } => {
                write!(f, "storage lock I/O failed at {}: {source}", path.display())
            }
        }
    }
}

impl Error for StorageLockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
