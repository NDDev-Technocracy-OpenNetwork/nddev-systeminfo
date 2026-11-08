//! Physical network throughput with hotplug-safe per-device counters.

use super::devices::Devices;
use super::{VirtualFile, field};
use std::io;
use std::time::Instant;

pub(super) struct Network {
    file: VirtualFile,
    devices: Devices,
}
pub(super) struct Throughput {
    pub(super) rx: u64,
    pub(super) tx: u64,
}

impl Network {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: VirtualFile::open("/proc/net/dev")?,
            devices: Devices::new("/sys/class/net")?,
        })
    }

    pub(super) fn throughput(&mut self) -> io::Result<Option<Throughput>> {
        self.devices.refresh();
        let now = Instant::now();
        let mut total = Throughput { rx: 0, tx: 0 };
        let mut found = false;
        for line in self.file.read()?.lines() {
            let Some((name, counters)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let Some(first) = field::<u64>(counters, 0) else {
                continue;
            };
            let Some(second) = field::<u64>(counters, 8) else {
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
            if let Some((first, second)) = device.counter.observe(first, second, now) {
                total.rx = total.rx.saturating_add(first);
                total.tx = total.tx.saturating_add(second);
                found = true;
            }
        }
        Ok(found.then_some(total))
    }
}
