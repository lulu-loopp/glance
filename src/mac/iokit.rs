//! Reading IOKit's registry: the services of a class, and their properties
//! as Core Foundation values.

use std::ffi::{c_void, CString};

use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
    io_object_t, kIOMainPortDefault, IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IORegistryEntryGetName,
    IORegistryEntryGetParentEntry, IORegistryEntryGetRegistryEntryID,
    IOServiceGetMatchingServices, IOServiceMatching,
};

/// A registry entry, released when dropped.
pub struct Entry(io_object_t);

impl Drop for Entry {
    fn drop(&mut self) {
        IOObjectRelease(self.0);
    }
}

impl Entry {
    /// One of a service's properties, if it has it.
    pub fn property(&self, key: &str) -> Option<CFRetained<CFType>> {
        let key = CFString::from_str(key);
        unsafe { IORegistryEntryCreateCFProperty(self.0, Some(&key), None, 0) }
    }

    /// A property that is a string.
    pub fn string(&self, key: &str) -> Option<String> {
        self.property(key)?.downcast::<CFString>().ok().map(|s| s.to_string())
    }

    /// A property that is raw bytes.
    pub fn data(&self, key: &str) -> Option<Vec<u8>> {
        self.property(key)?.downcast::<CFData>().ok().map(|data| data.to_vec())
    }

    /// Its name in the registry.
    pub fn name(&self) -> String {
        let mut name = [0 as std::ffi::c_char; 128];
        if unsafe { IORegistryEntryGetName(self.0, &mut name) } != 0 {
            return String::new();
        }
        unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned()
    }

    /// The number the registry knows it by for as long as it is there.
    pub fn id(&self) -> u64 {
        let mut id = 0;
        unsafe { IORegistryEntryGetRegistryEntryID(self.0, &mut id) };
        id
    }

    /// A property that is a dictionary.
    pub fn dictionary(&self, key: &str) -> Option<CFRetained<CFDictionary>> {
        self.property(key)?.downcast::<CFDictionary>().ok()
    }

    /// Its parent in the service plane.
    pub fn parent(&self) -> Option<Entry> {
        let mut parent: io_object_t = 0;
        let plane = CString::new("IOService").unwrap();
        let status = unsafe { IORegistryEntryGetParentEntry(self.0, plane.as_ptr() as *mut _, &mut parent) };
        (status == 0 && parent != 0).then_some(Entry(parent))
    }
}

/// The services of class `class` (and of its subclasses) there are now.
pub fn services(class: &str) -> Vec<Entry> {
    let name = CString::new(class).unwrap();
    let Some(matching) = (unsafe { IOServiceMatching(name.as_ptr()) }) else { return Vec::new() };
    let mut iterator: io_object_t = 0;
    // The matching dictionary is consumed by the call.
    // A mutable dictionary is a dictionary.
    let matching = unsafe { CFRetained::cast_unchecked::<CFDictionary>(matching) };
    if unsafe { IOServiceGetMatchingServices(kIOMainPortDefault, Some(matching), &mut iterator) } != 0 {
        return Vec::new();
    }
    let mut found = Vec::new();
    loop {
        let entry = IOIteratorNext(iterator);
        if entry == 0 {
            break;
        }
        found.push(Entry(entry));
    }
    IOObjectRelease(iterator);
    found
}

/// The value under `key` in a dictionary of string keys.
pub fn get(dictionary: &CFDictionary, key: &str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_str(key);
    let value = unsafe { dictionary.value((&*key as *const CFString).cast::<c_void>()) };
    // Borrowed from the dictionary: retained for the caller.
    std::ptr::NonNull::new(value as *mut CFType).map(|value| unsafe { CFRetained::retain(value) })
}

/// The number under `key`, as an integer.
pub fn number(dictionary: &CFDictionary, key: &str) -> Option<i64> {
    get(dictionary, key)?.downcast::<CFNumber>().ok()?.as_i64()
}
