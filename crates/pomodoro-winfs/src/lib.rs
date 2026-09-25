//! The single audited Win32 FFI boundary for file identity.
//!
//! The platform crate keeps `unsafe_code = "forbid"`. Windows does not expose
//! its 128-bit `FILE_ID_INFO` through a safe standard-library API on Rust 1.86.
#![deny(unsafe_code)]

#[cfg(windows)]
mod windows {
    use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    /// Identity of an open file on one volume while both compared handles live.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct FileIdentity {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }

    /// Obtain the full file ID from an already-open handle. An unsupported or
    /// failed query is an error; it must never fall back to ReFS's non-unique
    /// 64-bit file index when deciding whether a path may be deleted.
    #[allow(unsafe_code)]
    pub fn file_identity(file: &File) -> io::Result<FileIdentity> {
        let mut info = FILE_ID_INFO::default();
        // SAFETY: `file` owns a live handle for the call. `info` is a writable
        // FILE_ID_INFO of the exact size supplied to the Windows API.
        let succeeded = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&raw mut info).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(FileIdentity {
            volume_serial_number: info.VolumeSerialNumber,
            file_id: info.FileId.Identifier,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::FileIdentity;

        #[test]
        fn full_file_id_distinguishes_equal_low_64_bits() {
            let first = FileIdentity {
                volume_serial_number: 7,
                file_id: [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            };
            let second = FileIdentity {
                volume_serial_number: 7,
                file_id: [0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0],
            };
            assert_ne!(first, second);
        }
    }
}

#[cfg(windows)]
pub use windows::{FileIdentity, file_identity};
