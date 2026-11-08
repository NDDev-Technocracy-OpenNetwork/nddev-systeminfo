//! Windows aggregate CPU load from system counters.

use crate::proto::analysis::Core;
use std::io;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

pub(super) struct Cpu {
    system: System,
    primed: bool,
    cores: Vec<Core>,
}

impl Cpu {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            system: System::new_with_specifics(
                RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_usage()),
            )
            .map_err(io::Error::other)?,
            primed: false,
            cores: Vec::new(),
        })
    }

    /// Busy share of all cores since the previous call, in percent.
    ///
    /// The initial call only primes counters; the first publication can happen
    /// before sysinfo's minimum refresh interval and must not pretend to be idle.
    pub(super) fn usage(&mut self) -> Option<f32> {
        self.system.refresh_cpu_usage();
        let primed = std::mem::replace(&mut self.primed, true);
        self.cores.clear();
        self.cores
            .extend(self.system.cpus().iter().enumerate().map(|(id, cpu)| {
                Core {
                    id: id as u32,
                    usage: primed
                        .then(|| f64::from((cpu.usage() * 10.0).round() / 10.0))
                        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
                }
            }));
        if !primed {
            return None;
        }
        Some(self.system.global_cpu_usage())
    }
    pub(super) fn cores(&self) -> &[Core] {
        &self.cores
    }
}
