//! macOS metrics from Mach, BSD, IOKit, and AppleSMC.

mod cf;
mod cpu;
mod disk;
mod gpu;
mod hid;
mod mach;
mod mem;
mod net;
mod registry;
mod smc;
mod temp;

use crate::proto::Snapshot;
use crate::source::MetricsSource;
use cpu::Cpu;
use disk::Disk;
use gpu::Gpu;
use mem::Memory;
use net::Network;
use std::io;
use temp::Temperatures;

pub(crate) struct MacosCollector {
    cpu: Cpu,
    memory: Memory,
    network: Network,
    disk: Disk,
    gpu: Option<Gpu>,
    temperatures: Temperatures,
}

impl MetricsSource for MacosCollector {
    fn new(gpu: bool) -> io::Result<Self> {
        Ok(Self {
            cpu: Cpu::new(),
            memory: Memory::new()?,
            network: Network::new(),
            disk: Disk::new(),
            gpu: gpu.then(Gpu::new),
            temperatures: Temperatures::new(),
        })
    }

    fn sample(&mut self) -> Snapshot {
        let mut snapshot = Snapshot {
            cpu: self.cpu.usage().ok().flatten(),
            ..Snapshot::default()
        };
        if let Ok(Some(usage)) = self.memory.usage() {
            snapshot.memory = Some(usage.used);
            snapshot.swap = usage.swap;
        }
        if let Some(throughput) = self.disk.throughput() {
            snapshot.disk_read = Some(throughput.read);
            snapshot.disk_write = Some(throughput.write);
        }
        if let Ok(Some(throughput)) = self.network.throughput() {
            snapshot.net_rx = Some(throughput.rx);
            snapshot.net_tx = Some(throughput.tx);
        }
        let reading = self.gpu.as_mut().map(Gpu::read);
        snapshot.gpu = reading.as_ref().and_then(|reading| reading.usage);
        snapshot.cpu_temperature = self.temperatures.cpu();
        // SMC keys first, then the accelerator's own report, then the HID
        // sensor Apple silicon exposes — each a fallback for the one before.
        if let Some(reading) = reading {
            snapshot.gpu_temperature = self
                .temperatures
                .gpu()
                .or(reading.temperature)
                .or_else(|| self.temperatures.gpu_hid());
        }
        snapshot.disk_temperature = self.temperatures.disk();
        snapshot
    }
}

/// Mean of accumulated sensor readings, or `None` when nothing answered.
/// Callers fold readings into a (sum, count) pair as they go, so a tick
/// allocates nothing for temperature math.
fn mean(sum: f64, count: usize) -> Option<f32> {
    (count > 0).then(|| (sum / count as f64) as f32)
}
