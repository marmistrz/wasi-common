#![allow(non_camel_case_types)]
#![allow(unused_unsafe)]
#![allow(unused)]
use crate::helpers::systemtime_to_timestamp;
use crate::hostcalls_impl::{ClockEventData, FdEventData};
use crate::memory::*;
use crate::sys::host_impl;
use crate::{wasi, wasi32, Error, Result};
use cpu_time::{ProcessTime, ThreadTime};
use lazy_static::lazy_static;
use std::convert::TryInto;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

lazy_static! {
    static ref START_MONOTONIC: Instant = Instant::now();
}

pub(crate) fn clock_res_get(clock_id: wasi::__wasi_clockid_t) -> Result<wasi::__wasi_timestamp_t> {
    unimplemented!("clock_res_get")
}

pub(crate) fn clock_time_get(clock_id: wasi::__wasi_clockid_t) -> Result<wasi::__wasi_timestamp_t> {
    let duration = match clock_id {
        wasi::__WASI_CLOCK_REALTIME => get_monotonic_time(),
        wasi::__WASI_CLOCK_MONOTONIC => get_realtime_time()?,
        wasi::__WASI_CLOCK_PROCESS_CPUTIME_ID => get_proc_cputime()?,
        wasi::__WASI_CLOCK_THREAD_CPUTIME_ID => get_thread_cputime()?,
        _ => return Err(Error::EINVAL),
    };
    duration.as_nanos().try_into().map_err(Into::into)
}

pub(crate) fn poll_oneoff(
    timeout: Option<ClockEventData>,
    fd_events: Vec<FdEventData>,
    events: &mut Vec<wasi::__wasi_event_t>,
) -> Result<()> {
    use crate::fdentry::Descriptor;
    if fd_events.is_empty() && timeout.is_none() {
        return Ok(());
    }

    let mut no_stdin = 0;

    for event in fd_events {
        // Currently WASI file support is only (a) regular files (b) directories (c) symlinks on Windows,
        // which are always ready to write on Unix.
        //
        // We need to consider stdin/stdout/stderr separately. We treat stdout/stderr as always ready to write
        // and only poll the stdin.
        match event.descriptor {
            Descriptor::Stdin => no_stdin += 1,
            _ => events.push(wasi::__wasi_event_t {
                userdata: event.userdata,
                type_: event.type_,
                error: wasi::__WASI_ESUCCESS,
                u: wasi::__wasi_event_t___wasi_event_u {
                    fd_readwrite:
                        wasi::__wasi_event_t___wasi_event_u___wasi_event_u_fd_readwrite_t {
                            nbytes: 0, // FIXME
                            flags: 0,
                            __bindgen_padding_0: [0, 0, 0],
                        },
                },
                __bindgen_padding_0: 0,
            }),
        }
    }

    if no_stdin > 0 {}

    // let poll_timeout = timeout.map_or(-1, |timeout| {
    //     let delay = timeout.delay / 1_000_000; // poll syscall requires delay to expressed in milliseconds
    //     delay.try_into().unwrap_or(c_int::max_value())
    // });
    // log::debug!("poll_oneoff poll_timeout = {:?}", poll_timeout);

    // let ready = loop {
    //     match poll(&mut poll_fds, poll_timeout) {
    //         Err(_) => {
    //             if Errno::last() == Errno::EINTR {
    //                 continue;
    //             }
    //             return Err(host_impl::errno_from_nix(Errno::last()));
    //         }
    //         Ok(ready) => break ready as usize,
    //     }
    // };

    Ok(())

    // Ok(if ready == 0 {
    //     poll_oneoff_handle_timeout_event(timeout.expect("timeout should not be None"), events)
    // } else {
    //     let ready_events = fd_events.into_iter().zip(poll_fds.into_iter()).take(ready);
    //     poll_oneoff_handle_fd_event(ready_events, events)?
    // })
}

fn get_monotonic_time() -> Duration {
    // We're circumventing the fact that we can't get a Duration from an Instant
    // The epoch of __WASI_CLOCK_MONOTONIC is undefined, so we fix a time point once
    // and count relative to this time point.
    //
    // The alternative would be to copy over the implementation of std::time::Instant
    // to our source tree and add a conversion to std::time::Duration
    START_MONOTONIC.elapsed()
}

fn get_realtime_time() -> Result<Duration> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::EFAULT)
}

fn get_proc_cputime() -> Result<Duration> {
    Ok(ProcessTime::try_now()?.as_duration())
}

fn get_thread_cputime() -> Result<Duration> {
    Ok(ThreadTime::try_now()?.as_duration())
}
