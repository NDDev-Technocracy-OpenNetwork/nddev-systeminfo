//! Windows collector backed by system counters through sysinfo and optional
//! NVML.

mod cpu;
mod disk;
mod mem;
mod net;
mod temp;

use crate::proto::Snapshot;
use crate::source::MetricsSource;
use crate::source::nvidia::Gpu;
use cpu::Cpu;
use disk::Disk;
use mem::Memory;
use net::Network;
use std::io;
use temp::Temperatures;

pub(crate) struct WindowsCollector {
    cpu: Cpu,
    memory: Memory,
    disk: Disk,
    network: Network,
    gpu: Gpu,
    temperatures: Temperatures,
}

impl MetricsSource for WindowsCollector {
    fn new(gpu: bool) -> io::Result<Self> {
        Ok(Self {
            cpu: Cpu::new(),
            memory: Memory::new(),
            disk: Disk::new(),
            network: Network::new(),
            gpu: Gpu::new(gpu),
            temperatures: Temperatures::new(),
        })
    }

    fn sample(&mut self) -> Snapshot {
        let mut snapshot = Snapshot {
            cpu: self.cpu.usage(),
            ..Snapshot::default()
        };
        if let Some(usage) = self.memory.usage() {
            snapshot.memory = Some(usage.used);
            snapshot.swap = usage.swap;
        }
        if let Some(throughput) = self.disk.throughput() {
            snapshot.disk_read = Some(throughput.read);
            snapshot.disk_write = Some(throughput.write);
        }
        if let Some(throughput) = self.network.throughput() {
            snapshot.net_rx = Some(throughput.rx);
            snapshot.net_tx = Some(throughput.tx);
        }
        if let Some(reading) = self.gpu.read() {
            snapshot.gpu = reading.usage;
            snapshot.gpu_memory = reading.memory;
            snapshot.gpu_temperature = reading.temperature;
        }
        self.temperatures.refresh();
        snapshot.cpu_temperature = self.temperatures.cpu();
        snapshot.disk_temperature = self.temperatures.disk();
        snapshot
    }
}
