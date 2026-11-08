//! NVML stays loaded only while an enabled, awake NVIDIA adapter needs it.
//! Initialization uses NO_ATTACH; Linux supplies only PCI devices already awake.
use crate::proto::analysis::Gpu as Device;
#[cfg(feature = "nvidia")]
use crate::proto::analysis::MAX_GPUS;
#[cfg(feature = "nvidia")]
use crate::source::gpu;
use crate::source::gpu::Reading;
#[cfg(feature = "nvidia")]
pub(super) struct Gpu {
    nvml: Option<nvml_wrapper::Nvml>,
    devices: Vec<Device>,
    names: Vec<(String, String)>,
}
#[cfg(feature = "nvidia")]
impl Gpu {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            nvml: enabled
                .then(|| {
                    nvml_wrapper::Nvml::init_with_flags(
                        nvml_wrapper::bitmasks::InitFlags::NO_GPUS
                            | nvml_wrapper::bitmasks::InitFlags::NO_ATTACH,
                    )
                    .ok()
                })
                .flatten(),
            devices: Vec::new(),
            names: Vec::new(),
        }
    }
    pub(super) fn devices(&self) -> &[Device] {
        &self.devices
    }
    pub(super) fn read_selected(&mut self, pci: Option<&[String]>) -> Option<Reading> {
        use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
        self.devices.clear();
        self.names
            .retain(|(id, _)| pci.is_none_or(|ids| ids.iter().any(|current| current == id)));
        let nvml = self.nvml.as_ref()?;
        let count = pci
            .map_or_else(
                || nvml.device_count().unwrap_or(0) as usize,
                <[String]>::len,
            )
            .min(MAX_GPUS);
        for index in 0..count {
            let result = if let Some(ids) = pci {
                nvml.device_by_pci_bus_id(ids[index].as_str())
            } else {
                nvml.device_by_index(index as u32)
            };
            let Ok(device) = result else {
                continue;
            };
            let key = pci.map_or_else(|| index.to_string(), |ids| ids[index].clone());
            let name = if let Some((_, name)) = self.names.iter().find(|(id, _)| id == &key) {
                name.clone()
            } else {
                let name = gpu::label(device.name().unwrap_or_else(|_| "NVIDIA GPU".into()));
                self.names.push((key.clone(), name.clone()));
                name
            };
            let usage = device
                .utilization_rates()
                .ok()
                .map(|v| f64::from(v.gpu))
                .filter(|v| (0.0..=100.0).contains(v));
            let memory = device
                .memory_info()
                .ok()
                .filter(|m| m.total > 0 && m.used <= m.total);
            let temperature = device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(f64::from)
                .filter(|v| (1.0..=150.0).contains(v));
            self.devices.push(Device {
                id: format!("pci:{key}"),
                name,
                status: if usage.is_some() || memory.is_some() || temperature.is_some() {
                    "available"
                } else {
                    "unavailable"
                }
                .into(),
                usage,
                dedicated_used: memory.as_ref().map(|m| m.used),
                dedicated_total: memory.as_ref().map(|m| m.total),
                shared_used: None,
                temperature,
            });
        }
        gpu::aggregate(&self.devices)
    }
}
#[cfg(not(feature = "nvidia"))]
pub(super) struct Gpu;
#[cfg(not(feature = "nvidia"))]
impl Gpu {
    pub(super) fn new(_enabled: bool) -> Self {
        Self
    }
    pub(super) fn devices(&self) -> &[Device] {
        &[]
    }
    pub(super) fn read_selected(&mut self, _pci: Option<&[String]>) -> Option<Reading> {
        None
    }
}
