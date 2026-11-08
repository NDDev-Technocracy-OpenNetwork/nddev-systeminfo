//! Passive DRM/sysfs and NVML discovery. Never probe a suspended PCI adapter.
use super::VirtualFile;
use crate::{
    proto::analysis::{Gpu as Device, MAX_GPUS},
    source::{
        gpu::{self, Reading},
        nvidia,
    },
};
use std::{
    fs,
    time::{Duration, Instant},
};
struct Card {
    id: String,
    name: String,
    vendor: String,
    power: Option<VirtualFile>,
    busy: Option<VirtualFile>,
    used: Option<VirtualFile>,
    total: Option<VirtualFile>,
    temperature: Option<VirtualFile>,
}
pub(super) struct Gpu {
    enabled: bool,
    cards: Vec<Card>,
    nvidia: Option<nvidia::Gpu>,
    devices: Vec<Device>,
    discovered: Instant,
}
impl Gpu {
    pub(super) fn new(enabled: bool) -> Self {
        let mut gpu = Self {
            enabled,
            cards: Vec::new(),
            nvidia: None,
            devices: Vec::new(),
            discovered: Instant::now(),
        };
        if enabled {
            gpu.discover();
        }
        gpu
    }
    pub(super) fn devices(&self) -> &[Device] {
        &self.devices
    }
    fn discover(&mut self) {
        self.cards.clear();
        self.nvidia = None;
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name
                    .strip_prefix("card")
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                    || self.cards.len() == MAX_GPUS
                {
                    continue;
                }
                let path = entry.path().join("device");
                let Ok(device) = fs::canonicalize(&path) else {
                    continue;
                };
                let Some(id) = device
                    .file_name()
                    .and_then(|v| v.to_str())
                    .map(str::to_owned)
                else {
                    continue;
                };
                let vendor = fs::read_to_string(path.join("vendor"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let brand = match vendor.as_str() {
                    "0x1002" => "AMD GPU",
                    "0x8086" => "Intel GPU",
                    "0x10de" => "NVIDIA GPU",
                    _ => "GPU",
                };
                let temperature = fs::read_dir(path.join("hwmon"))
                    .ok()
                    .and_then(|mut entries| {
                        entries.find_map(|entry| {
                            VirtualFile::open(entry.ok()?.path().join("temp1_input")).ok()
                        })
                    });
                self.cards.push(Card {
                    id,
                    name: brand.into(),
                    vendor,
                    power: VirtualFile::open(path.join("power/runtime_status")).ok(),
                    busy: VirtualFile::open(path.join("gpu_busy_percent")).ok(),
                    used: VirtualFile::open(path.join("mem_info_vram_used")).ok(),
                    total: VirtualFile::open(path.join("mem_info_vram_total")).ok(),
                    temperature,
                });
            }
        }
        self.cards.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        self.discovered = Instant::now();
    }
    pub(super) fn read(&mut self) -> Option<Reading> {
        if !self.enabled {
            return None;
        }
        if self.discovered.elapsed() >= Duration::from_secs(30) {
            self.discover();
        }
        self.devices.clear();
        let mut awake = Vec::new();
        for card in &mut self.cards {
            let sleeping = card
                .power
                .as_mut()
                .and_then(|file| file.read().ok())
                .is_some_and(|v| v.trim() == "suspended");
            if !sleeping && card.vendor == "0x10de" {
                awake.push(card.id.clone());
                continue;
            }
            let (usage, used, total, temperature) = if sleeping {
                (None, None, None, None)
            } else {
                (
                    number(card.busy.as_mut())
                        .map(|v| v as f64)
                        .filter(|v| (0.0..=100.0).contains(v)),
                    number(card.used.as_mut()),
                    number(card.total.as_mut()).filter(|v| *v > 0),
                    number(card.temperature.as_mut())
                        .map(|v| v as f64 / 1000.0)
                        .filter(|v| (1.0..=150.0).contains(v)),
                )
            };
            let (used, total) = if used.zip(total).is_some_and(|(used, total)| used > total) {
                (None, None)
            } else {
                (used, total)
            };
            self.devices.push(Device {
                id: format!("pci:{}", card.id),
                name: card.name.clone(),
                status: if sleeping {
                    "sleeping"
                } else if usage.is_some() || used.is_some() || temperature.is_some() {
                    "available"
                } else {
                    "unavailable"
                }
                .into(),
                usage,
                dedicated_used: used,
                dedicated_total: total,
                shared_used: None,
                temperature,
            });
        }
        if awake.is_empty() {
            self.nvidia = None;
        } else {
            let gpu = self.nvidia.get_or_insert_with(|| nvidia::Gpu::new(true));
            gpu.read_selected(Some(&awake));
            for id in &awake {
                let key = format!("pci:{id}");
                if let Some(device) = gpu.devices().iter().find(|device| device.id == key) {
                    self.devices.push(device.clone());
                } else {
                    self.devices.push(Device {
                        id: key,
                        name: "NVIDIA GPU".into(),
                        status: "unavailable".into(),
                        usage: None,
                        dedicated_used: None,
                        dedicated_total: None,
                        shared_used: None,
                        temperature: None,
                    });
                }
            }
        }
        gpu::aggregate(&self.devices)
    }
}
fn number(file: Option<&mut VirtualFile>) -> Option<u64> {
    file?.read().ok()?.trim().parse().ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_gpu_does_not_discover_or_load_a_driver() {
        let mut gpu = Gpu::new(false);
        assert!(gpu.read().is_none());
        assert!(gpu.cards.is_empty());
        assert!(gpu.nvidia.is_none());
        assert!(gpu.devices().is_empty());
    }
    #[test]
    fn native_gpu_inventory_stays_bounded_and_read_only() {
        let mut gpu = Gpu::new(true);
        gpu.read();
        assert!(gpu.devices().len() <= MAX_GPUS);
        for d in gpu.devices() {
            assert!(d.usage.is_none_or(|v| (0.0..=100.0).contains(&v)));
            if d.status == "sleeping" {
                assert!(d.usage.is_none());
                assert!(d.temperature.is_none());
            }
        }
    }
}
