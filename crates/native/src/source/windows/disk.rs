//! Windows storage throughput from per-volume performance counters.

use crate::source::counters::Counter;
use std::{collections::BTreeMap, path::PathBuf, time::Instant};
use sysinfo::Disks;

pub(super) struct Disk {
    disks: Disks,
    counters: BTreeMap<PathBuf, Counter>,
}

pub(super) struct Throughput {
    pub(super) read: u64,
    pub(super) write: u64,
}

impl Disk {
    pub(super) fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            counters: BTreeMap::new(),
        }
    }

    /// Bytes read and written per second summed across every volume.
    ///
    /// The first call establishes the baseline and reports nothing, because a
    /// rate needs two samples.
    pub(super) fn throughput(&mut self) -> Option<Throughput> {
        self.disks.refresh(true);
        self.counters
            .retain(|path, _| self.disks.iter().any(|disk| disk.mount_point() == path));
        let now = Instant::now();
        let mut total = Throughput { read: 0, write: 0 };
        let mut found = false;
        for disk in self.disks.iter() {
            let key = disk.mount_point();
            if !self.counters.contains_key(key) {
                self.counters.insert(key.to_path_buf(), Counter::default());
            }
            let usage = disk.usage();
            if let Some((read, write)) = self.counters.get_mut(key).unwrap().observe(
                usage.total_read_bytes,
                usage.total_written_bytes,
                now,
            ) {
                total.read = total.read.saturating_add(read);
                total.write = total.write.saturating_add(write);
                found = true;
            }
        }
        found.then_some(total)
    }
}
