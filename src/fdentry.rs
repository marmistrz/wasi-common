use super::host;
use crate::sys::{errno_from_host, fdentry_impl};

use std::fs;
use std::io;
use std::mem::ManuallyDrop;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Descriptor {
    File(fs::File),
    Stdin,
    Stdout,
    Stderr,
}

#[derive(Debug)]
pub struct FdObject {
    pub file_type: host::__wasi_filetype_t,
    pub descriptor: ManuallyDrop<Descriptor>,
    pub needs_close: bool,
    // TODO: directories
}

#[derive(Debug)]
pub struct FdEntry {
    pub fd_object: FdObject,
    pub rights_base: host::__wasi_rights_t,
    pub rights_inheriting: host::__wasi_rights_t,
    pub preopen_path: Option<PathBuf>,
}

impl Drop for FdObject {
    fn drop(&mut self) {
        if self.needs_close {
            unsafe { ManuallyDrop::drop(&mut self.descriptor) };
        }
    }
}

impl FdEntry {
    pub fn from(file: fs::File) -> Result<Self, host::__wasi_errno_t> {
        fdentry_impl::determine_type_and_access_rights(&file).map(
            |(file_type, rights_base, rights_inheriting)| Self {
                fd_object: FdObject {
                    file_type,
                    descriptor: ManuallyDrop::new(Descriptor::File(file)),
                    needs_close: true,
                },
                rights_base,
                rights_inheriting,
                preopen_path: None,
            },
        )
    }

    pub fn duplicate(file: &fs::File) -> Result<Self, host::__wasi_errno_t> {
        file.try_clone()
            .map_err(|err| err.raw_os_error().map_or(host::__WASI_EIO, errno_from_host))
            .and_then(Self::from)
    }

    pub fn duplicate_stdin() -> Result<Self, host::__wasi_errno_t> {
        fdentry_impl::determine_type_and_access_rights(&io::stdin()).map(
            |(file_type, rights_base, rights_inheriting)| Self {
                fd_object: FdObject {
                    file_type,
                    descriptor: ManuallyDrop::new(Descriptor::Stdin),
                    needs_close: true,
                },
                rights_base,
                rights_inheriting,
                preopen_path: None,
            },
        )
    }

    pub fn duplicate_stdout() -> Result<Self, host::__wasi_errno_t> {
        fdentry_impl::determine_type_and_access_rights(&io::stdout()).map(
            |(file_type, rights_base, rights_inheriting)| Self {
                fd_object: FdObject {
                    file_type,
                    descriptor: ManuallyDrop::new(Descriptor::Stdout),
                    needs_close: true,
                },
                rights_base,
                rights_inheriting,
                preopen_path: None,
            },
        )
    }

    pub fn duplicate_stderr() -> Result<Self, host::__wasi_errno_t> {
        fdentry_impl::determine_type_and_access_rights(&io::stderr()).map(
            |(file_type, rights_base, rights_inheriting)| Self {
                fd_object: FdObject {
                    file_type,
                    descriptor: ManuallyDrop::new(Descriptor::Stderr),
                    needs_close: true,
                },
                rights_base,
                rights_inheriting,
                preopen_path: None,
            },
        )
    }
}
