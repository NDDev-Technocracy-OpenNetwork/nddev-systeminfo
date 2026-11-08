//! Cache IOKit services and key strings; read only the requested numbers.
use super::cf::Owned;
use core_foundation_sys::{base::*, dictionary::*, number::*};
use std::{
    ffi::CStr,
    ptr,
    time::{Duration, Instant},
};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const std::ffi::c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        port: u32,
        matching: CFDictionaryRef,
        iterator: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryGetRegistryEntryID(entry: u32, identifier: *mut u64) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: core_foundation_sys::string::CFStringRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
}

struct Object(u32);
impl Drop for Object {
    fn drop(&mut self) {
        /* SAFETY: this value owns one live IOKit send right. */
        unsafe { IOObjectRelease(self.0) };
    }
}

#[derive(Clone, Copy)]
pub(super) enum Number {
    Integer(i64),
    Decimal(f64),
}
impl Number {
    pub(super) fn unsigned(self) -> Option<u64> {
        match self {
            Self::Integer(value) => u64::try_from(value).ok(),
            Self::Decimal(_) => None,
        }
    }
    pub(super) fn decimal(self) -> f64 {
        match self {
            Self::Integer(value) => value as f64,
            Self::Decimal(value) => value,
        }
    }
}

pub(super) struct Registry<const N: usize> {
    class: &'static CStr,
    property: Owned,
    keys: [Owned; N],
    services: Vec<(u64, Object)>,
    discovered: Instant,
    generation: u64,
}
impl<const N: usize> Registry<N> {
    pub(super) fn new(class: &'static CStr, property: &CStr, keys: [&CStr; N]) -> Option<Self> {
        let keys: [Owned; N] = keys
            .into_iter()
            .map(Owned::string)
            .collect::<Option<Vec<_>>>()?
            .try_into()
            .ok()?;
        let mut registry = Self {
            class,
            property: Owned::string(property)?,
            keys,
            services: Vec::new(),
            discovered: Instant::now(),
            generation: 0,
        };
        registry.discover();
        Some(registry)
    }
    fn discover(&mut self) {
        let mut services = Vec::new();
        // SAFETY: static NUL-terminated class name; IOKit consumes matching.
        let matching = unsafe { IOServiceMatching(self.class.as_ptr()) };
        if !matching.is_null() {
            let mut iterator = 0;
            // SAFETY: initialized output storage; successful call transfers one iterator.
            if unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) } == 0
                && iterator != 0
            {
                let iterator = Object(iterator);
                // Bound discovery even if an unexpected driver exposes a large registry.
                for _ in 0..64 {
                    // SAFETY: owned iterator; each nonzero result is an owned send right.
                    let service = unsafe { IOIteratorNext(iterator.0) };
                    if service == 0 {
                        break;
                    }
                    let service = Object(service);
                    let mut identifier = 0;
                    // SAFETY: owned service and valid u64 output storage.
                    if unsafe { IORegistryEntryGetRegistryEntryID(service.0, &mut identifier) } == 0
                    {
                        services.push((identifier, service));
                    }
                }
            }
        }
        services.sort_unstable_by_key(|(identifier, _)| *identifier);
        if !services
            .iter()
            .map(|(identifier, _)| identifier)
            .eq(self.services.iter().map(|(identifier, _)| identifier))
        {
            self.generation = self.generation.wrapping_add(1);
        }
        self.services = services;
        self.discovered = Instant::now();
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn visit(&mut self, mut read: impl FnMut([Option<Number>; N])) -> bool {
        if self.discovered.elapsed() >= Duration::from_secs(30) {
            self.discover();
        }
        for (_, service) in &self.services {
            // SAFETY: live service, live CFString key and the default allocator.
            let dictionary = unsafe {
                Owned::take(IORegistryEntryCreateCFProperty(
                    service.0,
                    self.property.0.cast(),
                    ptr::null(),
                    0,
                ))
            };
            let Some(dictionary) = dictionary else {
                read([None; N]);
                continue;
            };
            // SAFETY: nonnull owned CF object; reject a provider's unexpected type.
            if unsafe { CFGetTypeID(dictionary.0) } != unsafe { CFDictionaryGetTypeID() } {
                read([None; N]);
                continue;
            }
            let values = std::array::from_fn(|index| {
                // SAFETY: dictionary and key remain alive; lookup returns a borrowed value.
                let value =
                    unsafe { CFDictionaryGetValue(dictionary.0.cast(), self.keys[index].0) };
                if value.is_null()
                    || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() }
                {
                    return None;
                }
                // SAFETY: a checked CFNumber and correctly typed output storage.
                if unsafe { CFNumberIsFloatType(value.cast()) } != 0 {
                    let mut number: f64 = 0.0;
                    (unsafe {
                        CFNumberGetValue(
                            value.cast(),
                            kCFNumberFloat64Type,
                            (&mut number as *mut f64).cast(),
                        )
                    } && number.is_finite())
                    .then_some(Number::Decimal(number))
                } else {
                    let mut number: i64 = 0;
                    (unsafe {
                        CFNumberGetValue(
                            value.cast(),
                            kCFNumberSInt64Type,
                            (&mut number as *mut i64).cast(),
                        )
                    })
                    .then_some(Number::Integer(number))
                }
            });
            read(values);
        }
        !self.services.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_handles_missing_classes_and_repeated_read_drop() {
        for _ in 0..8 {
            let mut missing =
                Registry::new(c"NDDevSysteminfoMissingClass", c"Statistics", [c"missing"]).unwrap();
            assert!(!missing.visit(|_| panic!("nonexistent service returned data")));
            let mut disks = Registry::new(
                c"IOBlockStorageDriver",
                c"Statistics",
                [c"Bytes (Read)", c"Bytes (Write)"],
            )
            .unwrap();
            disks.visit(|values| {
                for value in values.into_iter().flatten() {
                    assert!(value.unsigned().is_some());
                }
            });
        }
    }
}
