//! macOS aggregate CPU busy time from Mach host statistics.

use super::mach::Host;
use libc::{c_int, c_uint};
use std::io;
use std::mem;

const HOST_CPU_LOAD_INFO: c_int = 3;
const CPU_STATE_MAX: usize = 4;
const CPU_STATE_IDLE: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HostCpuLoadInfo {
    ticks: [u32; CPU_STATE_MAX],
}

unsafe extern "C" {
    fn host_statistics(host: c_uint, flavor: c_int, info: *mut c_int, count: *mut c_uint) -> c_int;
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
    let mut info = HostCpuLoadInfo::default();
    let mut count = (mem::size_of::<HostCpuLoadInfo>() / mem::size_of::<c_int>()) as c_uint;
    let result = unsafe {
        host_statistics(
            host,
            HOST_CPU_LOAD_INFO,
            (&mut info as *mut HostCpuLoadInfo).cast(),
            &mut count,
        )
    };
    if result != 0 {
        return Err(io::Error::other(format!(
            "host_statistics failed: {result}"
        )));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

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
