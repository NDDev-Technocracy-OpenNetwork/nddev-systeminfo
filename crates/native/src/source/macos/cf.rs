//! A single owner for every CoreFoundation Create/Copy/Retain reference.
use core_foundation_sys::{base::*, number::*, string::*};
use std::{ffi::CStr, ptr};

pub(super) struct Owned(pub(super) CFTypeRef);

impl Owned {
    // SAFETY: only +1 references from Create/Copy/Retain enter this wrapper.
    pub(super) unsafe fn take(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
    pub(super) fn string(value: &CStr) -> Option<Self> {
        unsafe {
            Self::take(
                CFStringCreateWithCString(ptr::null(), value.as_ptr(), kCFStringEncodingUTF8)
                    .cast(),
            )
        }
    }
    pub(super) fn integer(value: i32) -> Option<Self> {
        unsafe {
            Self::take(
                CFNumberCreate(
                    ptr::null(),
                    kCFNumberSInt32Type,
                    (&value as *const i32).cast(),
                )
                .cast(),
            )
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
