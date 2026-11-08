//! Windows physical interface throughput from network counters.
//!
//! sysinfo does not expose an interface's media type, so "physical" is
//! approximated by excluding the names Windows gives its software adapters:
//! loopback, pseudo-interfaces, tunnels, and the virtual switches that
//! hypervisors and VPN clients create.

use crate::source::counters::Counter;
use std::{collections::BTreeMap, time::Instant};
use sysinfo::Networks;

/// Substrings that mark an adapter as software-emulated rather than a wire.
const VIRTUAL_ADAPTERS: &[&str] = &[
    "loopback",
    "pseudo",
    "teredo",
    "isatap",
    "6to4",
    "tunnel",
    "bluetooth",
    "vethernet",
    "hyper-v",
    "wsl",
    "virtualbox",
    "vmware",
    "tap-",
    "tailscale",
    "wireguard",
    "zerotier",
    "docker",
];

pub(super) struct Network {
    networks: Networks,
    counters: BTreeMap<String, Counter>,
}

pub(super) struct Throughput {
    pub(super) rx: u64,
    pub(super) tx: u64,
}

impl Network {
    pub(super) fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            counters: BTreeMap::new(),
        }
    }

    /// Bytes received and transmitted per second across physical interfaces.
    ///
    /// The first call establishes the baseline and reports nothing, because a
    /// rate needs two samples.
    pub(super) fn throughput(&mut self) -> Option<Throughput> {
        self.networks.refresh(true);
        self.counters
            .retain(|name, _| self.networks.contains_key(name));
        let now = Instant::now();
        let mut total = Throughput { rx: 0, tx: 0 };
        let mut found = false;
        for (name, data) in self.networks.iter() {
            if VIRTUAL_ADAPTERS
                .iter()
                .any(|marker| contains_marker(name, marker))
            {
                continue;
            }
            // Allocate a key only when an interface first appears.
            if !self.counters.contains_key(name) {
                self.counters.insert(name.clone(), Counter::default());
            }
            if let Some((rx, tx)) = self.counters.get_mut(name).unwrap().observe(
                data.total_received(),
                data.total_transmitted(),
                now,
            ) {
                total.rx = total.rx.saturating_add(rx);
                total.tx = total.tx.saturating_add(tx);
                found = true;
            }
        }
        found.then_some(total)
    }
}

/// ASCII-insensitive substring test, so classifying an adapter costs no
/// lowercase copy on every tick. The markers are all ASCII, which is all this
/// needs to fold.
fn contains_marker(name: &str, marker: &str) -> bool {
    name.as_bytes()
        .windows(marker.len())
        .any(|window| window.eq_ignore_ascii_case(marker.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_names_match_case_insensitively() {
        assert!(contains_marker("Tailscale Tunnel", "tailscale"));
        assert!(contains_marker(
            "Hyper-V Virtual Ethernet Adapter",
            "hyper-v"
        ));
        assert!(!contains_marker("Intel Ethernet Controller", "vmware"));
        assert!(!contains_marker("Wi-Fi", "wsl"));
    }
}
