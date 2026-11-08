//! Passive accelerator counters. UMA usage is shared bytes, never VRAM percent.
use super::registry::Registry;
use crate::proto::analysis::Gpu as Device;
use crate::source::gpu::{self, Reading};

pub(super) struct Gpu {
    statistics: Option<Registry<4>>,
    power: Option<Registry<1>>,
    devices: Vec<Device>,
    apple_name: String,
}
impl Gpu {
    pub(super) fn new() -> Self {
        Self {
            statistics: Registry::new(
                c"IOAccelerator",
                c"PerformanceStatistics",
                [
                    c"Device Utilization %",
                    c"GPU Activity(%)",
                    c"Temperature(C)",
                    c"In use system memory",
                ],
            ),
            power: Registry::new(c"IOAccelerator", c"AGCInfo", [c"poweredOffByAGC"]),
            devices: Vec::new(),
            apple_name: apple_name(),
        }
    }
    pub(super) fn devices(&self) -> &[Device] {
        &self.devices
    }
    pub(super) fn apple_temperature(&mut self, value: Option<f32>) {
        if self.devices.len() == 1 {
            let device = &mut self.devices[0];
            if device.name.starts_with("Apple")
                && device.status == "available"
                && device.temperature.is_none()
            {
                device.temperature = value.map(f64::from);
            }
        }
    }
    pub(super) fn read(&mut self) -> Reading {
        self.devices.clear();
        let mut sleeping = Vec::new();
        if let Some(power) = &mut self.power {
            power.visit_identified(|id, _, values| {
                if values[0].and_then(|v| v.unsigned()).is_some_and(|v| v != 0) {
                    sleeping.push(id);
                }
            });
        }
        if let Some(registry) = &mut self.statistics {
            registry.visit_identified(|id, class, values| {
                let asleep = sleeping.contains(&id);
                let lower = class.to_ascii_lowercase();
                let (name, kind) = if lower.contains("agx") {
                    ("Apple GPU", "shared")
                } else if lower.contains("intel") {
                    ("Intel GPU", "shared")
                } else if lower.contains("amd") {
                    ("AMD GPU", "dedicated")
                } else if lower.contains("nv") {
                    ("NVIDIA GPU", "dedicated")
                } else {
                    ("GPU", "unknown")
                };
                let usage = (!asleep)
                    .then(|| {
                        values[0]
                            .or(values[1])
                            .map(|v| v.decimal())
                            .filter(|v| (0.0..=100.0).contains(v))
                    })
                    .flatten();
                let temperature = (!asleep)
                    .then(|| {
                        values[2]
                            .map(|v| v.decimal())
                            .filter(|v| (1.0..=120.0).contains(v))
                    })
                    .flatten();
                let memory_used = (!asleep && kind == "shared")
                    .then(|| values[3].and_then(|v| v.unsigned()))
                    .flatten();
                self.devices.push(Device {
                    id: format!("ioreg:{id}"),
                    name: if lower.contains("agx") {
                        self.apple_name.clone()
                    } else {
                        name.into()
                    },
                    status: if asleep {
                        "sleeping"
                    } else if usage.is_some() || memory_used.is_some() || temperature.is_some() {
                        "available"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    usage,
                    shared_used: memory_used,
                    dedicated_used: None,
                    dedicated_total: None,
                    temperature,
                });
            });
        }
        gpu::aggregate(&self.devices).unwrap_or(Reading {
            usage: None,
            memory: None,
            temperature: None,
        })
    }
}

fn apple_name() -> String {
    let mut bytes = [0u8; 128];
    let mut length = bytes.len();
    // SAFETY: one bounded, writable sysctl output buffer. No input or kernel
    // pointer is retained. Brand metadata is queried once per collector lifetime.
    let status = unsafe {
        libc::sysctlbyname(
            c"machdep.cpu.brand_string".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 && length <= bytes.len() {
        let end = bytes[..length]
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(length);
        if let Ok(name) = std::str::from_utf8(&bytes[..end])
            && name.starts_with("Apple ")
            && name.len() <= 120
            && name.bytes().all(|b| b.is_ascii() && !b.is_ascii_control())
        {
            return format!("{} GPU", name.trim());
        }
    }
    "Apple GPU".into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_accelerators_have_bounded_readings_and_uma_is_not_vram() {
        for _ in 0..8 {
            let mut gpu = Gpu::new();
            gpu.read();
            for d in gpu.devices() {
                assert!(d.usage.is_none_or(|v| (0.0..=100.0).contains(&v)));
                assert!(!d.id.is_empty());
                assert!(d.dedicated_total.is_none());
                if d.status == "sleeping" {
                    assert!(d.usage.is_none());
                }
            }
        }
    }
}
