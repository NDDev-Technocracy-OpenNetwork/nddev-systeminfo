//! Physical device discovery is amortised over thirty seconds, while each
//! device keeps its own counter baseline across unchanged discovery results.

use crate::source::counters::Counter;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) struct Device {
    pub(super) name: String,
    pub(super) counter: Counter,
}
pub(super) struct Devices {
    pub(super) entries: Vec<Device>,
    path: &'static str,
    discovered: Instant,
}

impl Devices {
    pub(super) fn new(path: &'static str) -> io::Result<Self> {
        let mut devices = Self {
            entries: Vec::new(),
            path,
            discovered: Instant::now(),
        };
        devices.discover()?;
        Ok(devices)
    }

    pub(super) fn refresh(&mut self) {
        if self.discovered.elapsed() >= Duration::from_secs(30) {
            // A transient discovery failure preserves known devices.
            let _ = self.discover();
            self.discovered = Instant::now();
        }
    }

    fn discover(&mut self) -> io::Result<()> {
        let names = physical_names(Path::new(self.path))?;
        self.entries.retain(|entry| names.contains(&entry.name));
        for name in names {
            if !self.entries.iter().any(|entry| entry.name == name) {
                self.entries.push(Device {
                    name,
                    counter: Counter::default(),
                });
            }
        }
        Ok(())
    }
}

fn physical_names(path: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        // Physical block devices and network adapters have a backing device
        // link. Partitions, loopback, tunnels and container bridges do not.
        if entry.path().join("device").exists() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}
