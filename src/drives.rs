//! Drive temperatures, as the drives report them to Windows. Asking for a
//! drive's properties needs no access to its data, so no administrator rights.
//! A SATA drive that gives Windows none is asked for its SMART attributes
//! itself (which does need them), now and then, and never out of standby.

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

/// Every physical drive: its number, and its model name.
pub fn models() -> Vec<(u32, String)> {
    let mut found = Vec::new();
    each_drive(|index, handle| found.push((index, model(handle).unwrap_or_else(|| format!("Disk {index}")))));
    found
}

/// Every physical drive that reports a temperature, by its model name.
pub fn temperatures() -> Vec<DriveTemperature> {
    let mut found = Vec::new();
    each_drive(|index, handle| {
        if let Some(celsius) = temperature(handle).or_else(|| smart_temperature(index, handle)) {
            found.push(DriveTemperature { id: index, name: model(handle).unwrap_or_else(|| format!("Disk {index}")), celsius });
        }
    });
    found
}

/// How often a SATA drive is asked for its SMART attributes.
const SMART_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
/// IOCTL_ATA_PASS_THROUGH, and its flags: the drive must be ready, and the
/// command reads data in.
const ATA_PASS_THROUGH: u32 = 0x0004_D02C;
const ATA_DRDY_REQUIRED: u16 = 0x01;
const ATA_DATA_IN: u16 = 0x02;
/// The SATA and (parallel) ATA buses, as STORAGE_BUS_TYPE numbers them.
const BUS_ATA: i32 = 3;
const BUS_SATA: i32 = 11;

/// ATA_PASS_THROUGH_EX, the data read following it in one buffer.
#[repr(C)]
#[derive(Default)]
struct AtaCommand {
    length: u16,
    flags: u16,
    path: u8,
    target: u8,
    lun: u8,
    reserved: u8,
    data_length: u32,
    timeout_s: u32,
    reserved2: u32,
    data_offset: usize,
    previous: [u8; 8],
    /// Features, sector count, LBA low, mid and high, device, command, reserved.
    task: [u8; 8],
}

/// A SATA drive's temperature from its SMART attributes, read at most every
/// `SMART_EVERY` (what was read last stands between), and only while the
/// drive is spinning: a drive in standby is left there, unread.
fn smart_temperature(index: u32, properties: HANDLE) -> Option<f32> {
    static LAST: std::sync::Mutex<Vec<(u32, std::time::Instant, Option<f32>)>> = std::sync::Mutex::new(Vec::new());
    let mut last = LAST.lock().unwrap();
    if let Some((_, at, celsius)) = last.iter().find(|(drive, ..)| *drive == index) {
        if at.elapsed() < SMART_EVERY {
            return *celsius;
        }
    }
    let read = (|| {
        if !matches!(bus(properties)?, BUS_ATA | BUS_SATA) {
            return None;
        }
        // Commands to the drive itself need it opened for reading and
        // writing (they touch none of its data).
        let path: Vec<u16> = format!("\\\\.\\PhysicalDrive{index}").encode_utf16().chain([0]).collect();
        let drive = unsafe {
            CreateFileW(PCWSTR(path.as_ptr()), 0xC000_0000, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
        }
        .ok()?;
        // CHECK POWER MODE: a sector count of zero is standby.
        let awake = ata(drive, [0, 0, 0, 0, 0, 0xA0, 0xE5, 0], None).is_some_and(|task| task[1] != 0);
        let mut data = [0u8; 512];
        // SMART READ DATA.
        let attributes = awake.then(|| ata(drive, [0xD0, 1, 0, 0x4F, 0xC2, 0xA0, 0xB0, 0], Some(&mut data))).flatten();
        let _ = unsafe { CloseHandle(drive) };
        attributes.and_then(|_| smart_celsius(&data))
    })();
    last.retain(|(drive, ..)| *drive != index);
    last.push((index, std::time::Instant::now(), read));
    read
}

/// Sends the drive an ATA command (its task file), reading 512 bytes into
/// `data` if given: the task file it answers with.
fn ata(drive: HANDLE, task: [u8; 8], data: Option<&mut [u8; 512]>) -> Option<[u8; 8]> {
    #[repr(C)]
    struct Buffer {
        command: AtaCommand,
        data: [u8; 512],
    }
    let reads = data.is_some();
    let mut buffer = Buffer {
        command: AtaCommand {
            length: size_of::<AtaCommand>() as u16,
            flags: ATA_DRDY_REQUIRED | if reads { ATA_DATA_IN } else { 0 },
            data_length: if reads { 512 } else { 0 },
            timeout_s: 3,
            data_offset: std::mem::offset_of!(Buffer, data),
            task,
            ..Default::default()
        },
        data: [0; 512],
    };
    let mut returned = 0u32;
    let size = size_of::<Buffer>() as u32;
    let pointer = &mut buffer as *mut Buffer as *mut core::ffi::c_void;
    unsafe { DeviceIoControl(drive, ATA_PASS_THROUGH, Some(pointer), size, Some(pointer), size, Some(&mut returned), None) }.ok()?;
    // An error bit in the status it answers with: not done.
    if buffer.command.task[6] & 1 != 0 {
        return None;
    }
    if let Some(data) = data {
        *data = buffer.data;
    }
    Some(buffer.command.task)
}

/// The temperature among a drive's SMART attributes: attribute 194
/// (temperature), else 190 (airflow temperature, which some drives give in
/// its place), the first byte of its raw value.
fn smart_celsius(data: &[u8; 512]) -> Option<f32> {
    // Thirty attributes of twelve bytes from the third byte: its number,
    // flags (2), value, worst, raw (6), reserved.
    let attribute = |id: u8| data[2..362].chunks_exact(12).find(|entry| entry[0] == id).map(|entry| entry[5] as i16);
    attribute(194).or_else(|| attribute(190)).filter(|celsius| plausible(*celsius) && *celsius > 0).map(f32::from)
}

/// The bus a drive is on, as STORAGE_BUS_TYPE numbers it.
fn bus(handle: HANDLE) -> Option<i32> {
    let mut buf = [0u64; 128];
    if !query(handle, StorageDeviceProperty, &mut buf) {
        return None;
    }
    Some(unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) }.BusType.0)
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
    fn reads_a_temperature_from_smart_attributes() {
        fn put(data: &mut [u8; 512], slot: usize, id: u8, raw: u8) {
            data[2 + slot * 12] = id;
            data[2 + slot * 12 + 5] = raw;
        }
        let mut data = [0u8; 512];
        // As a Samsung 750 EVO gives it: no 194, the airflow temperature.
        put(&mut data, 0, 9, 200);
        put(&mut data, 1, 190, 30);
        assert_eq!(super::smart_celsius(&data), Some(30.0));
        // 194 where there is one.
        put(&mut data, 2, 194, 41);
        assert_eq!(super::smart_celsius(&data), Some(41.0));
        assert_eq!(super::smart_celsius(&[0u8; 512]), None);
    }

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
