use crate::fdentry::{Descriptor, FdFlags};
use crate::{host, Error, Result};
use std::fs::File;
use std::io;
use std::ops::{Deref, DerefMut};
use std::os::windows::prelude::{AsRawHandle, FromRawHandle, RawHandle};

#[derive(Debug)]
pub(crate) struct OsFile(File, FdFlags);

impl OsFile {
    pub(crate) fn new(file: File, flags: FdFlags) -> Self {
        Self(file, flags)
    }
}

impl AsRawHandle for OsFile {
    fn as_raw_handle(&self) -> RawHandle {
        self.0.as_raw_handle()
    }
}

impl Deref for OsFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for OsFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRawHandle for Descriptor {
    fn as_raw_handle(&self) -> RawHandle {
        match self {
            Self::OsFile(file) => file.as_raw_handle(),
            Self::Stdin => io::stdin().as_raw_handle(),
            Self::Stdout => io::stdout().as_raw_handle(),
            Self::Stderr => io::stderr().as_raw_handle(),
        }
    }
}

/// This function is unsafe because it operates on a raw file handle.
pub(crate) unsafe fn determine_type_and_access_rights<Handle: AsRawHandle>(
    handle: &Handle,
) -> Result<(
    host::__wasi_filetype_t,
    host::__wasi_rights_t,
    host::__wasi_rights_t,
)> {
    use winx::file::{get_file_access_mode, AccessMode};

    let (file_type, mut rights_base, rights_inheriting) = determine_type_rights(handle)?;

    match file_type {
        host::__WASI_FILETYPE_DIRECTORY | host::__WASI_FILETYPE_REGULAR_FILE => {
            let mode = get_file_access_mode(handle.as_raw_handle())?;
            if mode.contains(AccessMode::FILE_GENERIC_READ) {
                rights_base |= host::__WASI_RIGHT_FD_READ;
            }
            if mode.contains(AccessMode::FILE_GENERIC_WRITE) {
                rights_base |= host::__WASI_RIGHT_FD_WRITE;
            }
        }
        _ => {
            // TODO: is there a way around this? On windows, it seems
            // we cannot check access rights for anything but dirs and regular files
        }
    }

    Ok((file_type, rights_base, rights_inheriting))
}

/// This function is unsafe because it operates on a raw file handle.
pub(crate) unsafe fn determine_type_rights<Handle: AsRawHandle>(
    handle: &Handle,
) -> Result<(
    host::__wasi_filetype_t,
    host::__wasi_rights_t,
    host::__wasi_rights_t,
)> {
    let (file_type, rights_base, rights_inheriting) = {
        let file_type = winx::file::get_file_type(handle.as_raw_handle())?;
        if file_type.is_char() {
            // character file: LPT device or console
            // TODO: rule out LPT device
            (
                host::__WASI_FILETYPE_CHARACTER_DEVICE,
                host::RIGHTS_TTY_BASE,
                host::RIGHTS_TTY_BASE,
            )
        } else if file_type.is_disk() {
            // disk file: file, dir or disk device
            let file = std::mem::ManuallyDrop::new(File::from_raw_handle(handle.as_raw_handle()));
            let meta = file.metadata().map_err(|_| Error::EINVAL)?;
            if meta.is_dir() {
                (
                    host::__WASI_FILETYPE_DIRECTORY,
                    host::RIGHTS_DIRECTORY_BASE,
                    host::RIGHTS_DIRECTORY_INHERITING,
                )
            } else if meta.is_file() {
                (
                    host::__WASI_FILETYPE_REGULAR_FILE,
                    host::RIGHTS_REGULAR_FILE_BASE,
                    host::RIGHTS_REGULAR_FILE_INHERITING,
                )
            } else {
                return Err(Error::EINVAL);
            }
        } else if file_type.is_pipe() {
            // pipe object: socket, named pipe or anonymous pipe
            // TODO: what about pipes, etc?
            (
                host::__WASI_FILETYPE_SOCKET_STREAM,
                host::RIGHTS_SOCKET_BASE,
                host::RIGHTS_SOCKET_INHERITING,
            )
        } else {
            return Err(Error::EINVAL);
        }
    };
    Ok((file_type, rights_base, rights_inheriting))
}
