//! Cached IOKit storage services; failed reads retain their elapsed interval.
use super::registry::Registry;
use crate::source::counters::Counter;
use std::time::Instant;

pub(super) struct Disk {
    registry: Option<Registry<2>>,
    generation: u64,
    counter: Counter,
}
pub(super) struct Throughput {
    pub(super) read: u64,
    pub(super) write: u64,
}
impl Disk {
    pub(super) fn new() -> Self {
        Self {
            registry: Registry::new(
                c"IOBlockStorageDriver",
                c"Statistics",
                [c"Bytes (Read)", c"Bytes (Write)"],
            ),
            generation: 0,
            counter: Counter::default(),
        }
    }
    pub(super) fn throughput(&mut self) -> Option<Throughput> {
        let registry = self.registry.as_mut()?;
        let mut total = (0_u64, 0_u64);
        let mut complete = true;
        let found = registry.visit(|values| {
            match (
                values[0].and_then(|number| number.unsigned()),
                values[1].and_then(|number| number.unsigned()),
            ) {
                (Some(read), Some(write)) => {
                    total.0 = total.0.saturating_add(read);
                    total.1 = total.1.saturating_add(write);
                }
                _ => complete = false,
            }
        });
        // Hotplug changes the population of lifetime counters. Begin a fresh
        // interval rather than subtracting two different device sets.
        if self.generation != registry.generation() {
            self.counter = Counter::default();
            self.generation = registry.generation();
        }
        if !found || !complete {
            return None;
        }
        self.counter
            .observe(total.0, total.1, Instant::now())
            .map(|(read, write)| Throughput { read, write })
    }
}
