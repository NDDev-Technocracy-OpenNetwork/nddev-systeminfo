//! Delegates snapshot collection to the native implementation for this OS.

use crate::proto::Snapshot;
use crate::source::{MetricsSource, PlatformCollector};
use std::io;

pub struct Collector(PlatformCollector);

impl Collector {
    pub fn new(gpu: bool) -> io::Result<Self> {
        MetricsSource::new(gpu).map(Self)
    }

    pub fn sample(&mut self) -> Snapshot {
        MetricsSource::sample(&mut self.0)
    }
    /// Values from the last CPU read, never a second hardware poll.
    pub fn cores(&self) -> &[crate::proto::analysis::Core] {
        MetricsSource::cores(&self.0)
    }
    pub fn gpus(&self) -> &[crate::proto::analysis::Gpu] {
        MetricsSource::gpus(&self.0)
    }
}
