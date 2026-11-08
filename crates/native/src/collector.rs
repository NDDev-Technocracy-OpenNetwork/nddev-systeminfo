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
}
