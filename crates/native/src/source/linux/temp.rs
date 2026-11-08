//! Linux temperatures from `sysfs` hwmon nodes.
//!
//! hwmon indices are assigned in probe order and are not stable across boots,
//! so every sensor is resolved once by driver name at startup and then read
//! through a held descriptor.

use super::VirtualFile;
use std::io;
use std::path::{Path, PathBuf};

/// hwmon reports temperatures in thousandths of a degree Celsius.
const MILLIDEGREES: f32 = 1000.0;

/// Drivers exposing package temperature, in order of preference.
const CPU_DRIVERS: &[&str] = &["coretemp", "k10temp", "zenpower"];
/// Drivers exposing storage temperature.
const DISK_DRIVERS: &[&str] = &["nvme", "drivetemp"];
/// The Intel label for the whole-package sensor, which is the figure a user
/// recognises as "the CPU temperature" rather than one core's reading.
const PACKAGE_LABEL: &str = "Package id 0";

pub(super) struct Sensor {
    file: VirtualFile,
}

impl Sensor {
    /// Resolves the CPU package sensor, or `None` when no known driver is bound.
    pub(super) fn cpu() -> Option<Self> {
        let hwmon = find_driver(CPU_DRIVERS)?;
        let input =
            labelled_input(&hwmon, PACKAGE_LABEL).unwrap_or_else(|| hwmon.join("temp1_input"));
        Self::at(input)
    }

    /// Resolves the storage sensor, or `None` when the disk exposes none.
    pub(super) fn disk() -> Option<Self> {
        let hwmon = find_driver(DISK_DRIVERS)?;
        Self::at(hwmon.join("temp1_input"))
    }

    fn at(path: PathBuf) -> Option<Self> {
        VirtualFile::open(path).ok().map(|file| Self { file })
    }

    /// Current reading in degrees Celsius.
    pub(super) fn celsius(&mut self) -> io::Result<Option<f32>> {
        Ok(self
            .file
            .read()?
            .trim()
            .parse::<f32>()
            .ok()
            .map(|value| value / MILLIDEGREES))
    }
}

/// First hwmon node whose driver name matches, preferring earlier candidates.
fn find_driver(candidates: &[&str]) -> Option<PathBuf> {
    let mut best: Option<(usize, PathBuf)> = None;

    for entry in std::fs::read_dir("/sys/class/hwmon").ok()? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(name) = std::fs::read_to_string(path.join("name")) else {
            continue;
        };
        let Some(rank) = candidates.iter().position(|driver| *driver == name.trim()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(current, _)| rank < *current) {
            best = Some((rank, path));
        }
    }

    best.map(|(_, path)| path)
}

/// The `tempN_input` whose sibling `tempN_label` carries the wanted label.
fn labelled_input(hwmon: &Path, label: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(hwmon).ok()? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(index) = name
            .strip_suffix("_label")
            .and_then(|n| n.strip_prefix("temp"))
        else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if contents.trim() == label {
            return Some(hwmon.join(format!("temp{index}_input")));
        }
    }
    None
}
