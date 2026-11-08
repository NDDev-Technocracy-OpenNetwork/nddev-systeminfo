//! Standalone Systeminfo contract, client and configuration. No NDS initialization.
pub mod client;
pub mod config;
pub mod presentation;
pub mod telemetry;
pub use nddev_systeminfo_contract as protocol;
pub use nddev_systeminfo_native::Collector;
