//! The only reader of application configuration. OS path variables are conventions.
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub const DEFAULT_TOML: &str = include_str!("../../config/default.toml");

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub sampling: Sampling,
    pub transport: Transport,
    pub display: Display,
    pub logging: Logging,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sampling {
    pub interval_seconds: u64,
    pub gpu: bool,
    pub idle_release_seconds: u64,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transport {
    pub max_clients: usize,
    pub handshake_bytes: usize,
    pub handshake_timeout_ms: u64,
    pub client_timeout_ms: u64,
    pub accept_stack_bytes: usize,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub locale: String,
    pub show_cpu: bool,
    pub show_memory: bool,
    pub show_gpu: bool,
    pub show_network: bool,
    pub show_temperatures: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    pub level: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read configuration: {0}")]
    Io(#[from] io::Error),
    #[error("invalid TOML configuration: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid configuration setting: {0}")]
    Invalid(&'static str),
    #[error("cannot serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        let (section, name) = key
            .split_once('.')
            .ok_or(ConfigError::Invalid("setting must be section.name"))?;
        let mut document = toml::Value::try_from(&*self)?;
        let slot = document
            .get_mut(section)
            .and_then(|section| section.get_mut(name))
            .ok_or(ConfigError::Invalid("unknown setting"))?;
        *slot = if slot.is_str() {
            toml::Value::String(value.to_string())
        } else {
            let input: toml::Value = toml::from_str(&format!("value = {value}"))?;
            input
                .get("value")
                .ok_or(ConfigError::Invalid("setting value missing"))?
                .clone()
        };
        let candidate: Self = document.try_into()?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn store(&self, path: &Path) -> Result<(), ConfigError> {
        use std::io::Write;
        self.validate()?;
        if !path.is_absolute() {
            return Err(ConfigError::Invalid("configuration path must be absolute"));
        }
        let parent = path
            .parent()
            .ok_or(ConfigError::Invalid("configuration parent missing"))?;
        let mut directory = std::fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(parent)?;
        let temporary = parent.join(format!(
            ".systeminfo-config-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| -> Result<(), ConfigError> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        if let Some(path) = path {
            return Self::parse(&std::fs::read_to_string(path)?);
        }
        match std::fs::read_to_string(default_path()?) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::parse(DEFAULT_TOML),
            Err(error) => Err(error.into()),
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let transport = self.transport;
        for (valid, key) in [
            (
                self.sampling.interval_seconds <= 60,
                "sampling.interval_seconds (0..60)",
            ),
            (
                (1..=3600).contains(&self.sampling.idle_release_seconds),
                "sampling.idle_release_seconds (1..3600)",
            ),
            (
                (1..=256).contains(&transport.max_clients),
                "transport.max_clients (1..256)",
            ),
            (
                (1..=4096).contains(&transport.handshake_bytes),
                "transport.handshake_bytes (1..4096)",
            ),
            (
                (1..=1000).contains(&transport.handshake_timeout_ms),
                "transport.handshake_timeout_ms (1..1000)",
            ),
            (
                (1..=60000).contains(&transport.client_timeout_ms),
                "transport.client_timeout_ms (1..60000)",
            ),
            (
                (65536..=1048576).contains(&transport.accept_stack_bytes),
                "transport.accept_stack_bytes (65536..1048576)",
            ),
            (
                ["system", "ru", "en", "kk", "ar", "zh"].contains(&self.display.locale.as_str()),
                "display.locale",
            ),
            (
                ["off", "error", "warn", "info", "debug", "trace"]
                    .contains(&self.logging.level.as_str()),
                "logging.level",
            ),
        ] {
            if !valid {
                return Err(ConfigError::Invalid(key));
            }
        }
        Ok(())
    }

    pub fn interval(&self) -> Duration {
        cadence(self.sampling.interval_seconds).expect("validated configuration")
    }
}

/// Wire v1 cadence, a protocol bound shared by all clients.
pub fn cadence(seconds: u64) -> Option<Duration> {
    match seconds {
        0 => Some(Duration::from_millis(500)),
        1..=60 => Some(Duration::from_secs(seconds)),
        _ => None,
    }
}

pub fn default_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let path = base
        .ok_or_else(|| io::Error::other("per-user configuration directory is unavailable"))?
        .join("nddev-systeminfo/config.toml");
    if !path.is_absolute() {
        return Err(io::Error::other("configuration directory must be absolute"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_configuration_is_complete_and_valid() {
        assert!(Config::parse(DEFAULT_TOML).is_ok());
    }
    #[test]
    fn missing_unknown_and_unbounded_settings_are_rejected() {
        let source = DEFAULT_TOML.replace("\r\n", "\n");
        for text in [
            String::new(),
            source.replace("gpu = false", "gpu = false\nunknown = 1"),
            source.replace("max_clients = 32", "max_clients = 0"),
            source.replace("interval_seconds = 5", "interval_seconds = 61"),
            source.replace("gpu = false\n", ""),
        ] {
            assert!(Config::parse(&text).is_err());
        }
    }
}
