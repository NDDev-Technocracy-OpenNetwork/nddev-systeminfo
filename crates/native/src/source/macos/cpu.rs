//! macOS aggregate CPU busy time from Mach host statistics.

use super::mach::Host;
use crate::proto::analysis::{Core, MAX_CORES};
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
    previous: Vec<HostCpuLoadInfo>,
    cores: Vec<Core>,
}

impl Cpu {
    pub(super) fn new() -> Self {
        Self {
            host: Host::new(),
            previous: Vec::new(),
            cores: Vec::new(),
        }
    }

    /// Busy share of all cores since the previous call, in percent.
    ///
    /// The first call establishes the baseline and reports nothing, because a
    /// percentage needs two samples.
    pub(super) fn usage(&mut self) -> io::Result<Option<f32>> {
        self.cores.clear();
        let current = read_ticks(self.host.0)?;
        let previous_count = self.previous.len();
        self.previous
            .resize(current.count / CPU_STATE_MAX, HostCpuLoadInfo::default());
        let (mut total, mut busy) = (0u64, 0u64);
        for (id, ticks) in current.ticks().iter().enumerate() {
            let now = HostCpuLoadInfo {
                ticks: ticks.map(|v| v as u32),
            };
            let previous = std::mem::replace(&mut self.previous[id], now);
            let delta = differences(now, previous);
            let elapsed: u64 = delta.iter().sum();
            let active = elapsed - delta[CPU_STATE_IDLE];
            let usage = (id < previous_count && elapsed > 0)
                .then(|| (active as f64 * 1000.0 / elapsed as f64).round() / 10.0);
            self.cores.push(Core {
                id: id as u32,
                usage,
            });
            total += elapsed;
            busy += active;
        }
        Ok((previous_count == self.previous.len() && total > 0)
            .then(|| busy as f32 * 100.0 / total as f32))
    }
    pub(super) fn cores(&self) -> &[Core] {
        &self.cores
    }
}

fn differences(current: HostCpuLoadInfo, previous: HostCpuLoadInfo) -> [u64; CPU_STATE_MAX] {
    std::array::from_fn(|index| current.ticks[index].wrapping_sub(previous.ticks[index]) as u64)
}

struct Array {
    pointer: *mut c_int,
    count: usize,
}
impl Array {
    fn ticks(&self) -> &[[c_int; CPU_STATE_MAX]] {
        // SAFETY: read_ticks validated the initialized count, nonnull pointer,
        // processor bound and exact four-counter stride. The VM owner outlives
        // this borrowed slice and no mutable alias is exposed.
        unsafe { std::slice::from_raw_parts(self.pointer.cast(), self.count / CPU_STATE_MAX) }
    }
}
impl Drop for Array {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: a successful Mach call transferred this one VM allocation.
            // The exact returned byte count is released once, including errors.
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
fn read_ticks(host: c_uint) -> io::Result<Array> {
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
    let array = Array {
        pointer,
        count: count as usize,
    };
    if pointer.is_null()
        || processors == 0
        || processors as usize > MAX_CORES
        || processors.checked_mul(CPU_STATE_MAX as u32) != Some(count)
    {
        return Err(io::Error::other("invalid processor counter array"));
    }
    Ok(array)
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
        let mut cpu = Cpu::new();
        assert!(cpu.usage().unwrap().is_none());
        assert_eq!(
            cpu.cores().len(),
            read_ticks(host.0).unwrap().count / CPU_STATE_MAX
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        cpu.usage().unwrap();
        assert!(cpu.cores().iter().any(|core| core.usage.is_some()));
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
