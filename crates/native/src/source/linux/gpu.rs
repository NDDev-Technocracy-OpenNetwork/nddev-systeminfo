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
    driver: Option<String>,
    power: Option<VirtualFile>,
    busy: Option<VirtualFile>,
    used: Option<VirtualFile>,
    shared: Option<VirtualFile>,
    total: Option<VirtualFile>,
    temperature: Option<VirtualFile>,
}
impl Card {
    fn discover(path: &std::path::Path) -> Option<Self> {
        let device = fs::canonicalize(path).ok()?;
        let id = device.file_name()?.to_str()?.to_string();
        let vendor = fs::read_to_string(path.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let driver = fs::read_link(path.join("driver"))
            .ok()
            .and_then(|target| target.file_name()?.to_str().map(str::to_owned));
        let brand = match vendor.as_str() {
            "0x1002" => "AMD GPU",
            "0x8086" => "Intel GPU",
            "0x10de" => "NVIDIA GPU",
            _ => "GPU",
        };
        let temperature = fs::read_dir(path.join("hwmon"))
            .ok()
            .and_then(|mut entries| {
                entries.find_map(|e| VirtualFile::scalar(e.ok()?.path().join("temp1_input")).ok())
            });
        Some(Self {
            id,
            name: brand.into(),
            vendor,
            driver,
            power: VirtualFile::scalar(path.join("power/runtime_status")).ok(),
            busy: VirtualFile::scalar(path.join("gpu_busy_percent")).ok(),
            used: VirtualFile::scalar(path.join("mem_info_vram_used")).ok(),
            shared: VirtualFile::scalar(path.join("mem_info_gtt_used")).ok(),
            total: VirtualFile::scalar(path.join("mem_info_vram_total")).ok(),
            temperature,
        })
    }
    fn has_nvidia_driver(&self) -> bool {
        self.vendor == "0x10de" && self.driver.as_deref() == Some("nvidia")
    }
}
pub(super) trait NvidiaBackend {
    fn available(&self) -> bool;
    fn devices(&self) -> &[Device];
    fn clear_devices(&mut self);
    fn read_selected(&mut self, pci: Option<&[String]>) -> Option<Reading>;
}
impl NvidiaBackend for nvidia::Gpu {
    fn available(&self) -> bool {
        nvidia::Gpu::available(self)
    }
    fn devices(&self) -> &[Device] {
        nvidia::Gpu::devices(self)
    }
    fn clear_devices(&mut self) {
        nvidia::Gpu::clear_devices(self);
    }
    fn read_selected(&mut self, pci: Option<&[String]>) -> Option<Reading> {
        nvidia::Gpu::read_selected(self, pci)
    }
}
enum NvidiaState<N = nvidia::Gpu> {
    Unloaded,
    Ready(N),
    Failed { bound_cards: Vec<String> },
}
pub(super) struct Gpu<N = nvidia::Gpu> {
    enabled: bool,
    cards: Vec<Card>,
    nvidia: NvidiaState<N>,
    devices: Vec<Device>,
    discovered: Instant,
}
impl Gpu {
    pub(super) fn new(enabled: bool) -> Self {
        Self::with_backend(enabled)
    }
    pub(super) fn read(&mut self) -> Option<Reading> {
        self.read_with(|| nvidia::Gpu::new(true))
    }
}
impl<N: NvidiaBackend> Gpu<N> {
    fn with_backend(enabled: bool) -> Self {
        let mut gpu = Self {
            enabled,
            cards: Vec::new(),
            nvidia: NvidiaState::Unloaded,
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
    fn nvidia_bound_cards(&self) -> Vec<String> {
        self.cards
            .iter()
            .filter(|card| card.has_nvidia_driver())
            .map(|card| card.id.clone())
            .collect()
    }
    fn discover(&mut self) {
        let mut paths = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name
                    .strip_prefix("card")
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                {
                    paths.push(entry.path().join("device"));
                }
            }
        }
        // Headless NVIDIA compute devices need not expose a DRM card.
        if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
            for entry in entries.flatten() {
                let path = entry.path();
                if fs::read_to_string(path.join("vendor")).is_ok_and(|v| v.trim() == "0x10de")
                    && fs::read_to_string(path.join("class")).is_ok_and(|v| v.starts_with("0x03"))
                {
                    paths.push(path);
                }
            }
        }
        self.cards.clear();
        for path in paths {
            if self.cards.len() == MAX_GPUS {
                break;
            }
            if let Some(card) = Card::discover(&path)
                && !self.cards.iter().any(|old| old.id == card.id)
            {
                self.cards.push(card);
            }
        }
        self.cards.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        self.reconcile_nvidia_failure();
        self.discovered = Instant::now();
    }
    fn reconcile_nvidia_failure(&mut self) {
        let bound_cards = self.nvidia_bound_cards();
        if let NvidiaState::Failed {
            bound_cards: previous,
        } = &self.nvidia
            && *previous != bound_cards
        {
            self.nvidia = NvidiaState::Unloaded;
        }
    }
    fn read_with<F>(&mut self, init_nvidia: F) -> Option<Reading>
    where
        F: FnOnce() -> N,
    {
        if !self.enabled {
            return None;
        }
        if self.discovered.elapsed() >= Duration::from_secs(30) {
            self.discover();
        }
        self.devices.clear();
        let mut awake = Vec::new();
        for card in &mut self.cards {
            let (sleeping, active) = card
                .power
                .as_mut()
                .and_then(|file| file.read().ok())
                .map(|v| {
                    let state = v.trim();
                    (
                        matches!(state, "suspended" | "suspending"),
                        state == "active",
                    )
                })
                .unwrap_or_default();
            if card.has_nvidia_driver() && active {
                awake.push(card.id.clone());
                continue;
            }
            let unreadable = sleeping || card.vendor == "0x10de";
            let (usage, used, total, temperature) = if unreadable {
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
                shared_used: if unreadable {
                    None
                } else {
                    number(card.shared.as_mut())
                },
                temperature,
            });
        }
        if awake.is_empty() {
            // Keep one NVML handle across sleep/wake transitions: repeated init/shutdown cycles
            // leak eventfd descriptors inside libnvidia-ml, and InitFlags::NO_GPUS does not wake
            // suspended adapters when no device queries are issued.
            if let NvidiaState::Ready(gpu) = &mut self.nvidia {
                gpu.clear_devices();
            }
        } else {
            if matches!(self.nvidia, NvidiaState::Unloaded) {
                let gpu = init_nvidia();
                self.nvidia = if gpu.available() {
                    NvidiaState::Ready(gpu)
                } else {
                    NvidiaState::Failed {
                        bound_cards: self.nvidia_bound_cards(),
                    }
                };
            }
            if let NvidiaState::Ready(gpu) = &mut self.nvidia {
                gpu.read_selected(Some(&awake));
            }
            for id in &awake {
                let key = format!("pci:{id}");
                let device = match &self.nvidia {
                    NvidiaState::Ready(gpu) => gpu.devices().iter().find(|d| d.id == key).cloned(),
                    NvidiaState::Unloaded | NvidiaState::Failed { .. } => None,
                };
                if let Some(device) = device {
                    self.devices.push(device);
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
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempCardDir(PathBuf);

    impl TempCardDir {
        fn new(label: &str) -> Self {
            let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "systeminfo-gpu-{label}-{}-{seq}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn set_power(&self, state: &str) {
            let power = self.0.join("power");
            fs::create_dir_all(&power).unwrap();
            fs::write(power.join("runtime_status"), format!("{state}\n")).unwrap();
        }
        fn bind_driver(&self, driver: &str) {
            let target = self.0.join(format!("fake-drivers/{driver}"));
            fs::create_dir_all(&target).unwrap();
            symlink(&target, self.0.join("driver")).unwrap();
        }
    }

    impl Drop for TempCardDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct FakeNvidia {
        available: bool,
        devices: Vec<Device>,
        reads: usize,
        clears: usize,
    }

    impl FakeNvidia {
        fn new(available: bool) -> Self {
            Self {
                available,
                devices: Vec::new(),
                reads: 0,
                clears: 0,
            }
        }
    }

    impl NvidiaBackend for FakeNvidia {
        fn available(&self) -> bool {
            self.available
        }
        fn devices(&self) -> &[Device] {
            &self.devices
        }
        fn clear_devices(&mut self) {
            self.clears += 1;
            self.devices.clear();
        }
        fn read_selected(&mut self, pci: Option<&[String]>) -> Option<Reading> {
            self.reads += 1;
            self.devices.clear();
            for id in pci.unwrap_or(&[]) {
                self.devices.push(Device {
                    id: format!("pci:{id}"),
                    name: "NVIDIA RTX".into(),
                    status: "available".into(),
                    usage: Some(42.0),
                    dedicated_used: Some(1024),
                    dedicated_total: Some(4096),
                    shared_used: None,
                    temperature: Some(55.0),
                });
            }
            gpu::aggregate(&self.devices)
        }
    }

    #[test]
    fn disabled_gpu_does_not_discover_or_load_a_driver() {
        let mut gpu = Gpu::new(false);
        assert!(gpu.read().is_none());
        assert!(gpu.cards.is_empty());
        assert!(matches!(gpu.nvidia, NvidiaState::Unloaded));
        assert!(gpu.devices().is_empty());
    }
    #[test]
    fn unknown_nvidia_power_state_does_not_load_the_driver() {
        let dir = TempCardDir::new("power");
        fs::write(dir.path().join("vendor"), "0x10de\n").unwrap();
        dir.bind_driver("nvidia");
        let card = Card::discover(dir.path()).unwrap();
        let mut gpu: Gpu<FakeNvidia> = Gpu::with_backend(false);
        gpu.enabled = true;
        gpu.cards.push(card);
        let mut inits = 0;
        gpu.read_with(|| {
            inits += 1;
            FakeNvidia::new(true)
        });
        assert_eq!(inits, 0);
        assert!(matches!(gpu.nvidia, NvidiaState::Unloaded));
        assert_eq!(gpu.devices()[0].status, "unavailable");
    }
    #[test]
    fn unbound_active_nvidia_card_does_not_initialize_nvml() {
        let dir = TempCardDir::new("unbound");
        fs::write(dir.path().join("vendor"), "0x10de\n").unwrap();
        dir.set_power("active");
        let card = Card::discover(dir.path()).unwrap();
        assert!(!card.has_nvidia_driver());

        let mut gpu: Gpu<FakeNvidia> = Gpu::with_backend(false);
        gpu.enabled = true;
        gpu.cards.push(card);
        let mut inits = 0;
        for _ in 0..5 {
            gpu.read_with(|| {
                inits += 1;
                FakeNvidia::new(false)
            });
        }
        assert_eq!(inits, 0);
        assert!(matches!(gpu.nvidia, NvidiaState::Unloaded));
        assert_eq!(gpu.devices()[0].status, "unavailable");
    }
    #[test]
    fn failed_nvml_init_stays_latched_until_bound_cards_change() {
        let dir = TempCardDir::new("failed");
        fs::write(dir.path().join("vendor"), "0x10de\n").unwrap();
        dir.set_power("active");
        dir.bind_driver("nvidia");
        let card = Card::discover(dir.path()).unwrap();
        assert!(card.has_nvidia_driver());

        let mut gpu: Gpu<FakeNvidia> = Gpu::with_backend(false);
        gpu.enabled = true;
        gpu.cards.push(card);
        let mut inits = 0;
        for _ in 0..5 {
            gpu.read_with(|| {
                inits += 1;
                FakeNvidia::new(false)
            });
            gpu.reconcile_nvidia_failure();
        }
        assert_eq!(inits, 1);
        assert!(matches!(gpu.nvidia, NvidiaState::Failed { .. }));
        assert_eq!(gpu.devices()[0].status, "unavailable");

        // Unbinding the driver clears the failure latch so a later reload retries once.
        gpu.cards[0].driver = None;
        gpu.reconcile_nvidia_failure();
        assert!(matches!(gpu.nvidia, NvidiaState::Unloaded));

        gpu.cards[0].driver = Some("nvidia".into());
        gpu.read_with(|| {
            inits += 1;
            FakeNvidia::new(true)
        });
        assert_eq!(inits, 2);
        assert!(matches!(gpu.nvidia, NvidiaState::Ready(_)));
        assert_eq!(gpu.devices()[0].status, "available");
    }
    #[test]
    fn sleep_and_wake_reuses_single_nvml_handle() {
        let dir = TempCardDir::new("sleep-wake");
        fs::write(dir.path().join("vendor"), "0x10de\n").unwrap();
        dir.set_power("active");
        dir.bind_driver("nvidia");
        let card = Card::discover(dir.path()).unwrap();

        let mut gpu: Gpu<FakeNvidia> = Gpu::with_backend(false);
        gpu.enabled = true;
        gpu.cards.push(card);
        let mut inits = 0;

        let reading = gpu.read_with(|| {
            inits += 1;
            FakeNvidia::new(true)
        });
        assert_eq!(inits, 1);
        assert!(reading.is_some());
        assert_eq!(gpu.devices()[0].status, "available");

        dir.set_power("suspended");
        let reading = gpu.read_with(|| {
            inits += 1;
            FakeNvidia::new(true)
        });
        assert_eq!(inits, 1);
        assert!(reading.is_none());
        assert_eq!(gpu.devices()[0].status, "sleeping");
        let NvidiaState::Ready(backend) = &gpu.nvidia else {
            panic!("NVML handle must stay alive across adapter suspend");
        };
        assert_eq!(backend.clears, 1);

        dir.set_power("active");
        let reading = gpu.read_with(|| {
            inits += 1;
            FakeNvidia::new(true)
        });
        assert_eq!(inits, 1);
        assert!(reading.is_some());
        assert_eq!(gpu.devices()[0].status, "available");
        let NvidiaState::Ready(backend) = &gpu.nvidia else {
            panic!("NVML handle must remain ready after wake");
        };
        assert_eq!(backend.reads, 2);
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
    #[cfg(feature = "nvidia")]
    #[test]
    fn awake_nvidia_telemetry_matches_the_native_driver() {
        let mut gpu = Gpu::new(true);
        let mut expected = Vec::new();
        for card in &mut gpu.cards {
            if !card.has_nvidia_driver()
                || card
                    .power
                    .as_mut()
                    .and_then(|file| file.read().ok())
                    .is_none_or(|v| v.trim() != "active")
            {
                continue;
            }
            // Run the independent reference outside this process: NVML's
            // process-global initialization count could otherwise mask a bad
            // initialization flag in the collector under test. Never enumerate
            // other adapters, and never ask the tool to wake a sleeping one.
            let Ok(output) = std::process::Command::new("nvidia-smi")
                .args([
                    "--id",
                    &card.id,
                    "--query-gpu=name",
                    "--format=csv,noheader",
                ])
                .output()
            else {
                continue;
            };
            if output.status.success()
                && let Ok(name) = String::from_utf8(output.stdout)
            {
                expected.push((format!("pci:{}", card.id), gpu::label(name.trim().into())));
            }
        }
        gpu.read();
        for (id, name) in expected {
            let device = gpu.devices().iter().find(|device| device.id == id).unwrap();
            if device.status == "sleeping" {
                continue; // The adapter may have suspended after the reference.
            }
            assert_eq!(device.name, name);
            assert_eq!(device.status, "available");
            assert!(
                device.usage.is_some()
                    || device.dedicated_used.is_some()
                    || device.temperature.is_some()
            );
        }
    }
}
