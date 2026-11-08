//! Linux memory and swap pressure from `/proc/meminfo`.

use super::VirtualFile;
use std::io;

pub(super) struct Memory {
    file: VirtualFile,
}

pub(super) struct Usage {
    /// Share of physical memory unavailable to new allocations, in percent.
    pub(super) used: f32,
    /// Share of swap in use, in percent. `None` when the system has no swap.
    pub(super) swap: Option<f32>,
}

impl Memory {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: VirtualFile::open("/proc/meminfo")?,
        })
    }

    pub(super) fn usage(&mut self) -> io::Result<Option<Usage>> {
        let (mut total, mut available) = (0u64, 0u64);
        let (mut swap_total, mut swap_free) = (0u64, 0u64);

        for line in self.file.read()?.lines() {
            let Some((key, rest)) = line.split_once(':') else {
                continue;
            };
            let Some(value) = rest.split_ascii_whitespace().next() else {
                continue;
            };
            let Ok(kilobytes) = value.parse::<u64>() else {
                continue;
            };
            match key {
                // `MemAvailable` is the kernel's own estimate of what a new
                // allocation can claim, which is what a user reads as "free".
                "MemTotal" => total = kilobytes,
                "MemAvailable" => available = kilobytes,
                "SwapTotal" => swap_total = kilobytes,
                "SwapFree" => swap_free = kilobytes,
                _ => {}
            }
        }

        if total == 0 {
            return Ok(None);
        }

        let used = (total.saturating_sub(available)) as f32 * 100.0 / total as f32;
        let swap = (swap_total > 0)
            .then(|| swap_total.saturating_sub(swap_free) as f32 * 100.0 / swap_total as f32);

        Ok(Some(Usage { used, swap }))
    }
}
