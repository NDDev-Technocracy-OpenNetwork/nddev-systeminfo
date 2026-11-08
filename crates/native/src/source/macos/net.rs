//! macOS physical interface throughput from BSD interface counters.

use crate::source::counters::Counter;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io;
use std::ptr;
use std::{collections::BTreeMap, time::Instant};

pub(super) struct Network {
    counters: BTreeMap<u32, (Counter, bool)>,
}

pub(super) struct Throughput {
    pub(super) rx: u64,
    pub(super) tx: u64,
}

impl Network {
    pub(super) fn new() -> Self {
        Self {
            counters: BTreeMap::new(),
        }
    }

    /// Bytes received and transmitted per second across physical interfaces.
    ///
    /// `en*` covers Ethernet, Wi-Fi and USB tethering; loopback, `utun`
    /// tunnels and bridges never carry an `en` name, so they stay out.
    pub(super) fn throughput(&mut self) -> io::Result<Option<Throughput>> {
        read_counters(&mut self.counters)
    }
}

fn read_counters(counters: &mut BTreeMap<u32, (Counter, bool)>) -> io::Result<Option<Throughput>> {
    let mut addresses: *mut libc::ifaddrs = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut addresses) } != 0 {
        return Err(io::Error::last_os_error());
    }
    for (_, seen) in counters.values_mut() {
        *seen = false;
    }
    let now = Instant::now();
    let mut total = Throughput { rx: 0, tx: 0 };
    let mut found = false;
    let mut current = addresses;
    while !current.is_null() {
        let item = unsafe { &*current };
        if !item.ifa_addr.is_null()
            && unsafe { (*item.ifa_addr).sa_family as c_int } == libc::AF_LINK
            && !item.ifa_data.is_null()
        {
            let name = unsafe { CStr::from_ptr(item.ifa_name as *const c_char) };
            let flags = item.ifa_flags as c_int;
            // Byte compare: interface names are ASCII, and the filter runs per
            // interface per tick — no reason to allocate a String for it.
            if name.to_bytes().starts_with(b"en") && flags & libc::IFF_UP != 0 {
                let data = unsafe { &*(item.ifa_data as *const libc::if_data) };
                // SAFETY: ifaddrs provides a live NUL-terminated interface name.
                let identifier = unsafe { libc::if_nametoindex(item.ifa_name) };
                if identifier != 0 {
                    let (counter, seen) = counters.entry(identifier).or_default();
                    *seen = true;
                    if let Some((rx, tx)) =
                        counter.observe(data.ifi_ibytes as u64, data.ifi_obytes as u64, now)
                    {
                        total.rx = total.rx.saturating_add(rx);
                        total.tx = total.tx.saturating_add(tx);
                        found = true;
                    }
                }
            }
        }
        current = item.ifa_next;
    }
    unsafe { libc::freeifaddrs(addresses) };
    counters.retain(|_, (_, seen)| *seen);
    Ok(found.then_some(total))
}
