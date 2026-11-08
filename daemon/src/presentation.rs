//! Shared labels, formatting and released design tokens for native clients.
use crate::{config::Display, protocol::WireSnapshot};
pub use nddev_opennetwork_design_tokens::{Colors, Density, Direction, Metrics, Platform, Theme};
use std::collections::BTreeMap;

pub const ABSENT: &str = "—";
pub const LANGUAGES: [(&str, &str); 5] = [
    ("en", "English"),
    ("ru", "Русский"),
    ("kk", "Қазақша"),
    ("ar", "العربية"),
    ("zh", "中文"),
];

pub struct Catalog {
    pub locale: &'static str,
    pub rtl: bool,
    labels: BTreeMap<&'static str, &'static str>,
}
impl Catalog {
    pub fn load(requested: &str, system_languages: &[&str]) -> Self {
        let locale = if requested == "system" {
            system_languages
                .iter()
                .find_map(|value| language(value))
                .unwrap_or("en")
        } else {
            language(requested).expect("validated display locale")
        };
        let source = match locale {
            "ru" => include_str!("../../locales/ru.json"),
            "kk" => include_str!("../../locales/kk.json"),
            "ar" => include_str!("../../locales/ar.json"),
            "zh" => include_str!("../../locales/zh.json"),
            _ => include_str!("../../locales/en.json"),
        };
        Self {
            locale,
            rtl: locale == "ar",
            labels: serde_json::from_str(source).expect("checked locale catalog"),
        }
    }
    pub fn text(&self, key: &str) -> &str {
        self.labels.get(key).expect("checked message identifier")
    }
    pub fn reading(&self, key: &str, value: &str) -> String {
        // Isolate machine-formatted numbers from the surrounding writing direction.
        format!("{}: \u{2068}{value}\u{2069}", self.text(key))
    }
}
fn language(value: &str) -> Option<&'static str> {
    let base = value.split(['-', '_', '.', '@']).next()?;
    LANGUAGES
        .iter()
        .find_map(|(code, _)| base.eq_ignore_ascii_case(code).then_some(*code))
}

pub fn theme(display: &Display, dark: bool, high_contrast: bool) -> Theme {
    // An OS accessibility request always takes priority over the saved color theme.
    let dark = match display.theme.as_str() {
        "light" | "light-contrast" => false,
        "dark" | "dark-contrast" => true,
        _ => dark,
    };
    let contrast = high_contrast || display.theme.ends_with("-contrast");
    match (dark, contrast) {
        (false, false) => Theme::Light,
        (true, false) => Theme::Dark,
        (false, true) => Theme::LightContrast,
        (true, true) => Theme::DarkContrast,
    }
}
pub fn metrics(display: &Display) -> &'static Metrics {
    Metrics::resolve(
        Platform::Desktop,
        if display.density == "compact" {
            Density::Compact
        } else {
            Density::Comfortable
        },
    )
}
pub fn percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| ABSENT.into())
}
pub fn temperature(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.0} °C"))
        .unwrap_or_else(|| ABSENT.into())
}
pub fn rate(value: Option<u64>) -> String {
    let Some(value) = value else {
        return ABSENT.into();
    };
    let mut amount = value as f64;
    let mut index = 0;
    let units = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s", "PiB/s", "EiB/s"];
    while amount >= 1024.0 && index < units.len() - 1 {
        amount /= 1024.0;
        index += 1;
    }
    if index > 0 && amount < 10.0 {
        format!("{amount:.1} {}", units[index])
    } else {
        format!("{amount:.0} {}", units[index])
    }
}
pub fn readings(sample: Option<&WireSnapshot>) -> Vec<(&'static str, String)> {
    vec![
        ("cpu", percent(sample.and_then(|s| s.cpu.usage))),
        (
            "cpu_temperature",
            temperature(sample.and_then(|s| s.cpu.temp)),
        ),
        ("memory", percent(sample.and_then(|s| s.memory.used))),
        ("swap", percent(sample.and_then(|s| s.memory.swap))),
        ("gpu", percent(sample.and_then(|s| s.gpu.usage))),
        ("gpu_memory", percent(sample.and_then(|s| s.gpu.memory))),
        (
            "gpu_temperature",
            temperature(sample.and_then(|s| s.gpu.temp)),
        ),
        ("disk_read", rate(sample.and_then(|s| s.disk.read))),
        ("disk_write", rate(sample.and_then(|s| s.disk.write))),
        (
            "disk_temperature",
            temperature(sample.and_then(|s| s.disk.temp)),
        ),
        ("network_rx", rate(sample.and_then(|s| s.net.rx))),
        ("network_tx", rate(sample.and_then(|s| s.net.tx))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_readings_are_localized_and_offline_is_unknown() {
        for (language, _) in LANGUAGES {
            let catalog = Catalog::load(language, &[]);
            for (key, value) in readings(None) {
                assert!(!catalog.text(key).is_empty());
                assert_eq!(value, ABSENT);
            }
            assert_eq!(catalog.rtl, language == "ar");
        }
        assert_eq!(
            Catalog::load("system", &["de-DE", "kk_KZ.UTF-8"]).locale,
            "kk"
        );
        assert_eq!(rate(Some(1024)), "1.0 KiB/s");
        assert_eq!(rate(Some(0)), "0 B/s");
        assert_eq!(percent(Some(f64::NAN)), ABSENT);
    }
}
