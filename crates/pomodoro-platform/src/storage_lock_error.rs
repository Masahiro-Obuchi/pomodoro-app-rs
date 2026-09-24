use std::{error::Error, fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum StorageLockError {
    NoBaseDirectory,
    InUse { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for StorageLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseDirectory => f.write_str("could not determine a user storage directory"),
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
