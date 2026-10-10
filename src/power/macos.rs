//! Public IOKit assertion; deliberately does not change global power settings.
use super::{Inhibitor, Kind, PowerApi, Protection};
use std::ffi::{CStr, c_char, c_void};

type CfRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(allocator: CfRef, value: *const c_char, encoding: u32) -> CfRef;
    fn CFRelease(value: CfRef);
    fn CFDictionaryGetValue(dictionary: CfRef, key: CfRef) -> CfRef;
    fn CFGetTypeID(value: CfRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CfRef, kind: isize, value: *mut c_void) -> u8;
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(kind: CfRef, level: u32, name: CfRef, id: *mut u32) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
    fn IOPMAssertionCopyProperties(id: u32) -> CfRef;
}

struct OwnedCf(CfRef);
impl OwnedCf {
    fn string(value: &CStr) -> Result<Self, Protection> {
        // The C string is valid for the call; Create returns a retained CFString.
        let value =
            unsafe { CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), 0x0800_0100) };
        if value.is_null() {
            return Err(Protection::Unavailable(
                "IOKit string allocation failed".into(),
            ));
        }
        Ok(Self(value))
    }
}
impl Drop for OwnedCf {
    fn drop(&mut self) {
        // OwnedCf is constructed only from non-null Create/Copy results.
        unsafe { CFRelease(self.0) };
    }
}

pub(super) struct Api;
impl PowerApi for Api {
    async fn acquire(&self, kind: Kind) -> Result<Box<dyn Inhibitor>, Protection> {
        match kind {
            Kind::Lid => {
                return Err(Protection::Unsupported(
                    "MacBook lid-close sleep is not overridden".into(),
                ));
            }
            Kind::Sleep => {
                return Err(Protection::Unsupported(
                    "explicit sleep and power/thermal policy are not overridden".into(),
                ));
            }
            Kind::Idle => {}
        }
        let kind = OwnedCf::string(c"PreventUserIdleSystemSleep")?;
        let name = OwnedCf::string(c"Herdr Relay: sharing sessions")?;
        let mut id = 0;
        // kIOPMAssertionLevelOn = 255; the returned ID belongs to this process.
        let result = unsafe { IOPMAssertionCreateWithName(kind.0, 255, name.0, &mut id) };
        if result != 0 {
            return Err(Protection::Unavailable(format!(
                "IOKit assertion failed ({result:#x})"
            )));
        }
        Ok(Box::new(Assertion(id)))
    }
}

struct Assertion(u32);
impl Inhibitor for Assertion {
    fn is_held(&self) -> bool {
        // Copy returns a retained dictionary, or null if the assertion was lost.
        let properties = unsafe { IOPMAssertionCopyProperties(self.0) };
        if properties.is_null() {
            return false;
        }
        let properties = OwnedCf(properties);
        let Ok(key) = OwnedCf::string(c"AssertLevel") else {
            return false;
        };
        let mut level: i32 = 0;
        // Validate the borrowed value before reading a kCFNumberIntType (9).
        unsafe {
            let value = CFDictionaryGetValue(properties.0, key.0);
            !value.is_null()
                && CFGetTypeID(value) == CFNumberGetTypeID()
                && CFNumberGetValue(value, 9, (&mut level as *mut i32).cast()) != 0
                && level == 255
        }
    }
}
impl Drop for Assertion {
    fn drop(&mut self) {
        // The OS also releases process-owned assertions on abnormal exit.
        unsafe { IOPMAssertionRelease(self.0) };
    }
}
