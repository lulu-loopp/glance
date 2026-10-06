//! The System Management Controller, read through IOKit's AppleSMC service
//! (no rights needed): its four-letter keys hold the fans' speeds.

use std::ffi::{c_void, CString};

use objc2_io_kit::{io_connect_t, kIOMainPortDefault, IOConnectCallStructMethod, IOServiceClose, IOServiceGetMatchingService, IOServiceMatching, IOServiceOpen};
use objc2_core_foundation::{CFDictionary, CFRetained};

/// The AppleSMC user client's one method, and what it is asked to do.
const HANDLE_EVENT: u32 = 2;
const READ_KEY_INFO: u8 = 9;
const READ_BYTES: u8 = 5;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Version {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PowerLimits {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    size: u32,
    kind: u32,
    attributes: u8,
}

/// What the SMC is asked, and answers, in one exchange.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Exchange {
    key: u32,
    version: Version,
    limits: PowerLimits,
    info: KeyInfo,
    result: u8,
    status: u8,
    command: u8,
    data: u32,
    bytes: [u8; 32],
}

// The layout the AppleSMC user client expects, to the byte.
const _: () = assert!(size_of::<Exchange>() == 80);

/// A key's four letters as the SMC numbers them.
fn code(key: &str) -> u32 {
    key.bytes().fold(0, |code, byte| code << 8 | byte as u32)
}

/// A connection to the SMC, closed when dropped.
pub struct Smc(io_connect_t);

impl Drop for Smc {
    fn drop(&mut self) {
        IOServiceClose(self.0);
    }
}

impl Smc {
    pub fn open() -> Option<Self> {
        let name = CString::new("AppleSMC").unwrap();
        let matching = unsafe { IOServiceMatching(name.as_ptr()) }?;
        let matching = unsafe { CFRetained::cast_unchecked::<CFDictionary>(matching) };
        let service = unsafe { IOServiceGetMatchingService(kIOMainPortDefault, Some(matching)) };
        if service == 0 {
            return None;
        }
        let mut connection: io_connect_t = 0;
        let opened = unsafe { IOServiceOpen(service, mach2::traps::mach_task_self(), 0, &mut connection) };
        objc2_io_kit::IOObjectRelease(service);
        (opened == 0).then_some(Smc(connection))
    }

    fn call(&self, input: &Exchange) -> Option<Exchange> {
        let mut output = Exchange::default();
        let mut size = size_of::<Exchange>();
        let status = unsafe {
            IOConnectCallStructMethod(
                self.0,
                HANDLE_EVENT,
                (input as *const Exchange).cast::<c_void>(),
                size_of::<Exchange>(),
                (&mut output as *mut Exchange).cast::<c_void>(),
                &mut size,
            )
        };
        (status == 0 && output.result == 0).then_some(output)
    }

    /// A key's value: its type's four letters, and its bytes.
    fn read(&self, key: &str) -> Option<(u32, Vec<u8>)> {
        let info = self.call(&Exchange { key: code(key), command: READ_KEY_INFO, ..Default::default() })?.info;
        let value = self.call(&Exchange { key: code(key), info, command: READ_BYTES, ..Default::default() })?;
        let size = (info.size as usize).min(value.bytes.len());
        Some((info.kind, value.bytes[..size].to_vec()))
    }

    /// A key that is a number: a float ("flt "), or an unsigned integer.
    pub fn number(&self, key: &str) -> Option<f32> {
        let (kind, bytes) = self.read(key)?;
        match (kind, bytes.as_slice()) {
            (k, [a, b, c, d]) if k == code("flt ") => Some(f32::from_le_bytes([*a, *b, *c, *d])),
            (k, [value]) if k == code("ui8 ") => Some(*value as f32),
            (k, [high, low]) if k == code("ui16") => Some(u16::from_be_bytes([*high, *low]) as f32),
            (k, [a, b, c, d]) if k == code("ui32") => Some(u32::from_be_bytes([*a, *b, *c, *d]) as f32),
            // The fixed point of Intel Macs' fans: 14 bits whole, 2 fraction.
            (k, [high, low]) if k == code("fpe2") => Some(u16::from_be_bytes([*high, *low]) as f32 / 4.0),
            _ => None,
        }
    }

    /// Each fan's speed now (RPM), numbered from 1.
    pub fn fans(&self) -> Vec<(String, f32)> {
        let count = self.number("FNum").unwrap_or(0.0) as usize;
        (0..count).filter_map(|i| Some(((i + 1).to_string(), self.number(&format!("F{i}Ac"))?))).collect()
    }
}
