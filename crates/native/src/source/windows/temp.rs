//! Windows component temperatures.
//!
//! sysinfo reads whatever the hardware publishes through the platform's
//! component providers; many machines expose nothing without elevated sensor
//! drivers, in which case every reading is simply `null`.

use sysinfo::Components;

/// Label fragments that identify the processor package sensor.
const CPU_LABELS: &[&str] = &["cpu", "package", "core", "tctl", "tdie"];
/// Label fragments that identify a storage sensor.
const DISK_LABELS: &[&str] = &["nvme", "ssd", "hdd", "drive", "storage"];

pub(super) struct Temperatures {
    components: Option<Components>,
}

impl Temperatures {
    pub(super) fn new() -> Self {
        Self {
            components: Components::new_with_refreshed_list().ok(),
        }
    }

    pub(super) fn refresh(&mut self) {
        if let Some(components) = &mut self.components {
            components.refresh(true);
        }
    }

    /// Only CPU-labelled sensors identify a processor temperature. An unrelated
    /// component must not be substituted when the OS exposes no CPU sensor.
    pub(super) fn cpu(&self) -> Option<f32> {
        self.hottest(CPU_LABELS)
    }

    /// Hottest storage-labelled sensor. There is no fallback: an unrelated
    /// component is not a disk temperature.
    pub(super) fn disk(&self) -> Option<f32> {
        self.hottest(DISK_LABELS)
    }

    fn hottest(&self, labels: &[&str]) -> Option<f32> {
        self.components
            .as_ref()?
            .iter()
            .filter(|component| {
                labels.is_empty()
                    || labels
                        .iter()
                        .any(|marker| component.label().to_lowercase().contains(marker))
            })
            .filter_map(|component| component.temperature())
            .filter(|temperature| (0.0..=120.0).contains(temperature))
            .max_by(|a, b| a.total_cmp(b))
    }
}
