#![allow(non_camel_case_types)]
#![allow(unused_unsafe)]
use crate::hostcalls_impl::{ClockEventData, FdEventData};
use crate::sys::host_impl;
use crate::{host, wasm32, Error, Result};
use nix::libc::{self, c_int};
use std::convert::TryInto;
use std::mem::MaybeUninit;

pub(crate) fn clock_res_get(clock_id: host::__wasi_clockid_t) -> Result<host::__wasi_timestamp_t> {
    // convert the supported clocks to the libc types, or return EINVAL
    let clock_id = match clock_id {
        host::__WASI_CLOCK_REALTIME => libc::CLOCK_REALTIME,
        host::__WASI_CLOCK_MONOTONIC => libc::CLOCK_MONOTONIC,
        host::__WASI_CLOCK_PROCESS_CPUTIME_ID => libc::CLOCK_PROCESS_CPUTIME_ID,
        host::__WASI_CLOCK_THREAD_CPUTIME_ID => libc::CLOCK_THREAD_CPUTIME_ID,
        _ => return Err(Error::EINVAL),
    };

    // no `nix` wrapper for clock_getres, so we do it ourselves
    let mut timespec = MaybeUninit::<libc::timespec>::uninit();
    let res = unsafe { libc::clock_getres(clock_id, timespec.as_mut_ptr()) };
    if res != 0 {
        return Err(host_impl::errno_from_nix(nix::errno::Errno::last()));
    }
    let timespec = unsafe { timespec.assume_init() };

    // convert to nanoseconds, returning EOVERFLOW in case of overflow;
    // this is freelancing a bit from the spec but seems like it'll
    // be an unusual situation to hit
    (timespec.tv_sec as host::__wasi_timestamp_t)
        .checked_mul(1_000_000_000)
        .and_then(|sec_ns| sec_ns.checked_add(timespec.tv_nsec as host::__wasi_timestamp_t))
        .map_or(Err(Error::EOVERFLOW), |resolution| {
            // a supported clock can never return zero; this case will probably never get hit, but
            // make sure we follow the spec
            if resolution == 0 {
                Err(Error::EINVAL)
            } else {
                Ok(resolution)
            }
        })
}

pub(crate) fn clock_time_get(clock_id: host::__wasi_clockid_t) -> Result<host::__wasi_timestamp_t> {
    // convert the supported clocks to the libc types, or return EINVAL
    let clock_id = match clock_id {
        host::__WASI_CLOCK_REALTIME => libc::CLOCK_REALTIME,
        host::__WASI_CLOCK_MONOTONIC => libc::CLOCK_MONOTONIC,
        host::__WASI_CLOCK_PROCESS_CPUTIME_ID => libc::CLOCK_PROCESS_CPUTIME_ID,
        host::__WASI_CLOCK_THREAD_CPUTIME_ID => libc::CLOCK_THREAD_CPUTIME_ID,
        _ => return Err(Error::EINVAL),
    };

    // no `nix` wrapper for clock_getres, so we do it ourselves
    let mut timespec = MaybeUninit::<libc::timespec>::uninit();
    let res = unsafe { libc::clock_gettime(clock_id, timespec.as_mut_ptr()) };
    if res != 0 {
        return Err(host_impl::errno_from_nix(nix::errno::Errno::last()));
    }
    let timespec = unsafe { timespec.assume_init() };

    // convert to nanoseconds, returning EOVERFLOW in case of overflow; this is freelancing a bit
    // from the spec but seems like it'll be an unusual situation to hit
    (timespec.tv_sec as host::__wasi_timestamp_t)
        .checked_mul(1_000_000_000)
        .and_then(|sec_ns| sec_ns.checked_add(timespec.tv_nsec as host::__wasi_timestamp_t))
        .map_or(Err(Error::EOVERFLOW), Ok)
}

pub(crate) fn poll_oneoff(
    fd_events: Vec<FdEventData>,
    timeout: Option<ClockEventData>,
) -> Result<Vec<host::__wasi_event_t>> {
    if fd_events.is_empty() && timeout.is_none() {
        return Ok(vec![]);
    }
    let mut poll_fds: Vec<_> = fd_events
        .iter()
        .map(|event| {
            let mut flags = nix::poll::PollFlags::empty();
            match event.type_ {
                wasm32::__WASI_EVENTTYPE_FD_READ => flags.insert(nix::poll::PollFlags::POLLIN),
                wasm32::__WASI_EVENTTYPE_FD_WRITE => flags.insert(nix::poll::PollFlags::POLLOUT),
                // An event on a file descriptor can currently only be of type FD_READ or FD_WRITE
                // Nothing else has been defined in the specification, and these are also the only two
                // events we filtered before. If we get something else here, the code has a serious bug.
                _ => unreachable!(),
            };
            Ok(nix::poll::PollFd::new(event.fd.try_into()?, flags))
        })
        .collect::<Result<Vec<_>>>()?;
    let poll_timeout = timeout.map_or(-1, |timeout| {
        timeout.delay.try_into().unwrap_or(c_int::max_value())
    });
    let ready = loop {
        match nix::poll::poll(&mut poll_fds, poll_timeout) {
            Err(_) => {
                if nix::errno::Errno::last() == nix::errno::Errno::EINTR {
                    continue;
                }
                return Err(host_impl::errno_from_nix(nix::errno::Errno::last()));
            }
            Ok(ready) => break ready as usize,
        }
    };
    Ok(if ready == 0 {
        // timeout occurred
        poll_oneoff_handle_timeout_event(timeout.expect("timeout should be Some"))
    } else {
        let events = fd_events.iter().zip(poll_fds.iter()).take(ready);
        poll_oneoff_handle_fd_event(events)?
    })
}

// define the `fionread()` function, equivalent to `ioctl(fd, FIONREAD, *bytes)`
nix::ioctl_read_bad!(fionread, nix::libc::FIONREAD, c_int);

fn poll_oneoff_handle_timeout_event(timeout: ClockEventData) -> Vec<host::__wasi_event_t> {
    let ClockEventData { userdata, .. } = timeout;
    let event = host::__wasi_event_t {
        userdata,
        type_: wasm32::__WASI_EVENTTYPE_CLOCK,
        error: wasm32::__WASI_ESUCCESS,
        u: host::__wasi_event_t___wasi_event_u {
            fd_readwrite: host::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                nbytes: 0,
                flags: 0,
            },
        },
    };
    vec![event]
}

fn poll_oneoff_handle_fd_event<'t>(
    events: impl Iterator<Item = (&'t FdEventData, &'t nix::poll::PollFd)>,
) -> Result<Vec<host::__wasi_event_t>> {
    let mut ret = vec![];
    for (fd_event, poll_fd) in events {
        let revents = match poll_fd.revents() {
            Some(revents) => revents,
            None => continue,
        };
        let mut nbytes = 0;
        if fd_event.type_ == wasm32::__WASI_EVENTTYPE_FD_READ {
            let _ = unsafe { fionread(fd_event.fd.try_into()?, &mut nbytes) };
        }
        let output_event = if revents.contains(nix::poll::PollFlags::POLLNVAL) {
            host::__wasi_event_t {
                userdata: fd_event.userdata,
                type_: fd_event.type_,
                error: wasm32::__WASI_EBADF,
                u: host::__wasi_event_t___wasi_event_u {
                    fd_readwrite:
                        host::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                            nbytes: 0,
                            flags: wasm32::__WASI_EVENT_FD_READWRITE_HANGUP,
                        },
                },
            }
        } else if revents.contains(nix::poll::PollFlags::POLLERR) {
            host::__wasi_event_t {
                userdata: fd_event.userdata,
                type_: fd_event.type_,
                error: wasm32::__WASI_EIO,
                u: host::__wasi_event_t___wasi_event_u {
                    fd_readwrite:
                        host::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                            nbytes: 0,
                            flags: wasm32::__WASI_EVENT_FD_READWRITE_HANGUP,
                        },
                },
            }
        } else if revents.contains(nix::poll::PollFlags::POLLHUP) {
            host::__wasi_event_t {
                userdata: fd_event.userdata,
                type_: fd_event.type_,
                error: wasm32::__WASI_ESUCCESS,
                u: host::__wasi_event_t___wasi_event_u {
                    fd_readwrite:
                        host::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                            nbytes: 0,
                            flags: wasm32::__WASI_EVENT_FD_READWRITE_HANGUP,
                        },
                },
            }
        } else if revents.contains(nix::poll::PollFlags::POLLIN)
            | revents.contains(nix::poll::PollFlags::POLLOUT)
        {
            host::__wasi_event_t {
                userdata: fd_event.userdata,
                type_: fd_event.type_,
                error: wasm32::__WASI_ESUCCESS,
                u: host::__wasi_event_t___wasi_event_u {
                    fd_readwrite:
                        host::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                            nbytes: nbytes as host::__wasi_filesize_t,
                            flags: 0,
                        },
                },
            }
        } else {
            continue;
        };
        ret.push(output_event)
    }
    Ok(ret)
}
