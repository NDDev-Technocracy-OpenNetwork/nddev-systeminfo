//! Pure, shared plot preparation. Thin native views retain only the current
//! bounded view model; aggregation and truthful gaps are implemented once.
pub use crate::protocol::analysis::{Point, Statistics, View};
use crate::protocol::{
    self,
    analysis::{Frame, HISTORY_BYTES, HISTORY_RECORDS, HISTORY_SECONDS},
};
use std::{collections::VecDeque, mem::size_of};

#[derive(Clone)]
pub struct Selection {
    pub key: String,
    pub label: String,
    pub unit: String,
}
impl Selection {
    pub fn parse(key: &str) -> Option<Self> {
        if let Some(metric) = protocol::METRICS.iter().find(|m| m.key == key) {
            return Some(Self {
                key: key.into(),
                label: metric.label.into(),
                unit: metric.unit.into(),
            });
        }
        if key
            .strip_prefix("core:")
            .is_some_and(|id| id.parse::<u32>().is_ok())
        {
            return Some(Self {
                key: key.into(),
                label: "logical_core".into(),
                unit: "percent".into(),
            });
        }
        let (id, field) = key.strip_prefix("gpu:")?.rsplit_once(':')?;
        if id.is_empty() || id.len() > 128 {
            return None;
        }
        let (label, unit) = match field {
            "usage" => ("gpu", "percent"),
            "temperature" => ("gpu_temperature", "celsius"),
            "dedicated_used" => ("dedicated_memory", "bytes"),
            "shared_used" => ("shared_memory", "bytes"),
            _ => return None,
        };
        Some(Self {
            key: key.into(),
            label: label.into(),
            unit: unit.into(),
        })
    }
    fn value(&self, frame: &Frame) -> Option<f64> {
        if let Some(id) = self.key.strip_prefix("core:") {
            return frame
                .detail
                .cores
                .iter()
                .find(|c| Some(c.id) == id.parse().ok())
                .and_then(|c| c.usage)
                .map(f64::from);
        }
        if let Some((id, field)) = self
            .key
            .strip_prefix("gpu:")
            .and_then(|s| s.rsplit_once(':'))
        {
            let gpu = frame
                .detail
                .gpus
                .iter()
                .find(|g| g.id == id && g.status == "available")?;
            return match field {
                "usage" => gpu.usage,
                "temperature" => gpu.temperature,
                "dedicated_used" => gpu.dedicated_used.map(|n| n as f64),
                "shared_used" => gpu.shared_used.map(|n| n as f64),
                _ => None,
            };
        }
        protocol::metric_value(&frame.sample, &self.key)
    }
}
#[derive(Default)]
pub struct Store {
    frames: VecDeque<(Frame, usize)>,
    bytes: usize,
}
impl Store {
    pub fn push(&mut self, frame: Frame) {
        if self.frames.back().is_some_and(|(old, _)| {
            frame.detail.sequence < old.detail.sequence
                || frame.detail.elapsed_ms < old.detail.elapsed_ms
        }) {
            self.clear();
        }
        if self
            .frames
            .back()
            .is_some_and(|(old, _)| frame.detail.sequence == old.detail.sequence)
        {
            return;
        }
        let bytes = size_of::<Frame>()
            + frame.detail.cores.capacity() * size_of::<protocol::analysis::Core>()
            + frame.detail.gpus.capacity() * size_of::<protocol::analysis::Gpu>()
            + frame
                .detail
                .gpus
                .iter()
                .map(|g| g.id.capacity() + g.name.capacity() + g.status.capacity() + 48)
                .sum::<usize>()
            + 32;
        let now = frame.detail.elapsed_ms;
        self.frames.push_back((frame, bytes));
        self.bytes += bytes;
        while self.frames.front().is_some_and(|(f, _)| {
            now.saturating_sub(f.detail.elapsed_ms) >= HISTORY_SECONDS as u64 * 1000
        }) || self.frames.len() > HISTORY_RECORDS
            || self.bytes > HISTORY_BYTES
        {
            let Some((_, bytes)) = self.frames.pop_front() else {
                break;
            };
            self.bytes -= bytes;
        }
    }
    pub fn clear(&mut self) {
        self.frames = VecDeque::new();
        self.bytes = 0;
    }
    pub fn view(&self, selection: &Selection, range_seconds: u64, columns: usize) -> View {
        let range_seconds = range_seconds.clamp(1, HISTORY_SECONDS as u64);
        let columns = columns.clamp(1, crate::protocol::analysis::MAX_PLOT_COLUMNS);
        let latest = self.frames.back().map(|(f, _)| f);
        let end_ms = latest.map_or(0, |f| f.detail.elapsed_ms);
        let start_ms = end_ms.saturating_sub(range_seconds * 1000);
        let span_ms = range_seconds * 1000;
        let available_ms = self
            .frames
            .front()
            .map_or(0, |(f, _)| end_ms.saturating_sub(f.detail.elapsed_ms))
            .min(span_ms);
        let mut buckets: Vec<Option<Point>> = vec![None; columns];
        let mut statistics = Statistics::default();
        let (mut weighted, mut previous) = (0.0, None::<(&Frame, Option<f64>)>);
        for (frame, _) in &self.frames {
            let value = selection.value(frame).filter(|v| v.is_finite());
            let gap = previous.is_none_or(|(last, v)| {
                v.is_none()
                    || frame.detail.sequence != last.detail.sequence.saturating_add(1)
                    || frame
                        .detail
                        .elapsed_ms
                        .saturating_sub(last.detail.elapsed_ms)
                        > frame.detail.period_ms.max(last.detail.period_ms) * 2
            });
            if frame.detail.elapsed_ms >= start_ms
                && let Some(value) = value
            {
                statistics.samples += 1;
                statistics.minimum = Some(statistics.minimum.map_or(value, |old| old.min(value)));
                statistics.maximum = Some(statistics.maximum.map_or(value, |old| old.max(value)));
                if !gap && let Some((last, _)) = previous {
                    let weight = frame
                        .detail
                        .elapsed_ms
                        .saturating_sub(last.detail.elapsed_ms.max(start_ms));
                    let total = statistics.covered_ms + weight;
                    if total > 0 {
                        weighted = weighted * (statistics.covered_ms as f64 / total as f64)
                            + value * (weight as f64 / total as f64);
                    }
                    statistics.covered_ms = total;
                }
                // Anchor the full requested span even before an hour exists.
                // Early samples occupy the right side instead of pretending
                // that a few seconds cover the whole hour.
                let age = end_ms.saturating_sub(frame.detail.elapsed_ms).min(span_ms);
                let index = ((span_ms - age) as u128 * columns as u128 / span_ms as u128)
                    .min(columns as u128 - 1) as usize;
                match &mut buckets[index] {
                    Some(point) => {
                        point.low = point.low.min(value);
                        point.high = point.high.max(value);
                        point.value = value;
                        point.x_ms = frame.detail.elapsed_ms;
                        point.gap |= gap;
                    }
                    slot @ None => {
                        *slot = Some(Point {
                            x_ms: frame.detail.elapsed_ms,
                            low: value,
                            high: value,
                            value,
                            gap,
                        })
                    }
                }
            }
            previous = Some((frame, value));
        }
        statistics.current = latest.and_then(|f| selection.value(f));
        statistics.average = if statistics.covered_ms > 0 {
            Some(weighted)
        } else if statistics.samples == 1 {
            statistics.current
        } else {
            None
        };
        let y_max = if selection.unit == "percent" {
            100.0
        } else {
            let maximum = statistics.maximum.unwrap_or(0.0).max(0.0);
            if maximum == 0.0 {
                1.0
            } else {
                (maximum * 1.1).min(f64::MAX)
            }
        };
        let y_min = statistics.minimum.unwrap_or(0.0).min(0.0);
        View {
            v: crate::protocol::analysis::VIEW_VERSION,
            latest: latest.cloned(),
            metric: selection.key.clone(),
            label: selection.label.clone(),
            unit: selection.unit.clone(),
            start_ms,
            end_ms,
            available_ms,
            range_seconds,
            y_min,
            y_max,
            points: buckets.into_iter().flatten().collect(),
            statistics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(seq: u64, at: u64, value: Option<f32>) -> Frame {
        let mut line = String::new();
        protocol::Snapshot {
            cpu: value,
            ..Default::default()
        }
        .encode(&mut line);
        Frame {
            sample: protocol::decode(line.as_bytes()).unwrap(),
            detail: protocol::analysis::Details {
                at_ms: at,
                elapsed_ms: at,
                sequence: seq,
                period_ms: 1000,
                gpu_enabled: false,
                cores: vec![],
                gpus: vec![],
            },
        }
    }
    #[test]
    fn narrow_plots_keep_peaks_and_statistics_are_time_weighted() {
        let mut store = Store::default();
        for (seq, at, value) in [(1, 0, 0.0), (2, 1000, 100.0), (3, 3000, 20.0)] {
            store.push(frame(seq, at, Some(value)));
        }
        let view = store.view(&Selection::parse("cpu.usage").unwrap(), 60, 1);
        assert_eq!(view.points.len(), 1);
        assert_eq!(view.points[0].high, 100.0);
        assert_eq!(view.statistics.samples, 3);
        assert!((view.statistics.average.unwrap() - 140.0 / 3.0).abs() < 0.0001);
    }
    #[test]
    fn missed_intervals_and_nulls_are_not_filled_or_averaged_as_zero() {
        let mut store = Store::default();
        store.push(frame(1, 0, Some(80.0)));
        store.push(frame(2, 1000, None));
        store.push(frame(3, 2000, Some(20.0)));
        store.push(frame(4, 20000, Some(10.0)));
        let view = store.view(&Selection::parse("cpu.usage").unwrap(), 60, 512);
        assert!(view.points.iter().all(|p| p.gap));
        assert_eq!(view.statistics.covered_ms, 0);
        assert!(view.statistics.average.is_none());
        assert_eq!(view.statistics.minimum, Some(10.0));
    }
    #[test]
    fn restart_and_retention_release_old_sessions() {
        let mut store = Store::default();
        store.push(frame(4, 3_600_000, Some(10.0)));
        store.push(frame(1, 0, Some(20.0)));
        assert_eq!(store.frames.len(), 1);
        store.push(frame(2, 3_600_000, Some(30.0)));
        assert_eq!(store.frames.len(), 1);
        store.clear();
        assert_eq!(store.frames.capacity(), 0);
    }
}
