//! Read-only WDDM counters for every vendor. No driver attachment or elevated
//! process access. Busiest engine is summed across processes, then maximized.
use crate::{
    proto::analysis::{Gpu as Device, MAX_GPUS},
    source::gpu::{self, Reading},
};
use std::{collections::BTreeMap, mem::size_of, ptr};

type Handle = isize;
#[repr(C)]
#[derive(Clone, Copy)]
struct Value {
    status: u32,
    value: f64,
}
#[repr(C)]
struct Item {
    name: *const u16,
    value: Value,
}
#[link(name = "pdh")]
unsafe extern "system" {
    fn PdhOpenQueryW(source: *const u16, data: usize, query: *mut Handle) -> u32;
    fn PdhAddEnglishCounterW(
        query: Handle,
        path: *const u16,
        data: usize,
        counter: *mut Handle,
    ) -> u32;
    fn PdhCollectQueryData(query: Handle) -> u32;
    fn PdhGetFormattedCounterArrayW(
        counter: Handle,
        format: u32,
        bytes: *mut u32,
        count: *mut u32,
        items: *mut Item,
    ) -> u32;
    fn PdhCloseQuery(query: Handle) -> u32;
}
struct Query {
    handle: Handle,
    counters: [Handle; 3],
    buffer: Vec<u64>,
}
impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: this owner closes the successful query once; PDH owns and
        // releases its counters. No borrowed counter or buffer escapes.
        unsafe { PdhCloseQuery(self.handle) };
    }
}
impl Query {
    fn new() -> Option<Self> {
        Self::open([
            r"\GPU Engine(*)\Utilization Percentage",
            r"\GPU Adapter Memory(*)\Dedicated Usage",
            r"\GPU Adapter Memory(*)\Shared Usage",
        ])
    }
    fn open(paths: [&str; 3]) -> Option<Self> {
        let mut handle = 0;
        // SAFETY: default live data source and correctly typed output storage.
        if unsafe { PdhOpenQueryW(ptr::null(), 0, &mut handle) } != 0 {
            return None;
        }
        let mut query = Self {
            handle,
            counters: [0; 3],
            buffer: Vec::new(),
        };
        for (index, path) in paths.iter().enumerate() {
            let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            // SAFETY: owned query, terminated UTF-16 path and valid output.
            // English-counter lookup is independent of the Windows locale.
            if unsafe {
                PdhAddEnglishCounterW(handle, path.as_ptr(), 0, &mut query.counters[index])
            } != 0
                && index == 0
            {
                return None;
            }
        }
        Some(query)
    }
    fn collect(&mut self) -> bool {
        // SAFETY: this thread owns the live query and serializes collection.
        unsafe { PdhCollectQueryData(self.handle) == 0 }
    }
    fn visit(&mut self, index: usize, mut read: impl FnMut(&str, f64)) {
        let counter = self.counters[index];
        if counter == 0 {
            return;
        }
        let (mut bytes, mut count) = (0, 0);
        const FORMAT: u32 = 0x200 | 0x8000; // DOUBLE | NOCAP100; cap after engine sum.
        // SAFETY: zero-sized sizing request writes only these two integers.
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, FORMAT, &mut bytes, &mut count, ptr::null_mut())
        };
        if status != 0x800007d2 || bytes == 0 || bytes > 4 * 1024 * 1024 {
            return;
        }
        self.buffer
            .resize((bytes as usize).div_ceil(size_of::<u64>()), 0);
        let capacity = self.buffer.len() * size_of::<u64>();
        // SAFETY: aligned u64 allocation supplies the requested byte capacity.
        // PDH initializes the array and embedded names only on successful return.
        if unsafe {
            PdhGetFormattedCounterArrayW(
                counter,
                FORMAT,
                &mut bytes,
                &mut count,
                self.buffer.as_mut_ptr().cast(),
            )
        } != 0
            || bytes as usize > capacity
            || count > 65536
            || count as usize > bytes as usize / size_of::<Item>()
        {
            return;
        }
        let begin = self.buffer.as_ptr() as usize;
        let end = begin + bytes as usize;
        // SAFETY: validated initialized count and C layout/alignment. The query
        // buffer is not resized or passed to PDH while these borrows exist.
        let items = unsafe {
            std::slice::from_raw_parts(self.buffer.as_ptr().cast::<Item>(), count as usize)
        };
        let mut name = String::with_capacity(256);
        for item in items {
            if item.value.status > 1 || !item.value.value.is_finite() || item.value.value < 0.0 {
                continue;
            }
            let address = item.name as usize;
            if address < begin || address >= end || !address.is_multiple_of(2) {
                continue;
            }
            let maximum = ((end - address) / 2).min(512);
            // SAFETY: pointer range and alignment were checked against this
            // initialized buffer. A missing bounded terminator rejects the item.
            let units = unsafe { std::slice::from_raw_parts(item.name, maximum) };
            let Some(length) = units.iter().position(|v| *v == 0) else {
                continue;
            };
            name.clear();
            for character in char::decode_utf16(units[..length].iter().copied()) {
                name.push(character.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            read(&name, item.value.value);
        }
    }
}
pub(super) struct Gpu {
    query: Option<Query>,
    primed: bool,
    devices: Vec<Device>,
    names: Vec<String>,
}
impl Gpu {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            query: enabled.then(Query::new).flatten(),
            primed: false,
            devices: Vec::new(),
            names: Vec::new(),
        }
    }
    pub(super) fn devices(&self) -> &[Device] {
        &self.devices
    }
    pub(super) fn read(&mut self) -> Option<Reading> {
        self.devices.clear();
        let query = self.query.as_mut()?;
        if !query.collect() {
            return None;
        }
        let primed = std::mem::replace(&mut self.primed, true);
        let mut engines: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        query.visit(0, |name, value| {
            if let Some((adapter, engine)) = engine_key(name) {
                if engines.len() >= MAX_GPUS && !engines.contains_key(adapter) {
                    return;
                }
                if !engines.contains_key(adapter) {
                    engines.insert(adapter.into(), BTreeMap::new());
                }
                let row = engines.get_mut(adapter).unwrap();
                if let Some(old) = row.get_mut(engine) {
                    *old += value;
                } else if row.len() < 256 {
                    row.insert(engine.into(), value);
                }
            }
        });
        let mut memory: BTreeMap<String, (Option<u64>, Option<u64>)> = BTreeMap::new();
        for index in [1, 2] {
            query.visit(index, |name, value| {
                let Some(adapter) = adapter_key(name) else {
                    return;
                };
                if memory.len() >= MAX_GPUS && !memory.contains_key(adapter) {
                    return;
                }
                if !memory.contains_key(adapter) {
                    memory.insert(adapter.into(), (None, None));
                }
                let row = memory.get_mut(adapter).unwrap();
                if index == 1 {
                    row.0 = Some(value as u64);
                } else {
                    row.1 = Some(value as u64);
                }
            });
        }
        self.names
            .retain(|id| engines.contains_key(id) || memory.contains_key(id));
        for id in engines.keys().chain(memory.keys()) {
            if !self.names.iter().any(|old| old == id) && self.names.len() < MAX_GPUS {
                self.names.push(id.clone());
            }
        }
        for (index, id) in self.names.iter().enumerate() {
            if !engines.contains_key(id) && !memory.contains_key(id) {
                continue;
            }
            let usage = primed
                .then(|| {
                    engines
                        .get(id)
                        .and_then(|rows| rows.values().copied().max_by(f64::total_cmp))
                        .map(|v| (v.clamp(0.0, 100.0) * 10.0).round() / 10.0)
                })
                .flatten();
            let (dedicated_used, shared_used) = memory.get(id).copied().unwrap_or_default();
            self.devices.push(Device {
                id: id.clone(),
                name: format!("GPU {index}"),
                status: "available".into(),
                usage,
                dedicated_used,
                dedicated_total: None,
                shared_used,
                temperature: None,
            });
        }
        gpu::aggregate(&self.devices)
    }
}
fn adapter_key(name: &str) -> Option<&str> {
    let start = name.find("luid_")?;
    let value = &name[start..];
    let phys = value.find("_phys_")? + 6;
    let end = value[phys..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(value.len(), |n| phys + n);
    (end > phys && end <= 128).then_some(&value[..end])
}
fn engine_key(name: &str) -> Option<(&str, &str)> {
    let adapter = adapter_key(name)?;
    let (_, engine) = name.split_once("_eng_")?;
    let end = engine.find('_').unwrap_or(engine.len());
    (!engine[..end].is_empty() && engine[..end].bytes().all(|c| c.is_ascii_digit()))
        .then_some((adapter, &engine[..end]))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_instances_share_an_adapter_and_engine_key() {
        assert_eq!(
            engine_key("pid_42_luid_0x00000000_0x00012345_phys_0_eng_2_engtype_3D"),
            Some(("luid_0x00000000_0x00012345_phys_0", "2"))
        );
        assert!(engine_key("unknown").is_none());
    }
    #[test]
    fn formatted_counter_arrays_are_exercised_against_real_cpu_counters() {
        let mut query = Query::open([
            r"\Processor(*)\% Processor Time",
            r"\Memory\Available Bytes",
            r"\Memory\Available Bytes",
        ])
        .expect("native PDH counters");
        assert!(query.collect());
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert!(query.collect());
        let mut count = 0;
        query.visit(0, |name, value| {
            assert!(!name.is_empty());
            assert!(value.is_finite() && value >= 0.0);
            count += 1;
        });
        assert!(count > 0);
    }
    #[test]
    fn native_queries_release_handles_and_tolerate_missing_drivers() {
        for _ in 0..8 {
            let mut gpu = Gpu::new(true);
            gpu.read();
            std::thread::sleep(std::time::Duration::from_millis(20));
            gpu.read();
            assert!(gpu.devices().len() <= MAX_GPUS);
        }
        let mut disabled = Gpu::new(false);
        assert!(disabled.read().is_none());
        assert!(disabled.query.is_none());
    }
}
