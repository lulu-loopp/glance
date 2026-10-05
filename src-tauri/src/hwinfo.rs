//! Sensors that only a kernel driver can read (CPU temperature and power,
//! fans), taken from HWiNFO when it shares them. HWiNFO publishes its
//! readings in a named shared memory block once "Shared Memory Support" is
//! enabled in its settings; nothing here needs a driver or administrator
//! rights of its own.

use serde::Serialize;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
};

#[derive(Clone, Serialize)]
pub struct HwSensors {
    pub cpu_temp: Option<f32>,
    pub cpu_power: Option<f32>,
    /// Every fan that is turning, by the name HWiNFO shows, in RPM.
    pub fans: Vec<(String, f32)>,
}

/// "HWiS" when the block holds live readings; HWiNFO writes "DEAD" on exit.
const SIGNATURE: u32 = u32::from_le_bytes(*b"HWiS");

// Reading types.
const TEMPERATURE: u32 = 1;
const FAN: u32 = 3;
const POWER: u32 = 5;

/// HWiNFO's own names for the CPU temperature, most telling first: AMD's
/// control temperature, Intel's package temperature, then the fallbacks.
const CPU_TEMPERATURES: [&str; 4] = ["CPU (Tctl/Tdie)", "CPU Package", "CPU (Tctl)", "CPU (Tdie)"];
const CPU_POWER: &str = "CPU Package Power";

/// The block's header (packed, as HWiNFO writes it).
#[repr(C, packed)]
struct Header {
    signature: u32,
    version: u32,
    revision: u32,
    poll_time: i64,
    sensor_offset: u32,
    sensor_size: u32,
    sensor_count: u32,
    reading_offset: u32,
    reading_size: u32,
    reading_count: u32,
}

/// The start of each reading; later revisions append fields, so readings
/// are stepped through by the size the header gives.
#[repr(C, packed)]
struct Reading {
    kind: u32,
    sensor_index: u32,
    reading_id: u32,
    label_original: [u8; 128],
    label_user: [u8; 128],
    unit: [u8; 16],
    value: f64,
}

pub struct Hwinfo {
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

// The view is read-only memory owned by this struct.
unsafe impl Send for Hwinfo {}

impl Hwinfo {
    /// Opens the block, if HWiNFO is sharing one.
    pub fn open() -> Option<Self> {
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, w!("Global\\HWiNFO_SENS_SM2")) }.ok()?;
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
        if view.Value.is_null() {
            let _ = unsafe { CloseHandle(mapping) };
            return None;
        }
        Some(Hwinfo { mapping, view })
    }

    /// The current readings; `None` once HWiNFO has stopped sharing.
    pub fn read(&self) -> Option<HwSensors> {
        unsafe { parse(self.view.Value as *const u8) }
    }
}

/// Reads a block laid out as HWiNFO lays it out, starting at `base`.
unsafe fn parse(base: *const u8) -> Option<HwSensors> {
    let header = unsafe { std::ptr::read_unaligned(base as *const Header) };
    if header.signature != SIGNATURE {
        return None;
    }
    let mut temps: Vec<(usize, f32)> = Vec::new();
    let mut sensors = HwSensors { cpu_temp: None, cpu_power: None, fans: Vec::new() };
    for index in 0..header.reading_count as usize {
        let at = header.reading_offset as usize + index * header.reading_size as usize;
        let reading = unsafe { std::ptr::read_unaligned(base.add(at) as *const Reading) };
        let original = text(&reading.label_original);
        let value = reading.value as f32;
        match reading.kind {
            TEMPERATURE => {
                if let Some(rank) = CPU_TEMPERATURES.iter().position(|name| *name == original) {
                    temps.push((rank, value));
                }
            }
            POWER if original == CPU_POWER && sensors.cpu_power.is_none() => sensors.cpu_power = Some(value),
            FAN if value > 0.0 => sensors.fans.push((text(&reading.label_user), value)),
            _ => {}
        }
    }
    sensors.cpu_temp = temps.into_iter().min_by_key(|(rank, _)| *rank).map(|(_, value)| value);
    Some(sensors)
}

impl Drop for Hwinfo {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
        }
    }
}

/// A zero-terminated string in the system code page, which is how HWiNFO
/// writes labels (a user can rename them in their own language).
fn text(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..len];
    let mut wide = vec![0u16; bytes.len()];
    let written = unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
    String::from_utf16_lossy(&wide[..written.max(0) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(kind: u32, label: &str, value: f64) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(kind.to_le_bytes());
        bytes.extend([0u8; 8]);
        let mut name = [0u8; 128];
        name[..label.len()].copy_from_slice(label.as_bytes());
        bytes.extend(name);
        bytes.extend(name);
        bytes.extend([0u8; 16]);
        bytes.extend(value.to_le_bytes());
        // Fields later revisions append, which the reader steps over.
        bytes.extend([0u8; 40]);
        bytes
    }

    #[test]
    fn picks_the_cpu_sensors_and_turning_fans() {
        let readings = [
            reading(TEMPERATURE, "Core Max", 70.0),
            reading(TEMPERATURE, "CPU (Tctl)", 61.0),
            reading(TEMPERATURE, "CPU (Tctl/Tdie)", 62.5),
            reading(POWER, "CPU Package Power", 88.25),
            reading(FAN, "CPU", 1200.0),
            reading(FAN, "System 2", 0.0),
        ];
        let size = readings[0].len() as u32;
        let header_len = size_of::<Header>() as u32;
        let mut block = Vec::new();
        block.extend(SIGNATURE.to_le_bytes());
        block.extend(2u32.to_le_bytes());
        block.extend(0u32.to_le_bytes());
        block.extend(0i64.to_le_bytes());
        block.extend([header_len, 0, 0, header_len, size, readings.len() as u32].iter().flat_map(|v| v.to_le_bytes()));
        readings.iter().for_each(|r| block.extend(r));

        let sensors = unsafe { parse(block.as_ptr()) }.unwrap();
        assert_eq!(sensors.cpu_temp, Some(62.5));
        assert_eq!(sensors.cpu_power, Some(88.25));
        assert_eq!(sensors.fans, vec![("CPU".to_string(), 1200.0)]);

        block[..4].copy_from_slice(b"DEAD");
        assert!(unsafe { parse(block.as_ptr()) }.is_none());
    }
}
