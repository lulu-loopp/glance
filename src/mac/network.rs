//! The network as System Settings names it, through SystemConfiguration:
//! which interface traffic leaves by now, and what each interface is called.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFType};

use super::iokit;

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCDynamicStoreCreate(allocator: *const c_void, name: &CFString, callout: *const c_void, context: *const c_void) -> *mut CFType;
    fn SCDynamicStoreCopyValue(store: &CFType, key: &CFString) -> *mut CFType;
    fn SCNetworkInterfaceCopyAll() -> *mut CFArray;
    fn SCNetworkInterfaceGetBSDName(interface: *const c_void) -> *const CFString;
    fn SCNetworkInterfaceGetLocalizedDisplayName(interface: *const c_void) -> *const CFString;
}

pub struct Network {
    store: CFRetained<CFType>,
}

impl Network {
    pub fn open() -> Option<Self> {
        let name = CFString::from_str("Glance");
        let store = NonNull::new(unsafe { SCDynamicStoreCreate(std::ptr::null(), &name, std::ptr::null(), std::ptr::null()) })?;
        Some(Network { store: unsafe { CFRetained::from_raw(store) } })
    }

    /// The BSD name ("en0") of the interface the default route uses now.
    pub fn primary(&self) -> Option<String> {
        let key = CFString::from_str("State:/Network/Global/IPv4");
        let value = NonNull::new(unsafe { SCDynamicStoreCopyValue(&self.store, &key) })?;
        let value = unsafe { CFRetained::from_raw(value) }.downcast::<CFDictionary>().ok()?;
        iokit::get(&value, "PrimaryInterface")?.downcast::<CFString>().ok().map(|name| name.to_string())
    }
}

/// What System Settings calls the interface with BSD name `bsd` ("Wi-Fi",
/// "Ethernet"), in the user's language.
pub fn display_name(bsd: &str) -> Option<String> {
    let all = NonNull::new(unsafe { SCNetworkInterfaceCopyAll() })?;
    let all = unsafe { CFRetained::from_raw(all) };
    (0..all.count()).find_map(|i| {
        let interface = unsafe { all.value_at_index(i) };
        let name = unsafe { SCNetworkInterfaceGetBSDName(interface) };
        if name.is_null() || unsafe { &*name }.to_string() != bsd {
            return None;
        }
        let shown = unsafe { SCNetworkInterfaceGetLocalizedDisplayName(interface) };
        (!shown.is_null()).then(|| unsafe { &*shown }.to_string())
    })
}
