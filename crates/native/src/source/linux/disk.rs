//! Physical disk throughput with hotplug-safe per-device counters.

use super::devices::Devices;
use super::{VirtualFile, field};
use std::io;
use std::time::Instant;

pub(super) struct Disk {
    file: VirtualFile,
    devices: Devices,
}
pub(super) struct Throughput {
    pub(super) read: u64,
    pub(super) write: u64,
}

impl Disk {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: VirtualFile::open("/proc/diskstats")?,
            devices: Devices::new("/sys/block")?,
        })
    }

    pub(super) fn throughput(&mut self) -> io::Result<Option<Throughput>> {
        self.devices.refresh();
        let now = Instant::now();
        let mut total = Throughput { read: 0, write: 0 };
        let mut found = false;
        for line in self.file.read()?.lines() {
            let Some(name) = line.split_ascii_whitespace().nth(2) else {
                continue;
            };
            let Some(first) = field::<u64>(line, 5) else {
                continue;
            };
            let Some(second) = field::<u64>(line, 9) else {
                continue;
            };
            let Some(device) = self
                .devices
                .entries
                .iter_mut()
                .find(|device| device.name == name)
            else {
                continue;
            };
            // diskstats sectors are always 512 bytes, regardless of block size.
            if let Some((first, second)) =
                device
                    .counter
                    .observe(first.saturating_mul(512), second.saturating_mul(512), now)
            {
                total.read = total.read.saturating_add(first);
                total.write = total.write.saturating_add(second);
                found = true;
            }
        }
        Ok(found.then_some(total))
    }
}
