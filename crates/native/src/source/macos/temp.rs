//! macOS temperatures from AppleSMC keys, classified once at startup.
//!
//! SMC key names differ between Intel and Apple silicon generations, so the
//! reader enumerates every temperature key the machine reports once and sorts
//! them by prefix: `Tp*`/`Te*`/`Tf0*`/`TC*` are processor, `Tg*`/`Tf1*`/`Tf2*`/
//! `TGDD` are graphics, `TH*` is storage. A machine that reports none simply
//! yields `null`.

use super::hid::Hid;
use super::mean;
use super::smc::{Key, Smc};
use std::time::{Duration, Instant};

pub(super) struct Temperatures {
    smc: Option<Smc>,
    cpu_keys: Vec<Key>,
    gpu_keys: Vec<Key>,
    disk_keys: Vec<Key>,
    hid: Option<Hid>,
    hid_discovered: Option<Instant>,
}

impl Temperatures {
    pub(super) fn new() -> Self {
        let smc = Smc::new();
        let [cpu_keys, gpu_keys, disk_keys] =
            smc.as_ref().map(Smc::temperature_keys).unwrap_or_default();
        Self {
            smc,
            cpu_keys,
            gpu_keys,
            disk_keys,
            hid: None,
            hid_discovered: None,
        }
    }

    pub(super) fn cpu(&mut self) -> Option<f32> {
        self.average(&self.cpu_keys).or_else(|| self.hid()?.cpu())
    }

    pub(super) fn gpu(&self) -> Option<f32> {
        self.average(&self.gpu_keys)
    }

    pub(super) fn disk(&self) -> Option<f32> {
        self.average(&self.disk_keys)
    }

    fn average(&self, keys: &[Key]) -> Option<f32> {
        let smc = self.smc.as_ref()?;
        let (sum, count) = keys
            .iter()
            .filter_map(|key| smc.temperature(*key).map(f64::from))
            .filter(|value| (10.0..=120.0).contains(value))
            .fold((0.0, 0), |(sum, count), value| (sum + value, count + 1));
        mean(sum, count)
    }

    pub(super) fn gpu_hid(&mut self) -> Option<f32> {
        self.hid()?.gpu()
    }

    fn hid(&mut self) -> Option<&Hid> {
        // Refresh discovery occasionally to recover after wake or permission
        // changes; an unavailable provider is not retried on every tick.
        if self
            .hid_discovered
            .is_none_or(|then| then.elapsed() >= Duration::from_secs(30))
        {
            self.hid = Hid::new();
            self.hid_discovered = Some(Instant::now());
        }
        self.hid.as_ref()
    }
}
