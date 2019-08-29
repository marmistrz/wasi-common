#![allow(non_camel_case_types)]
#![allow(unused)]
use super::fs_helpers::*;
use crate::ctx::WasiCtx;
use crate::fdentry::FdEntry;
use crate::helpers::systemtime_to_timestamp;
use crate::hostcalls_impl::{fd_filestat_set_times_impl, PathGet};
use crate::hostcalls_impl::{Dirent, FileType};
use crate::sys::fdentry_impl::determine_type_rights;
use crate::sys::host_impl::{self, path_from_host};
use crate::sys::hostcalls_impl::fs_helpers::PathGetExt;
use crate::sys::{errno_from_host, errno_from_ioerror};
use crate::{host, Result};
use log::{debug, trace};
use std::convert::TryInto;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::mem;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::os::windows::prelude::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::slice;
use winx::file::{AccessMode, Flags};

fn read_at(mut file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    // get current cursor position
    let cur_pos = file.seek(SeekFrom::Current(0))?;
    // perform a seek read by a specified offset
    let nread = file.seek_read(buf, offset)?;
    // rewind the cursor back to the original position
    file.seek(SeekFrom::Start(cur_pos))?;
    Ok(nread)
}

fn write_at(mut file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    // get current cursor position
    let cur_pos = file.seek(SeekFrom::Current(0))?;
    // perform a seek write by a specified offset
    let nwritten = file.seek_write(buf, offset)?;
    // rewind the cursor back to the original position
    file.seek(SeekFrom::Start(cur_pos))?;
    Ok(nwritten)
}

pub(crate) fn fd_pread(
    file: &File,
    buf: &mut [u8],
    offset: host::__wasi_filesize_t,
) -> Result<usize> {
    read_at(file, buf, offset).map_err(errno_from_ioerror)
}

pub(crate) fn fd_pwrite(file: &File, buf: &[u8], offset: host::__wasi_filesize_t) -> Result<usize> {
    write_at(file, buf, offset).map_err(errno_from_ioerror)
}

pub(crate) fn fd_fdstat_get(fd: &File) -> Result<host::__wasi_fdflags_t> {
    winx::file::get_file_access_mode(fd.as_raw_handle())
        .map(host_impl::fdflags_from_win)
        .map_err(host_impl::errno_from_win)
}

pub(crate) fn fd_fdstat_set_flags(fd: &File, fdflags: host::__wasi_fdflags_t) -> Result<()> {
    unimplemented!("fd_fdstat_set_flags")
}

pub(crate) fn fd_advise(
    file: &File,
    advice: host::__wasi_advice_t,
    offset: host::__wasi_filesize_t,
    len: host::__wasi_filesize_t,
) -> Result<()> {
    unimplemented!("fd_advise")
}

pub(crate) fn path_create_directory(resolved: PathGet) -> Result<()> {
    let path = resolved.concatenate()?;
    std::fs::create_dir(&path).map_err(errno_from_ioerror)
}

pub(crate) fn path_link(resolved_old: PathGet, resolved_new: PathGet) -> Result<()> {
    unimplemented!("path_link")
}

pub(crate) fn path_open(
    resolved: PathGet,
    read: bool,
    write: bool,
    oflags: host::__wasi_oflags_t,
    fdflags: host::__wasi_fdflags_t,
) -> Result<File> {
    use winx::file::{AccessMode, CreationDisposition, Flags};

    let mut access_mode = AccessMode::READ_CONTROL;
    if read {
        access_mode.insert(AccessMode::FILE_GENERIC_READ);
    }
    if write {
        access_mode.insert(AccessMode::FILE_GENERIC_WRITE);
    }

    let mut flags = Flags::FILE_FLAG_BACKUP_SEMANTICS;

    // convert open flags
    let mut opts = OpenOptions::new();
    match host_impl::win_from_oflags(oflags) {
        CreationDisposition::CREATE_ALWAYS => {
            opts.create(true).append(true);
        }
        CreationDisposition::CREATE_NEW => {
            opts.create_new(true).write(true);
        }
        CreationDisposition::TRUNCATE_EXISTING => {
            opts.truncate(true);
        }
        _ => {}
    }

    // convert file descriptor flags
    let (add_access_mode, add_flags) = host_impl::win_from_fdflags(fdflags);
    access_mode.insert(add_access_mode);
    flags.insert(add_flags);

    let path = resolved.concatenate()?;

    match path.symlink_metadata().map(|metadata| metadata.file_type()) {
        Ok(file_type) => {
            // check if we are trying to open a symlink
            if file_type.is_symlink() {
                return Err(host::__WASI_ELOOP);
            }
            // check if we are trying to open a file as a dir
            if file_type.is_file() && oflags & host::__WASI_O_DIRECTORY != 0 {
                return Err(host::__WASI_ENOTDIR);
            }
        }
        Err(e) => match e.raw_os_error() {
            Some(e) => {
                use winx::winerror::WinError;
                log::debug!("path_open at symlink_metadata error code={:?}", e);
                let e = WinError::from_u32(e as u32);

                if e != WinError::ERROR_FILE_NOT_FOUND {
                    return Err(host_impl::errno_from_win(e));
                }
                // file not found, let it proceed to actually
                // trying to open it
            }
            None => {
                log::debug!("Inconvertible OS error: {}", e);
                return Err(host::__WASI_EIO);
            }
        },
    }

    opts.access_mode(access_mode.bits())
        .custom_flags(flags.bits())
        .open(&path)
        .map_err(errno_from_ioerror)
}

fn dirent_from_path<P: AsRef<Path>>(path: P, cookie: host::__wasi_dircookie_t) -> Result<Dirent> {
    let path = path.as_ref();
    trace!("dirent_from_path: opening {}", path.to_string_lossy());

    // To open a directory on Windows, FILE_FLAG_BACKUP_SEMANTICS flag must be used
    let file = OpenOptions::new()
        .custom_flags(Flags::FILE_FLAG_BACKUP_SEMANTICS.bits())
        .read(true)
        .open(path)
        .map_err(errno_from_ioerror)?;
    let ty = file.metadata().map_err(errno_from_ioerror)?.file_type();
    Ok(Dirent {
        ftype: filetype_from_std(&ty),
        name: path_from_host(path.file_name().expect("dirent_from_path: invalid path"))?,
        cookie,
        ino: file_serial_no(&file).map_err(errno_from_ioerror)?,
    })
}

pub(crate) fn fd_readdir(fd: &File, cookie: host::__wasi_dircookie_t) -> Result<Vec<Dirent>> {
    use winx::file::get_path_by_handle;

    // TODO document caveats and the order assumptions
    let cookie = cookie.try_into().map_err(|_| host::__WASI_EOVERFLOW)?;
    let path = get_path_by_handle(fd.as_raw_handle()).map_err(host_impl::errno_from_win)?;
    // std::fs::ReadDir doesn't return . and .., so we need to emulate it
    let path = Path::new(&path);
    // The directory /.. is the same as / on Unix, so emulate this behavior too
    let parent = path.parent().unwrap_or(path);
    trace!("    | fd_readdir impl: emulating .");
    let dot = dirent_from_path(path, 1)?;
    trace!("    | fd_readdir impl: emulating ..");
    let dotdot = dirent_from_path(parent, 2)?;
    trace!("    | fd_readdir impl: executing std::fs::ReadDir");
    let iter = path
        .read_dir()
        .map_err(errno_from_ioerror)?
        .zip(3..)
        .map(|(dir, no)| {
            let dir: std::fs::DirEntry = dir.map_err(errno_from_ioerror)?;

            Ok(Dirent {
                name: path_from_host(dir.file_name())?,
                ftype: filetype_from_std(&dir.file_type().map_err(errno_from_ioerror)?),
                ino: File::open(dir.path())
                    .and_then(|f| file_serial_no(&f))
                    .map_err(errno_from_ioerror)?,
                cookie: no,
            })
        });

    // into_iter for arrays is broken and returns references instead of values,
    // so we need to use a Vec
    let iter = vec![dot, dotdot].into_iter().map(Ok).chain(iter);

    // Emulate seekdir(). This may give O(n^2) TODO explain why
    // TODO explain why it's the least evil
    iter.skip(cookie).collect() //fixme cast
}

pub(crate) fn path_readlink(resolved: PathGet, buf: &mut [u8]) -> Result<usize> {
    use winx::file::get_path_by_handle;

    let path = resolved.concatenate()?;
    let target_path = path.read_link().map_err(errno_from_ioerror)?;

    // since on Windows we are effectively emulating 'at' syscalls
    // we need to strip the prefix from the absolute path
    // as otherwise we will error out since WASI is not capable
    // of dealing with absolute paths
    let dir_path =
        get_path_by_handle(resolved.dirfd().as_raw_handle()).map_err(host_impl::errno_from_win)?;
    let dir_path = PathBuf::from(strip_extended_prefix(dir_path));
    let target_path = target_path
        .strip_prefix(dir_path)
        .map_err(|_| host::__WASI_ENOTCAPABLE)
        .and_then(|path| path.to_str().map(String::from).ok_or(host::__WASI_EILSEQ))?;

    if buf.len() > 0 {
        let mut chars = target_path.chars();
        let mut nread = 0usize;

        for i in 0..buf.len() {
            match chars.next() {
                Some(ch) => {
                    buf[i] = ch as u8;
                    nread += 1;
                }
                None => break,
            }
        }

        Ok(nread)
    } else {
        Ok(0)
    }
}

pub(crate) fn path_rename(resolved_old: PathGet, resolved_new: PathGet) -> Result<()> {
    use std::fs;

    let old_path = resolved_old.concatenate()?;
    let new_path = resolved_new.concatenate()?;

    // First, sanity check that we're not trying to rename dir to file or vice versa.
    // NB on Windows, the former is actually permitted [std::fs::rename].
    //
    // [std::fs::rename]: https://doc.rust-lang.org/std/fs/fn.rename.html
    if old_path.is_dir() && new_path.is_file() {
        return Err(host::__WASI_ENOTDIR);
    }

    // TODO handle symlinks

    fs::rename(&old_path, &new_path).or_else(|e| match e.raw_os_error() {
        Some(e) => {
            use winx::winerror::WinError;

            log::debug!("path_rename at rename error code={:?}", e);
            match WinError::from_u32(e as u32) {
                WinError::ERROR_ACCESS_DENIED => {
                    // So most likely dealing with new_path == dir.
                    // Eliminate case old_path == file first.
                    if old_path.is_file() {
                        Err(host::__WASI_EISDIR)
                    } else {
                        // Ok, let's try removing an empty dir at new_path if it exists
                        // and is a nonempty dir.
                        fs::remove_dir(&new_path)
                            .and_then(|()| fs::rename(old_path, new_path))
                            .map_err(errno_from_ioerror)
                    }
                }
                e => Err(host_impl::errno_from_win(e)),
            }
        }
        None => {
            log::debug!("Inconvertible OS error: {}", e);
            Err(host::__WASI_EIO)
        }
    })
}

pub(crate) fn num_hardlinks(file: &File) -> io::Result<u64> {
    Ok(winx::file::get_fileinfo(file)?.nNumberOfLinks.into())
}

pub(crate) fn device_id(file: &File) -> io::Result<u64> {
    Ok(winx::file::get_fileinfo(file)?.dwVolumeSerialNumber.into())
}

pub(crate) fn file_serial_no(file: &File) -> io::Result<u64> {
    let info = winx::file::get_fileinfo(file)?;
    let high = info.nFileIndexHigh;
    let low = info.nFileIndexLow;
    let no = ((high as u64) << 32) | (low as u64);
    Ok(no)
}

pub(crate) fn change_time(file: &File, _metadata: &Metadata) -> io::Result<i64> {
    winx::file::change_time(file)
}

pub(crate) fn fd_filestat_get_impl(file: &std::fs::File) -> Result<host::__wasi_filestat_t> {
    let metadata = file.metadata().map_err(errno_from_ioerror)?;
    Ok(host::__wasi_filestat_t {
        st_dev: device_id(file).map_err(errno_from_ioerror)?,
        st_ino: file_serial_no(file).map_err(errno_from_ioerror)?,
        st_nlink: num_hardlinks(file)
            .map_err(errno_from_ioerror)?
            .try_into()
            .map_err(|_| host::__WASI_EOVERFLOW)?, // u64 doesn't fit into u32
        st_size: metadata.len(),
        st_atim: metadata
            .accessed()
            .map_err(errno_from_ioerror)
            .and_then(systemtime_to_timestamp)?,
        st_ctim: change_time(file, &metadata)
            .map_err(errno_from_ioerror)?
            .try_into()
            .map_err(|_| host::__WASI_EOVERFLOW)?, // i64 doesn't fit into u64
        st_mtim: metadata
            .modified()
            .map_err(errno_from_ioerror)
            .and_then(systemtime_to_timestamp)?,
        st_filetype: filetype_from_std(&metadata.file_type()).to_wasi(),
    })
}

pub(crate) fn filetype_from_std(ftype: &std::fs::FileType) -> FileType {
    if ftype.is_file() {
        FileType::RegularFile
    } else if ftype.is_dir() {
        FileType::Directory
    } else if ftype.is_symlink() {
        FileType::Symlink
    } else {
        FileType::Unknown
    }
}

pub(crate) fn path_filestat_get(
    resolved: PathGet,
    dirflags: host::__wasi_lookupflags_t,
) -> Result<host::__wasi_filestat_t> {
    let path = resolved.concatenate()?;
    let file = File::open(path).map_err(errno_from_ioerror)?;
    fd_filestat_get_impl(&file)
}

pub(crate) fn path_filestat_set_times(
    resolved: PathGet,
    dirflags: host::__wasi_lookupflags_t,
    st_atim: host::__wasi_timestamp_t,
    mut st_mtim: host::__wasi_timestamp_t,
    fst_flags: host::__wasi_fstflags_t,
) -> Result<()> {
    let path = resolved.concatenate()?;
    let file = OpenOptions::new()
        .access_mode(AccessMode::FILE_WRITE_ATTRIBUTES.bits())
        .open(path)
        .map_err(errno_from_ioerror)?;
    fd_filestat_set_times_impl(&file, st_atim, st_mtim, fst_flags)
}

pub(crate) fn path_symlink(old_path: &str, resolved: PathGet) -> Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    use winx::winerror::WinError;

    let old_path = concatenate(resolved.dirfd(), Path::new(old_path))?;
    let new_path = resolved.concatenate()?;

    // try creating a file symlink
    symlink_file(&old_path, &new_path).or_else(|e| {
        match e.raw_os_error() {
            Some(e) => {
                log::debug!("path_symlink at symlink_file error code={:?}", e);
                match WinError::from_u32(e as u32) {
                    WinError::ERROR_NOT_A_REPARSE_POINT => {
                        // try creating a dir symlink instead
                        symlink_dir(old_path, new_path).map_err(errno_from_ioerror)
                    }
                    e => Err(host_impl::errno_from_win(e)),
                }
            }
            None => {
                log::debug!("Inconvertible OS error: {}", e);
                Err(host::__WASI_EIO)
            }
        }
    })
}

pub(crate) fn path_unlink_file(resolved: PathGet) -> Result<()> {
    use std::fs;
    use winx::winerror::WinError;

    let path = resolved.concatenate()?;
    let file_type = path
        .symlink_metadata()
        .map(|metadata| metadata.file_type())
        .map_err(errno_from_ioerror)?;

    // check if we're unlinking a symlink
    // NB this will get cleaned up a lot when [std::os::windows::fs::FileTypeExt]
    // stabilises
    //
    // [std::os::windows::fs::FileTypeExt]: https://doc.rust-lang.org/std/os/windows/fs/trait.FileTypeExt.html
    if file_type.is_symlink() {
        fs::remove_file(&path).or_else(|e| {
            match e.raw_os_error() {
                Some(e) => {
                    log::debug!("path_unlink_file at symlink_file error code={:?}", e);
                    match WinError::from_u32(e as u32) {
                        WinError::ERROR_ACCESS_DENIED => {
                            // try unlinking a dir symlink instead
                            fs::remove_dir(path).map_err(errno_from_ioerror)
                        }
                        e => Err(host_impl::errno_from_win(e)),
                    }
                }
                None => {
                    log::debug!("Inconvertible OS error: {}", e);
                    Err(host::__WASI_EIO)
                }
            }
        })
    } else if file_type.is_dir() {
        Err(host::__WASI_EISDIR)
    } else if file_type.is_file() {
        fs::remove_file(path).map_err(errno_from_ioerror)
    } else {
        Err(host::__WASI_EINVAL)
    }
}

pub(crate) fn path_remove_directory(resolved: PathGet) -> Result<()> {
    let path = resolved.concatenate()?;
    std::fs::remove_dir(&path).map_err(errno_from_ioerror)
}
