//! macOS aggregate CPU busy time from Mach host statistics.

use super::mach::Host;
use libc::{c_int, c_uint};
use std::io;
const CPU_STATE_MAX: usize = 4;
const CPU_STATE_IDLE: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HostCpuLoadInfo {
    ticks: [u32; CPU_STATE_MAX],
}

pub(super) struct Cpu {
    host: Host,
    previous: Option<HostCpuLoadInfo>,
}

impl Cpu {
    pub(super) fn new() -> Self {
        Self {
            host: Host::new(),
            previous: None,
        }
    }

    /// Busy share of all cores since the previous call, in percent.
    ///
    /// The first call establishes the baseline and reports nothing, because a
    /// percentage needs two samples.
    pub(super) fn usage(&mut self) -> io::Result<Option<f32>> {
        let current = read_ticks(self.host.0)?;
        let previous = self.previous.replace(current);

        Ok(previous.and_then(|previous| {
            let ticks = differences(current, previous);
            let elapsed: u64 = ticks.iter().sum();
            if elapsed == 0 {
                return None;
            }
            let busy = elapsed - ticks[CPU_STATE_IDLE];
            Some(busy as f32 * 100.0 / elapsed as f32)
        }))
    }
}

fn differences(current: HostCpuLoadInfo, previous: HostCpuLoadInfo) -> [u64; CPU_STATE_MAX] {
    std::array::from_fn(|index| current.ticks[index].wrapping_sub(previous.ticks[index]) as u64)
}

fn read_ticks(host: c_uint) -> io::Result<HostCpuLoadInfo> {
    let mut processors: c_uint = 0;
    let mut pointer: *mut c_int = std::ptr::null_mut();
    let mut count: c_uint = 0;
    // Unlike host_statistics, this per-processor API is not cached for a
    // second. The 500 ms protocol cadence therefore measures distinct ticks.
    // SAFETY: live host send right and initialized, correctly typed outputs.
    let result = unsafe {
        libc::host_processor_info(
            host,
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut processors,
            &mut pointer,
            &mut count,
        )
    };
    if result != 0 {
        return Err(io::Error::other(format!(
            "host_processor_info failed: {result}"
        )));
    }
    struct Array {
        pointer: *mut c_int,
        count: usize,
    }
    impl Drop for Array {
        fn drop(&mut self) {
            // SAFETY: the successful Mach call transfers this VM allocation
            // to the caller. Release it exactly once, including validation errors.
            if !self.pointer.is_null() {
                unsafe {
                    libc::vm_deallocate(
                        super::mach::task_port(),
                        self.pointer as _,
                        (self.count * std::mem::size_of::<c_int>()) as _,
                    )
                };
            }
        }
    }
    let array = Array {
        pointer,
        count: count as usize,
    };
    if pointer.is_null()
        || processors == 0
        || processors.checked_mul(CPU_STATE_MAX as u32) != Some(count)
    {
        return Err(io::Error::other("invalid processor counter array"));
    }
    // SAFETY: Mach returned count initialized integer entries and their owner
    // stays alive until this slice is no longer used.
    let entries = unsafe { std::slice::from_raw_parts(array.pointer, array.count) };
    let mut info = HostCpuLoadInfo::default();
    for core in entries.as_chunks::<CPU_STATE_MAX>().0 {
        for (total, ticks) in info.ticks.iter_mut().zip(core) {
            *total = total.wrapping_add(*ticks as u32);
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_processor_arrays_can_be_read_and_released_repeatedly() {
        let host = Host::new();
        for _ in 0..16 {
            assert!(read_ticks(host.0).is_ok());
        }
    }

    #[test]
    fn wrapping_a_32_bit_mach_counter_preserves_load() {
        let previous = HostCpuLoadInfo {
            ticks: [u32::MAX - 5, 10, 100, 0],
        };
        let current = HostCpuLoadInfo {
            ticks: [4, 15, 185, 0],
        };
        assert_eq!(differences(current, previous), [10, 5, 85, 0]);
    }
}
