//! Owned, read-only OS resources. Unsafe code is confined here (decision 0002).
use nddev_systeminfo_contract as proto;
mod collector;
mod source;
pub mod transport;
pub use collector::Collector;
#[cfg(target_os = "windows")]
pub mod process;
#[cfg(target_os = "windows")]
pub mod tray;
