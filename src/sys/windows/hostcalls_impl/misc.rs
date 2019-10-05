#![allow(non_camel_case_types)]
#![allow(unused_unsafe)]
#![allow(unused)]
use crate::helpers::systemtime_to_timestamp;
use crate::memory::*;
use crate::sys::host_impl;
use crate::{host, wasm32, Error, Result};
use cpu_time::{ProcessTime, ThreadTime};
use lazy_static::lazy_static;
use std::convert::TryInto;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

lazy_static! {
    static ref START_MONOTONIC: Instant = Instant::now();
}

use wasi_common_cbindgen::wasi_common_cbindgen;

pub(crate) fn clock_res_get(clock_id: host::__wasi_clockid_t) -> Result<host::__wasi_timestamp_t> {
    unimplemented!("clock_res_get")
}

pub(crate) fn clock_time_get(clock_id: host::__wasi_clockid_t) -> Result<host::__wasi_timestamp_t> {
    let duration = match clock_id {
        host::__WASI_CLOCK_REALTIME => get_monotonic_time(),
        host::__WASI_CLOCK_MONOTONIC => get_realtime_time()?,
        host::__WASI_CLOCK_PROCESS_CPUTIME_ID => get_proc_cputime()?,
        host::__WASI_CLOCK_THREAD_CPUTIME_ID => get_thread_cputime()?,
        _ => return Err(Error::EINVAL),
    };
    duration.as_nanos().try_into().map_err(Into::into)
}

pub(crate) fn poll_oneoff(
    input: Vec<Result<host::__wasi_subscription_t>>,
    output_slice: &mut [wasm32::__wasi_event_t],
) -> Result<wasm32::size_t> {
    unimplemented!("poll_oneoff")
}

fn get_monotonic_time() -> Duration {
    // We're circumventing the fact that we can't get a Duration from an Instant
    // The epoch of __WASI_CLOCK_MONOTONIC is undefined, so we fix a time point once
    // and count relative to this time point.
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
