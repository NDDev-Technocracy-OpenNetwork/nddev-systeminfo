//! Windows aggregate CPU load from system counters.

use sysinfo::{CpuRefreshKind, RefreshKind, System};

pub(super) struct Cpu {
    system: System,
    primed: bool,
}

impl Cpu {
    pub(super) fn new() -> Self {
        Self {
            system: System::new_with_specifics(
                RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
            ),
            primed: false,
        }
    }

    /// Busy share of all cores since the previous call, in percent.
    ///
    /// The initial call only primes counters; the first publication can happen
    /// before sysinfo's minimum refresh interval and must not pretend to be idle.
    pub(super) fn usage(&mut self) -> Option<f32> {
        self.system.refresh_cpu_usage();
        if !std::mem::replace(&mut self.primed, true) {
            return None;
        }
        Some(self.system.global_cpu_usage())
    }
}
