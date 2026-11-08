//! Standalone Systeminfo contract, client and configuration. No NDS initialization.
pub mod client;
pub mod config;
pub mod installation;
pub mod presentation;
pub mod telemetry;
#[cfg(target_os = "windows")]
pub mod windows_ui;
pub use nddev_systeminfo_contract as protocol;
pub use nddev_systeminfo_native::Collector;
