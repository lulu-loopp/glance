//! Temperatures on Apple silicon, from the sensors the system's HID event
//! system offers (the way the system itself and other monitors read them):
//! private API, declared here, which needs no rights to call.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDEventSystemClientCreate(allocator: *const c_void) -> *mut CFType;
    fn IOHIDEventSystemClientSetMatching(client: &CFType, matching: &CFDictionary) -> i32;
    fn IOHIDEventSystemClientCopyServices(client: &CFType) -> *mut CFArray;
    fn IOHIDServiceClientCopyProperty(service: *const c_void, key: &CFString) -> *mut CFType;
    fn IOHIDServiceClientCopyEvent(service: *const c_void, kind: i64, options: i32, timestamp: i64) -> *mut CFType;
    fn IOHIDEventGetFloatValue(event: &CFType, field: i32) -> f64;
}

/// The vendor usage page and usage of the system's temperature sensors.
const SENSOR_PAGE: i32 = 0xFF00;
const TEMPERATURE_SENSOR: i32 = 5;
/// The HID event type of a temperature, and its value's field.
const TEMPERATURE_EVENT: i64 = 15;
const TEMPERATURE_FIELD: i32 = (TEMPERATURE_EVENT as i32) << 16;

/// The temperature sensors, found once.
pub struct Sensors {
    _client: CFRetained<CFType>,
    services: CFRetained<CFArray>,
    names: Vec<String>,
}

impl Sensors {
    pub fn open() -> Option<Self> {
        let client = NonNull::new(unsafe { IOHIDEventSystemClientCreate(std::ptr::null()) })?;
        let client = unsafe { CFRetained::from_raw(client) };
        let keys = [CFString::from_str("PrimaryUsagePage"), CFString::from_str("PrimaryUsage")];
        let values = [CFNumber::new_i32(SENSOR_PAGE), CFNumber::new_i32(TEMPERATURE_SENSOR)];
        let matching = CFDictionary::from_slices(&[&*keys[0], &*keys[1]], &[&*values[0], &*values[1]]);
        unsafe { IOHIDEventSystemClientSetMatching(&client, matching.as_ref()) };
        let services = NonNull::new(unsafe { IOHIDEventSystemClientCopyServices(&client) })?;
        let services = unsafe { CFRetained::from_raw(services) };
        let product = CFString::from_str("Product");
        let names = (0..services.count())
            .map(|i| {
                let service = unsafe { services.value_at_index(i) };
                NonNull::new(unsafe { IOHIDServiceClientCopyProperty(service, &product) })
                    .map(|name| unsafe { CFRetained::from_raw(name) })
                    .and_then(|name| name.downcast::<CFString>().ok())
                    .map_or_else(String::new, |name| name.to_string())
            })
            .collect();
        Some(Sensors { _client: client, services, names })
    }

    /// Each sensor's name and its temperature now (°C); a sensor that does
    /// not answer is left out.
    pub fn read(&self) -> Vec<(&str, f32)> {
        self.names
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let service = unsafe { self.services.value_at_index(i as isize) };
                let event = NonNull::new(unsafe { IOHIDServiceClientCopyEvent(service, TEMPERATURE_EVENT, 0, 0) })?;
                let event = unsafe { CFRetained::from_raw(event) };
                let value = unsafe { IOHIDEventGetFloatValue(&event, TEMPERATURE_FIELD) } as f32;
                // A sensor not yet sampled reads as nothing, or as nonsense.
                (value > 0.0 && value < 150.0).then_some((name.as_str(), value))
            })
            .collect()
    }
}
