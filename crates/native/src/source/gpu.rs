//! Consistent aggregation: the busiest available adapter, never an average
//! that hides a saturated device among idle adapters.
use crate::proto::analysis::Gpu;

pub(super) struct Reading {
    pub(super) usage: Option<f32>,
    pub(super) memory: Option<f32>,
    pub(super) temperature: Option<f32>,
}
pub(super) fn aggregate(devices: &[Gpu]) -> Option<Reading> {
    let gpu = devices
        .iter()
        .filter(|g| g.status == "available")
        .max_by(|a, b| a.usage.unwrap_or(-1.0).total_cmp(&b.usage.unwrap_or(-1.0)))?;
    Some(Reading {
        usage: gpu.usage.map(|v| v as f32),
        memory: gpu
            .dedicated_used
            .zip(gpu.dedicated_total)
            .filter(|(used, total)| *total > 0 && used <= total)
            .map(|(used, total)| used as f32 * 100.0 / total as f32),
        temperature: gpu.temperature.map(|v| v as f32),
    })
}
#[cfg(all(target_os = "linux", feature = "nvidia"))]
pub(super) fn label(value: String) -> String {
    let mut value: String = value.chars().filter(|c| !c.is_control()).collect();
    let mut end = value.len().min(128);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}
