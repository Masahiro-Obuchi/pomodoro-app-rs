//! Unix file operations used by the V1 load/save policy.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use rustix::fs::{Mode, OFlags, open};

use crate::storage_v1::LoadProblem;

// Never use Path::exists or a blanket NotFound fallback through symlinks. A
// dangling link is not an empty store, and a FIFO must not hang application startup.
pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, LoadProblem> {
    let fd = match open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(LoadProblem::io(path, error.into())),
    };
    let mut file = File::from(fd);
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

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    sync_file(&File::from(fd))
}

pub(super) fn sync_file(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // fsync alone does not request a flush of the drive's volatile cache.
        rustix::fs::fcntl_fullfsync(file).map_err(Into::into)
    }
    #[cfg(target_os = "linux")]
    {
        file.sync_all()
    }
}

pub(super) fn create_new_private(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

pub(super) fn owns_path(file: &File, path: &Path) -> io::Result<bool> {
    let actual = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let expected = file.metadata()?;
    Ok(actual.is_file() && actual.dev() == expected.dev() && actual.ino() == expected.ino())
}
