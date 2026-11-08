//! Read-only AppleSMC interface. No fan control or key writes are exposed.
use std::{ffi::c_void, mem::size_of};

#[repr(C)]
#[derive(Default)]
struct Version {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}
#[repr(C)]
#[derive(Default)]
struct PowerLimits {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    memory: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyInfo {
    size: u32,
    kind: u32,
    attributes: u8,
    reserved: [u8; 3],
}
#[repr(C)]
#[derive(Default)]
struct Message {
    key: u32,
    version: Version,
    version_padding: [u8; 2],
    limits: PowerLimits,
    info: KeyInfo,
    result: u8,
    status: u8,
    command: u8,
    command_padding: u8,
    index: u32,
    bytes: [u8; 32],
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(
        name: *const std::ffi::c_char,
    ) -> core_foundation_sys::dictionary::CFMutableDictionaryRef;
    fn IOServiceGetMatchingService(
        port: u32,
        matching: core_foundation_sys::dictionary::CFDictionaryRef,
    ) -> u32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOConnectCallStructMethod(
        connection: u32,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
}
unsafe extern "C" {
    static mach_task_self_: u32;
}

pub(super) struct Smc(u32);
#[derive(Clone, Copy)]
pub(super) struct Key {
    name: u32,
    info: KeyInfo,
}
impl Smc {
    pub(super) fn new() -> Option<Self> {
        // SAFETY: static NUL-terminated class name. IOKit consumes the matching
        // dictionary and returns one owned service send right, released below.
        let matching = unsafe { IOServiceMatching(c"AppleSMC".as_ptr()) };
        if matching.is_null() {
            return None;
        }
        let service = unsafe { IOServiceGetMatchingService(0, matching) };
        if service == 0 {
            return None;
        }
        let mut connection = 0;
        // SAFETY: live service, current task, valid initialized output storage.
        let result = unsafe { IOServiceOpen(service, mach_task_self_, 0, &mut connection) };
        unsafe { IOObjectRelease(service) };
        (result == 0 && connection != 0).then_some(Self(connection))
    }
    fn call(&self, input: &Message) -> Option<Message> {
        let mut output = Message::default();
        let mut length = size_of::<Message>();
        // SAFETY: owned connection and correctly aligned 80-byte C-layout
        // messages. The kernel cannot write beyond output's declared capacity.
        let result = unsafe {
            IOConnectCallStructMethod(
                self.0,
                2,
                (input as *const Message).cast(),
                size_of::<Message>(),
                (&mut output as *mut Message).cast(),
                &mut length,
            )
        };
        (result == 0 && length == size_of::<Message>() && output.result == 0).then_some(output)
    }
    fn key(&self, name: u32) -> Option<Key> {
        self.call(&Message {
            key: name,
            command: 9,
            ..Message::default()
        })
        .map(|message| Key {
            name,
            info: message.info,
        })
    }
    fn read(&self, key: Key) -> Option<[u8; 32]> {
        if key.info.size == 0 || key.info.size > 32 {
            return None;
        }
        self.call(&Message {
            key: key.name,
            info: key.info,
            command: 5,
            ..Message::default()
        })
        .map(|message| message.bytes)
    }
    pub(super) fn temperature_keys(&self) -> [Vec<Key>; 3] {
        let mut groups = [Vec::new(), Vec::new(), Vec::new()];
        let Some(count) = self
            .key(u32::from_be_bytes(*b"#KEY"))
            .and_then(|key| self.read(key))
            .map(|bytes| u32::from_be_bytes(bytes[..4].try_into().unwrap()))
        else {
            return groups;
        };
        // A hard safety ceiling bounds discovery even on a broken driver.
        for index in 0..count.min(4096) {
            let Some(message) = self.call(&Message {
                command: 8,
                index,
                ..Message::default()
            }) else {
                continue;
            };
            let bytes = message.key.to_be_bytes();
            let group = if bytes.starts_with(b"Tp")
                || bytes.starts_with(b"Te")
                || bytes.starts_with(b"Tf0")
                || bytes.starts_with(b"TC")
            {
                0
            } else if bytes.starts_with(b"Tg")
                || bytes.starts_with(b"Tf1")
                || bytes.starts_with(b"Tf2")
                || bytes == *b"TGDD"
            {
                1
            } else if bytes.starts_with(b"TH") {
                2
            } else {
                continue;
            };
            let Some(key) = self.key(message.key) else {
                continue;
            };
            if (key.info.kind == u32::from_be_bytes(*b"sp78") && key.info.size == 2)
                || (key.info.kind == u32::from_be_bytes(*b"flt ") && key.info.size == 4)
            {
                groups[group].push(key);
            }
        }
        groups
    }
    pub(super) fn temperature(&self, key: Key) -> Option<f32> {
        let bytes = self.read(key)?;
        let value = if key.info.kind == u32::from_be_bytes(*b"sp78") {
            f32::from(i16::from_be_bytes([bytes[0], bytes[1]])) / 256.0
        } else if key.info.kind == u32::from_be_bytes(*b"flt ") {
            f32::from_le_bytes(bytes[..4].try_into().unwrap())
        } else {
            return None;
        };
        (value.is_finite() && (10.0..=120.0).contains(&value)).then_some(value)
    }
}
impl Drop for Smc {
    fn drop(&mut self) {
        /* SAFETY: this value owns the sole open connection. */
        unsafe { IOServiceClose(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_message_layout_matches_the_smc_interface() {
        assert_eq!(size_of::<Message>(), 80);
        assert_eq!(std::mem::offset_of!(Message, bytes), 48);
    }
    #[test]
    fn read_only_native_lifecycle_tolerates_missing_smc() {
        for _ in 0..4 {
            if let Some(smc) = Smc::new() {
                for group in smc.temperature_keys() {
                    for key in group {
                        if let Some(value) = smc.temperature(key) {
                            assert!((10.0..=120.0).contains(&value));
                        }
                    }
                }
            }
        }
    }
}
