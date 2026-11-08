//! Aggregate and per-logical-CPU load from one buffered `/proc/stat` read.
use crate::proto::analysis::{Core, MAX_CORES};
use crate::source::delta;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Seek, SeekFrom},
};

pub(super) struct Cpu {
    file: BufReader<File>,
    line: String,
    previous: Vec<(u32, Sample)>,
    current: Vec<(u32, Sample)>,
    aggregate: Option<Sample>,
    readings: Vec<Core>,
}
#[derive(Clone, Copy)]
struct Sample {
    total: u64,
    idle: u64,
}
impl Sample {
    fn usage(self, before: Self) -> Option<f32> {
        let elapsed = delta(self.total, before.total);
        (elapsed > 0).then(|| {
            (elapsed - delta(self.idle, before.idle).min(elapsed)) as f32 * 100.0 / elapsed as f32
        })
    }
}
impl Cpu {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: BufReader::new(File::open("/proc/stat")?),
            line: String::with_capacity(256),
            previous: Vec::new(),
            current: Vec::new(),
            aggregate: None,
            readings: Vec::new(),
        })
    }
    pub(super) fn cores(&self) -> &[Core] {
        &self.readings
    }
    pub(super) fn usage(&mut self) -> io::Result<Option<f32>> {
        self.readings.clear();
        self.current.clear();
        self.file.seek(SeekFrom::Start(0))?;
        self.line.clear();
        self.file.read_line(&mut self.line)?;
        if !self.line.starts_with("cpu ") {
            return Err(io::Error::other("missing aggregate CPU counters"));
        }
        let aggregate = parse_stat_line(&self.line)?;
        let usage = self
            .aggregate
            .replace(aggregate)
            .and_then(|before| aggregate.usage(before));
        loop {
            self.line.clear();
            if self.file.read_line(&mut self.line)? == 0 || !self.line.starts_with("cpu") {
                break;
            }
            let Some(id) = self
                .line
                .split_ascii_whitespace()
                .next()
                .and_then(|s| s.strip_prefix("cpu"))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                break;
            };
            if self.current.len() == MAX_CORES
                || self.current.last().is_some_and(|(old, _)| *old >= id)
            {
                return Err(io::Error::other("invalid or excessive CPU counter list"));
            }
            let sample = parse_stat_line(&self.line)?;
            let index = self.current.len();
            let value = self
                .previous
                .get(index)
                .filter(|(old, _)| *old == id)
                .and_then(|(_, before)| sample.usage(*before));
            self.readings.push(Core {
                id,
                usage: value.map(|v| (v * 10.0).round() / 10.0),
            });
            self.current.push((id, sample));
        }
        std::mem::swap(&mut self.previous, &mut self.current);
        Ok(usage)
    }
}
// Guest time is already included in user/nice. Stop at steal; idle+iowait
// are not busy. Checked totals reject malformed or overflowing OS input.
fn parse_stat_line(line: &str) -> io::Result<Sample> {
    let (mut total, mut idle) = (0u64, 0u64);
    for (position, value) in line.split_ascii_whitespace().skip(1).take(8).enumerate() {
        let ticks = value.parse::<u64>().map_err(io::Error::other)?;
        total = total
            .checked_add(ticks)
            .ok_or_else(|| io::Error::other("CPU counter overflow"))?;
        if position == 3 || position == 4 {
            idle = idle
                .checked_add(ticks)
                .ok_or_else(|| io::Error::other("idle counter overflow"))?;
        }
    }
    Ok(Sample { total, idle })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_ticks_are_not_counted_twice() {
        let s = parse_stat_line("cpu 10 5 10 70 5 0 0 0 300 100").unwrap();
        assert_eq!((s.total, s.idle), (100, 75));
    }
    #[test]
    fn native_core_counters_match_the_online_cpu_rows() {
        let expected = std::fs::read_to_string("/proc/stat")
            .unwrap()
            .lines()
            .filter(|s| s.starts_with("cpu") && s.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
            .count();
        let mut cpu = Cpu::new().unwrap();
        assert!(cpu.usage().unwrap().is_none());
        assert_eq!(cpu.cores().len(), expected);
        std::thread::sleep(std::time::Duration::from_millis(100));
        cpu.usage().unwrap();
        assert!(cpu.cores().iter().any(|v| v.usage.is_some()));
        assert!(
            cpu.cores()
                .iter()
                .all(|v| v.usage.is_none_or(|x| (0.0..=100.0).contains(&x)))
        );
    }
}
