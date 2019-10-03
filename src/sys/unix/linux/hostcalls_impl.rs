use super::super::dir::{Dir, Entry, SeekLoc};
use super::osfile::OsFile;
use crate::hostcalls_impl::{Dirent, PathGet};
use crate::sys::host_impl;
use crate::sys::unix::str_to_cstring;
use crate::{host, Error, Result};
use log::{debug, trace};
use std::convert::TryInto;
use std::fs::File;
use std::os::unix::prelude::AsRawFd;

pub(crate) fn path_unlink_file(resolved: PathGet) -> Result<()> {
    use nix::errno;
    use nix::libc::unlinkat;

    let path_cstr = str_to_cstring(resolved.path())?;

    // nix doesn't expose unlinkat() yet
    let res = unsafe { unlinkat(resolved.dirfd().as_raw_fd(), path_cstr.as_ptr(), 0) };
    if res == 0 {
        Ok(())
    } else {
        Err(host_impl::errno_from_nix(errno::Errno::last()))
    }
}

pub(crate) fn path_symlink(old_path: &str, resolved: PathGet) -> Result<()> {
    use nix::{errno::Errno, libc::symlinkat};

    let old_path_cstr = str_to_cstring(old_path)?;
    let new_path_cstr = str_to_cstring(resolved.path())?;

    log::debug!("path_symlink old_path = {:?}", old_path);
    log::debug!("path_symlink resolved = {:?}", resolved);

    let res = unsafe {
        symlinkat(
            old_path_cstr.as_ptr(),
            resolved.dirfd().as_raw_fd(),
            new_path_cstr.as_ptr(),
        )
    };
    if res != 0 {
        Err(host_impl::errno_from_nix(Errno::last()))
    } else {
        Ok(())
    }
}

pub(crate) fn path_rename(resolved_old: PathGet, resolved_new: PathGet) -> Result<()> {
    use nix::libc::renameat;
    let old_path_cstr = str_to_cstring(resolved_old.path())?;
    let new_path_cstr = str_to_cstring(resolved_new.path())?;

    let res = unsafe {
        renameat(
            resolved_old.dirfd().as_raw_fd(),
            old_path_cstr.as_ptr(),
            resolved_new.dirfd().as_raw_fd(),
            new_path_cstr.as_ptr(),
        )
    };
    if res != 0 {
        Err(host_impl::errno_from_nix(nix::errno::Errno::last()))
    } else {
        Ok(())
    }
}

pub(crate) fn fd_readdir_impl(fd: &File, cookie: host::__wasi_dircookie_t) -> Result<Vec<Dirent>> {
    // We need to duplicate the fd, because `opendir(3)`:
    //     After a successful call to fdopendir(), fd is used internally by the implementation,
    //     and should not otherwise be used by the application.
    // `opendir(3p)` also says that it's undefined behavior to
    // modify the state of the fd in a different way than by accessing DIR*.
    //
    // Still, rewinddir will be needed because the two file descriptors
    // share progress. But we can safely execute closedir now.
    let fd = fd.try_clone()?;
    let mut dir = Dir::from(fd)?;

    // Seek if needed. Unless cookie is host::__WASI_DIRCOOKIE_START,
    // new items may not be returned to the caller.
    //
    // According to `opendir(3p)`:
    //     If a file is removed from or added to the directory after the most recent call
    //     to opendir() or rewinddir(), whether a subsequent call to readdir() returns an entry
    //     for that file is unspecified.
    if cookie == host::__WASI_DIRCOOKIE_START {
        trace!("     | fd_readdir: doing rewinddir");
        dir.rewind();
    } else {
        trace!("     | fd_readdir: doing seekdir to {}", cookie);
        let loc = unsafe { SeekLoc::from_raw(cookie as i64) };
        dir.seek(loc);
    }

    dir.iter()
        .map(|dir| {
            let dir: Entry = dir?;
            Ok(Dirent {
                name: dir // TODO can we reuse path_from_host for CStr?
                    .file_name()
                    .to_str()?
                    .to_owned(),
                ino: dir.ino(),
                ftype: dir.file_type().into(),
                cookie: dir.seek_loc().to_raw().try_into()?,
            })
        })
        .collect()
}

// This should actually be common code with Windows,
// but there's BSD stuff remaining
pub(crate) fn fd_readdir(
    os_file: &mut OsFile,
    mut host_buf: &mut [u8],
    cookie: host::__wasi_dircookie_t,
) -> Result<usize> {
    let vec = fd_readdir_impl(os_file, cookie)?;
    debug!("Received dirents: {:?}", vec);
    let mut used = 0;
    for dirent in vec {
        let dirent_raw = dirent.to_raw()?;
        let offset = dirent_raw.len();
        host_buf[0..offset].copy_from_slice(&dirent_raw);
        used += offset;
        host_buf = &mut host_buf[offset..];
    }

    trace!("     | *buf_used={:?}", used);
    Ok(used)
}

pub(crate) fn fd_advise(
    file: &File,
    advice: host::__wasi_advice_t,
    offset: host::__wasi_filesize_t,
    len: host::__wasi_filesize_t,
) -> Result<()> {
    {
        use nix::fcntl::{posix_fadvise, PosixFadviseAdvice};

        let offset = offset.try_into()?;
        let len = len.try_into()?;
        let host_advice = match advice {
            host::__WASI_ADVICE_DONTNEED => PosixFadviseAdvice::POSIX_FADV_DONTNEED,
            host::__WASI_ADVICE_SEQUENTIAL => PosixFadviseAdvice::POSIX_FADV_SEQUENTIAL,
            host::__WASI_ADVICE_WILLNEED => PosixFadviseAdvice::POSIX_FADV_WILLNEED,
            host::__WASI_ADVICE_NOREUSE => PosixFadviseAdvice::POSIX_FADV_NOREUSE,
            host::__WASI_ADVICE_RANDOM => PosixFadviseAdvice::POSIX_FADV_RANDOM,
            host::__WASI_ADVICE_NORMAL => PosixFadviseAdvice::POSIX_FADV_NORMAL,
            _ => return Err(Error::EINVAL),
        };

        posix_fadvise(file.as_raw_fd(), offset, len, host_advice)?;
    }

    Ok(())
}
