//! Drive temperatures, as the drives report them to Windows. Asking for a
//! drive's properties needs no access to its data, so no administrator rights.

use crate::reading::DriveTemperature;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, StorageDeviceTemperatureProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    STORAGE_TEMPERATURE_INFO,
};
use windows::Win32::System::IO::DeviceIoControl;


/// Physical drives are numbered from zero; a gap this long means no more.
const MAX_GAP: u32 = 4;

/// The model name of every physical drive.
pub fn models() -> Vec<String> {
    let mut found = Vec::new();
    each_drive(|index, handle| found.push(model(handle).unwrap_or_else(|| format!("Disk {index}"))));
    found
}

/// Every physical drive that reports a temperature, by its model name.
pub fn temperatures() -> Vec<DriveTemperature> {
    let mut found = Vec::new();
    each_drive(|index, handle| {
        if let Some(celsius) = temperature(handle) {
            found.push(DriveTemperature { name: model(handle).unwrap_or_else(|| format!("Disk {index}")), celsius });
        }
    });
    found
}

/// Calls `visit` with each physical drive, opened for its properties only.
fn each_drive(mut visit: impl FnMut(u32, HANDLE)) {
    let mut misses = 0;
    for index in 0.. {
        let path: Vec<u16> = format!("\\\\.\\PhysicalDrive{index}").encode_utf16().chain([0]).collect();
        // Access 0: properties only, never the data.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
        let Ok(handle) = handle else {
            misses += 1;
            if misses >= MAX_GAP {
                break;
            }
            continue;
        };
        misses = 0;
        visit(index, handle);
        let _ = unsafe { CloseHandle(handle) };
    }
}

fn query(handle: HANDLE, property: STORAGE_PROPERTY_ID, buf: &mut [u64]) -> bool {
    let query = STORAGE_PROPERTY_QUERY { PropertyId: property, QueryType: PropertyStandardQuery, ..Default::default() };
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const _),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr() as *mut _),
            (buf.len() * 8) as u32,
            Some(&mut returned),
            None,
        )
    }
    .is_ok()
}

/// The drive's own temperature: the first of its sensors with a reading
/// (on NVMe drives the first is the composite temperature).
fn temperature(handle: HANDLE) -> Option<f32> {
    let mut buf = [0u64; 64];
    if !query(handle, StorageDeviceTemperatureProperty, &mut buf) {
        return None;
    }
    let descriptor = unsafe { &*(buf.as_ptr() as *const STORAGE_TEMPERATURE_DATA_DESCRIPTOR) };
    // The sensors that fit in the buffer, of those the drive lists.
    let room = (std::mem::size_of_val(&buf) - std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, TemperatureInfo))
        / std::mem::size_of::<STORAGE_TEMPERATURE_INFO>();
    let count = (descriptor.InfoCount as usize).min(room);
    let sensors = unsafe { std::slice::from_raw_parts(descriptor.TemperatureInfo.as_ptr(), count) };
    sensors.iter().map(|sensor| sensor.Temperature).find(|&celsius| plausible(celsius)).map(f32::from)
}

/// Whether a drive's reported temperature can be one: not the "unknown"
/// marker (the lowest value), and within what a working drive can be at
/// (no storage device works below −55 °C or above 150 °C; virtual drives
/// report numbers far outside).
fn plausible(celsius: i16) -> bool {
    (-55..=150).contains(&celsius)
}

fn model(handle: HANDLE) -> Option<String> {
    let mut buf = [0u64; 128];
    if !query(handle, StorageDeviceProperty, &mut buf) {
        return None;
    }
    let descriptor = unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    let offset = descriptor.ProductIdOffset as usize;
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
    if offset == 0 || offset >= bytes.len() {
        return None;
    }
    let end = bytes[offset..].iter().position(|&b| b == 0).map_or(bytes.len(), |n| offset + n);
    Some(String::from_utf8_lossy(&bytes[offset..end]).trim().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tells_readings_from_markers() {
        assert!(super::plausible(34));
        // The unknown marker, and what a virtual NVMe drive reports.
        assert!(!super::plausible(i16::MIN));
        assert!(!super::plausible(11759));
    }

    #[test]
    fn reads_this_machines_drives() {
        let start = std::time::Instant::now();
        let drives = super::temperatures();
        println!("{} drives in {:?}", drives.len(), start.elapsed());
        for drive in &drives {
            println!("  {} {} °C", drive.name, drive.celsius);
        }
    }
}
