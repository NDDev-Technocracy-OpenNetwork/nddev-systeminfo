//! One rolling in-memory owner. No journal, database, timer or filesystem IO.
use crate::protocol::{
    Snapshot,
    analysis::{self, Details},
};
use std::{
    collections::VecDeque,
    mem::size_of,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Record {
    pub sample: Snapshot,
    pub detail: Details,
    at: Instant,
    bytes: usize,
}
impl Record {
    pub fn new(sample: Snapshot, detail: Details, at: Instant) -> Self {
        let bytes = size_of::<Self>()
            + 32
            + detail.cores.capacity() * size_of::<analysis::Core>()
            + detail.gpus.capacity() * size_of::<analysis::Gpu>()
            + detail
                .gpus
                .iter()
                .map(|g| g.id.capacity() + g.name.capacity() + g.status.capacity() + 48)
                .sum::<usize>();
        Self {
            sample,
            detail,
            at,
            bytes,
        }
    }
}
#[derive(Default)]
pub struct History {
    records: VecDeque<Arc<Record>>,
    bytes: usize,
}
impl History {
    pub fn push(&mut self, record: Record) {
        if self.records.back().is_some_and(|old| {
            record.detail.sequence < old.detail.sequence || record.detail.at_ms < old.detail.at_ms
        }) {
            self.clear();
        }
        let now = record.at;
        let now_ms = record.detail.at_ms;
        self.bytes += record.bytes;
        self.records.push_back(Arc::new(record));
        self.prune(now, now_ms);
    }
    pub fn prune(&mut self, now: Instant, now_ms: u64) {
        while self.records.front().is_some_and(|r| {
            now.saturating_duration_since(r.at)
                >= Duration::from_secs(analysis::HISTORY_SECONDS as u64)
                || now_ms.saturating_sub(r.detail.at_ms) >= analysis::HISTORY_SECONDS as u64 * 1000
        }) || self.records.len() > analysis::HISTORY_RECORDS
            || self.bytes > analysis::HISTORY_BYTES
        {
            let Some(record) = self.records.pop_front() else {
                break;
            };
            self.bytes -= record.bytes;
        }
    }
    pub fn replace_latest(&mut self, record: Record) {
        if self.records.capacity() > 2 {
            self.clear();
        }
        self.records.clear();
        self.bytes = record.bytes;
        self.records.push_back(Arc::new(record));
    }
    pub fn latest(&self) -> Option<&Record> {
        self.records.back().map(Arc::as_ref)
    }
    /// Replay workers share immutable records; hardware collection never waits
    /// on a reader and no full payload is duplicated for each open view.
    pub fn snapshot(&mut self) -> Vec<Arc<Record>> {
        self.prune(
            Instant::now(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        );
        self.records.iter().cloned().collect()
    }
    pub fn clear(&mut self) {
        self.records = VecDeque::new();
        self.bytes = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record(at: Instant, sequence: u64) -> Record {
        Record::new(
            Snapshot::default(),
            Details {
                at_ms: sequence,
                elapsed_ms: sequence,
                sequence,
                period_ms: 500,
                gpu_enabled: false,
                cores: vec![],
                gpus: vec![],
            },
            at,
        )
    }
    #[test]
    fn expiry_and_record_limits_keep_only_the_actual_window() {
        let at = Instant::now();
        let mut h = History::default();
        h.push(record(at, 1));
        h.push(record(
            at + Duration::from_secs(analysis::HISTORY_SECONDS as u64),
            2,
        ));
        assert_eq!(h.records.len(), 1);
        assert_eq!(h.latest().unwrap().detail.sequence, 2);
        for n in 0..analysis::HISTORY_RECORDS + 4 {
            h.push(record(
                at + Duration::from_secs(analysis::HISTORY_SECONDS as u64),
                n as u64,
            ));
        }
        assert_eq!(h.records.len(), analysis::HISTORY_RECORDS);
        assert!(h.bytes <= analysis::HISTORY_BYTES);
        h.clear();
        assert_eq!(h.bytes, 0);
        assert_eq!(h.records.capacity(), 0);
    }
    #[test]
    fn byte_limit_applies_before_publishing_a_replay() {
        let at = Instant::now();
        let mut h = History::default();
        for n in 0..20 {
            let mut r = record(at, n);
            r.bytes = analysis::HISTORY_BYTES / 4;
            h.push(r);
        }
        assert_eq!(h.records.len(), 4);
        assert!(h.bytes <= analysis::HISTORY_BYTES);
    }
    #[test]
    fn suspend_expires_old_samples_even_if_the_awake_clock_did_not_move() {
        let at = Instant::now();
        let mut history = History::default();
        history.push(record(at, 1));
        history.prune(at, 1 + analysis::HISTORY_SECONDS as u64 * 1000);
        assert!(history.records.is_empty());
        assert_eq!(history.bytes, 0);
    }
}
