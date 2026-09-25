//! Windows file operations used by the shared V1 load/save policy.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use pomodoro_winfs::file_identity;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

use crate::storage_v1::LoadProblem;

// Opening the reparse point itself avoids following a saved-state link. A
// directory, junction, or other special entry never becomes an empty store.
fn open_regular(path: &Path) -> io::Result<Option<File>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return match fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(None),
                Ok(_) => Err(error),
                Err(metadata_error) => Err(metadata_error),
            };
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "saved state must be a regular file without a reparse point",
        ));
    }
    Ok(Some(file))
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, LoadProblem> {
    let Some(mut file) = open_regular(path).map_err(|error| LoadProblem::io(path, error))? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| LoadProblem::io(path, error))?;
    Ok(Some(bytes))
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    // A write-access directory handle is required for FlushFileBuffers. Do not
    // follow a junction while syncing the storage directory or its ancestors.
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage directory must not be a reparse point",
        ));
    }
    file.sync_all()
}

pub(super) fn sync_file(file: &File) -> io::Result<()> {
    file.sync_all()
}

pub(super) fn create_new_private(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

pub(super) fn owns_path(file: &File, path: &Path) -> io::Result<bool> {
    let mut path_file = match open_regular(path) {
        Ok(Some(file)) => file,
        Ok(None) => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    let original = file_identity(file)?;
    let current = file_identity(&path_file)?;
    if original != current {
        return Ok(false);
    }
    // Keep the content check as a guard against unexpected identity behavior
    // before deleting a temp or trusting a recovery archive at the same path.
    let mut original_file = file.try_clone()?;
    original_file.seek(SeekFrom::Start(0))?;
    let mut original_bytes = Vec::new();
    original_file.read_to_end(&mut original_bytes)?;
    let mut current_bytes = Vec::new();
    path_file.read_to_end(&mut current_bytes)?;
    Ok(original_bytes == current_bytes)
}
