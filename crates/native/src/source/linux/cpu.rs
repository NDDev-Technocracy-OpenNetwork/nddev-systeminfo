//! Linux aggregate CPU busy time from `/proc/stat`.

use super::VirtualFile;
use crate::source::delta;
use std::io;

pub(super) struct Cpu {
    file: VirtualFile,
    previous: Option<Sample>,
}

#[derive(Clone, Copy)]
struct Sample {
    total: u64,
    idle: u64,
}

impl Cpu {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: VirtualFile::open("/proc/stat")?,
            previous: None,
        })
    }

    /// Busy share of all cores since the previous call, in percent.
    ///
    /// The first call establishes the baseline and reports nothing, because a
    /// percentage needs two samples.
    pub(super) fn usage(&mut self) -> io::Result<Option<f32>> {
        let line = self
            .file
            .read_prefix(1024)?
            .lines()
            .next()
            .unwrap_or_default();
        let current = parse_stat_line(line);
        let previous = self.previous.replace(current);

        Ok(previous.and_then(|previous| {
            let elapsed = delta(current.total, previous.total);
            if elapsed == 0 {
                return None;
            }
            let busy = elapsed - delta(current.idle, previous.idle).min(elapsed);
            Some(busy as f32 * 100.0 / elapsed as f32)
        }))
    }
}

/// Reads the aggregate `cpu` row: `user nice system idle iowait irq softirq
/// steal guest guest_nice`.
///
/// `guest` and `guest_nice` are already folded into `user` and `nice` by the
/// kernel, so the count stops at `steal` — summing them again would inflate
/// the denominator and understate load on hosts running virtual machines.
fn parse_stat_line(line: &str) -> Sample {
    let mut total = 0u64;
    let mut idle = 0u64;
    for (position, value) in line.split_ascii_whitespace().skip(1).enumerate() {
        if position > 7 {
            break;
        }
        let Ok(ticks) = value.parse::<u64>() else {
            continue;
        };
        total += ticks;
        // `idle` and `iowait` both mean the core had nothing to run.
        if position == 3 || position == 4 {
            idle += ticks;
        }
    }
    Sample { total, idle }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_ticks_are_not_counted_twice() {
        // user nice system idle iowait irq softirq steal guest guest_nice
        let sample = parse_stat_line("cpu  10 5 10 70 5 0 0 0 300 100");
        assert_eq!(sample.total, 100);
        assert_eq!(sample.idle, 75);
    }

    #[test]
    fn a_short_line_parses_what_is_there() {
        let sample = parse_stat_line("cpu  1 0 1 8");
        assert_eq!(sample.total, 10);
        assert_eq!(sample.idle, 8);
    }
}
