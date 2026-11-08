//! Native metric collectors selected at compile time.
//!
//! Every platform directory holds the same set of per-domain readers —
//! `cpu`, `mem`, `disk`, `net`, `temp` — assembled by its `mod.rs` into one
//! `MetricsSource`. NVIDIA readings live in `nvidia`, shared by the two
//! platforms where that driver exists.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

mod counters;

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod nvidia;

#[cfg(target_os = "linux")]
pub(crate) use linux::LinuxCollector as PlatformCollector;
#[cfg(target_os = "macos")]
pub(crate) use macos::MacosCollector as PlatformCollector;
#[cfg(target_os = "windows")]
pub(crate) use windows::WindowsCollector as PlatformCollector;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("nddev-systeminfod supports Linux, macOS, and Windows");

use crate::proto::Snapshot;
use std::io;

/// The contract every platform collector fulfils: build once at daemon start,
/// then produce one snapshot per tick. Implementing it through this trait is
/// what keeps the three backends in lockstep — a missing or mistyped method
/// fails the build rather than drifting silently.
pub(crate) trait MetricsSource {
    fn new(gpu: bool) -> io::Result<Self>
    where
        Self: Sized;
    fn sample(&mut self) -> Snapshot;
}

// Compile-time proof that the selected platform honours the contract.
const _: fn() = || {
    fn contract<T: MetricsSource>() {}
    contract::<PlatformCollector>();
};

/// A counter difference that treats a reset as zero.
///
/// Kernel counters only ever move forward, but a hot-unplugged device or a
/// 32-bit wraparound can make the new reading smaller; saturating keeps a
/// reset from becoming a huge false spike.
fn delta(current: u64, previous: u64) -> u64 {
    current.saturating_sub(previous)
}

/// Bytes per second over a real elapsed interval.
fn rate(bytes: u64, seconds: f64) -> u64 {
    if seconds <= 0.0 {
        0
    } else {
        (bytes as f64 / seconds) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_tolerate_resets_and_zero_intervals() {
        assert_eq!(delta(10, 4), 6);
        assert_eq!(delta(4, 10), 0);
        assert_eq!(rate(1024, 2.0), 512);
        assert_eq!(rate(1024, 0.0), 0);
    }
}
