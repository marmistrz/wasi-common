#![allow(non_camel_case_types)]
#![allow(unused_unsafe)]
use crate::sys::host_impl;
use crate::{host, Result};
use nix::libc::{self, c_long};
use std::fs::File;

pub(crate) fn path_open_rights(
    rights_base: host::__wasi_rights_t,
    rights_inheriting: host::__wasi_rights_t,
    oflags: host::__wasi_oflags_t,
    fs_flags: host::__wasi_fdflags_t,
) -> (host::__wasi_rights_t, host::__wasi_rights_t) {
    use nix::fcntl::OFlag;

    // which rights are needed on the dirfd?
    let mut needed_base = host::__WASI_RIGHT_PATH_OPEN;
    let mut needed_inheriting = rights_base | rights_inheriting;

    // convert open flags
    let oflags = host_impl::nix_from_oflags(oflags);
    if oflags.contains(OFlag::O_CREAT) {
        needed_base |= host::__WASI_RIGHT_PATH_CREATE_FILE;
    }
    if oflags.contains(OFlag::O_TRUNC) {
        needed_base |= host::__WASI_RIGHT_PATH_FILESTAT_SET_SIZE;
    }

    // convert file descriptor flags
    let fdflags = host_impl::nix_from_fdflags(fs_flags);
    if fdflags.contains(OFlag::O_DSYNC) {
        needed_inheriting |= host::__WASI_RIGHT_FD_DATASYNC;
    }
    if fdflags.intersects(host_impl::O_RSYNC | OFlag::O_SYNC) {
        needed_inheriting |= host::__WASI_RIGHT_FD_SYNC;
    }

    (needed_base, needed_inheriting)
}

pub(crate) fn openat(dirfd: &File, path: &str) -> Result<File> {
    use nix::fcntl::{self, OFlag};
    use nix::sys::stat::Mode;
    use std::os::unix::prelude::{AsRawFd, FromRawFd};

    fcntl::openat(
        dirfd.as_raw_fd(),
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(|new_fd| unsafe { File::from_raw_fd(new_fd) })
    .map_err(|e| host_impl::errno_from_nix(e.as_errno().unwrap()))
}

pub(crate) fn readlinkat(dirfd: &File, path: &str) -> Result<String> {
    use nix::fcntl;
    use std::os::unix::prelude::AsRawFd;

    fcntl::readlinkat(dirfd.as_raw_fd(), path)
        .map_err(|e| host_impl::errno_from_nix(e.as_errno().unwrap()))
        .and_then(host_impl::path_from_host)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn utime_now() -> c_long {
    libc::UTIME_NOW
}

#[cfg(target_os = "macos")]
pub(crate) fn utime_now() -> c_long {
    -1
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn utime_omit() -> c_long {
    libc::UTIME_OMIT
}

#[cfg(target_os = "macos")]
pub(crate) fn utime_omit() -> c_long {
    -2
}
