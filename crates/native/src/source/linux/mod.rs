//! Linux collector backed directly by procfs, sysfs, and optional NVML.

mod cpu;
mod devices;
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
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use temp::Sensor;

pub(crate) struct LinuxCollector {
    cpu: Cpu,
    memory: Memory,
    disk: Disk,
    network: Network,
    gpu: Gpu,
    cpu_temperature: Option<Sensor>,
    disk_temperature: Option<Sensor>,
}

impl MetricsSource for LinuxCollector {
    fn new(gpu: bool) -> io::Result<Self> {
        Ok(Self {
            cpu: Cpu::new()?,
            memory: Memory::new()?,
            disk: Disk::new()?,
            network: Network::new()?,
            gpu: Gpu::new(gpu),
            cpu_temperature: Sensor::cpu(),
            disk_temperature: Sensor::disk(),
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
        if let Ok(Some(throughput)) = self.disk.throughput() {
            snapshot.disk_read = Some(throughput.read);
            snapshot.disk_write = Some(throughput.write);
        }
        if let Ok(Some(throughput)) = self.network.throughput() {
            snapshot.net_rx = Some(throughput.rx);
            snapshot.net_tx = Some(throughput.tx);
        }
        if let Some(reading) = self.gpu.read() {
            snapshot.gpu = reading.usage;
            snapshot.gpu_memory = reading.memory;
            snapshot.gpu_temperature = reading.temperature;
        }
        snapshot.cpu_temperature = read_sensor(self.cpu_temperature.as_mut());
        snapshot.disk_temperature = read_sensor(self.disk_temperature.as_mut());
        snapshot
    }
}

fn read_sensor(sensor: Option<&mut Sensor>) -> Option<f32> {
    sensor?.celsius().ok().flatten()
}

/// A procfs or sysfs file held open across samples. `read` rewinds and
/// returns the fresh contents without reallocating the buffer.
struct VirtualFile {
    file: File,
    buf: String,
}

impl VirtualFile {
    fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            buf: String::with_capacity(4096),
        })
    }

    fn read(&mut self) -> io::Result<&str> {
        self.file.seek(SeekFrom::Start(0))?;
        self.buf.clear();
        self.file.read_to_string(&mut self.buf)?;
        Ok(&self.buf)
    }

    fn read_prefix(&mut self, limit: u64) -> io::Result<&str> {
        self.file.seek(SeekFrom::Start(0))?;
        self.buf.clear();
        (&mut self.file).take(limit).read_to_string(&mut self.buf)?;
        Ok(&self.buf)
    }
}

/// The Nth whitespace-separated field of a procfs line, parsed.
fn field<T: std::str::FromStr>(line: &str, index: usize) -> Option<T> {
    line.split_ascii_whitespace().nth(index)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_reads_the_nth_column() {
        let line = " 259 0 nvme0n1 2397597 508478 60721278 220367";
        assert_eq!(field::<u64>(line, 5), Some(60721278));
    }
}
