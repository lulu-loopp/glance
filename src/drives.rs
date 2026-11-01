//! Drive temperatures, as the drives report them to Windows. Every drive is
//! opened for its properties only (no access to its data, so no administrator
//! rights), never to read or write: an NVMe drive whose driver gives none is
//! asked for its health log, a SATA drive for the SMART attributes Windows
//! reads of it for failure prediction, now and then, never while it sleeps.

use crate::reading::DriveTemperature;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, StorageDeviceTemperatureProperty, IOCTL_STORAGE_PREDICT_FAILURE,
    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PREDICT_FAILURE, STORAGE_PROPERTY_ID,
    STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR, STORAGE_TEMPERATURE_INFO,
};
use windows::Win32::System::IO::DeviceIoControl;


/// Every physical drive: its number, and its model name.
pub fn models() -> Vec<(u32, String)> {
    let mut found = Vec::new();
    each_drive(|index, _, handle| found.push((index, model(handle).unwrap_or_else(|| format!("Disk {index}")))));
    found
}

/// Every physical drive that reports a temperature, by its model name. Each drive asked only the way it gave one the
/// first time (see `Way`): one that gave none is not asked again while it stays (a USB drive's bridge asked every
/// second of what it does not know may stall the drive's own reading and writing).
pub fn temperatures() -> Vec<DriveTemperature> {
    static WAYS: std::sync::Mutex<Vec<(String, Way)>> = std::sync::Mutex::new(Vec::new());
    let mut ways = WAYS.lock().unwrap();
    let mut seen = Vec::new();
    let mut found = Vec::new();
    each_drive(|index, path, handle| {
        seen.push(path.to_string());
        if let Some(celsius) = asked(&mut ways, path, &Opened { index, handle }) {
            found.push(DriveTemperature { id: index, name: model(handle).unwrap_or_else(|| format!("Disk {index}")), celsius });
        }
    });
    // (A drive taken out is forgotten: put back, it is learned anew.)
    ways.retain(|(path, _)| seen.contains(path));
    found
}

/// The temperature `drive` (listed as `path`) gives now, asked the way `ways` knows it gives one, learned (and kept
/// there) the first time it is asked.
fn asked(ways: &mut Vec<(String, Way)>, path: &str, drive: &impl Asked) -> Option<f32> {
    match ways.iter().find(|(known, _)| known == path) {
        Some((_, way)) => way.read(drive),
        None => {
            let (way, celsius) = Way::learn(drive);
            ways.push((path.to_string(), way));
            celsius
        }
    }
}

/// What a drive can be asked for its temperature: each way it may give one, and the bus it is on.
trait Asked {
    fn property(&self) -> Option<f32>;
    fn bus(&self) -> Option<i32>;
    fn health_log(&self) -> Option<f32>;
    fn smart(&self) -> Option<f32>;
}

/// A drive by its number, opened for its properties.
struct Opened {
    index: u32,
    handle: HANDLE,
}

impl Asked for Opened {
    fn property(&self) -> Option<f32> {
        temperature(self.handle)
    }
    fn bus(&self) -> Option<i32> {
        bus(self.handle)
    }
    fn health_log(&self) -> Option<f32> {
        health_celsius(self.handle)
    }
    fn smart(&self) -> Option<f32> {
        smart_temperature(self.index, self.handle)
    }
}

/// The way a drive gives its temperature, as found the first time it was asked.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    /// Its property (see `temperature`).
    Property,
    /// Its NVMe health log (see `health_celsius`).
    HealthLog,
    /// Its SMART attributes (see `smart_temperature`): a SATA drive's, asked now and then while it is awake.
    Smart,
    /// None: not asked again.
    None,
}

impl Way {
    /// How `drive` gives its temperature, and what it gives now: its property, else its health log where it is an
    /// NVMe drive, else its SMART attributes where it is a SATA drive (one asleep now, or not due, gives none yet, and
    /// is asked so again), else none (a drive on another bus, a USB drive's, is asked nothing more).
    fn learn(drive: &impl Asked) -> (Way, Option<f32>) {
        if let Some(celsius) = drive.property() {
            return (Way::Property, Some(celsius));
        }
        match drive.bus() {
            Some(BUS_NVME) => {
                let celsius = drive.health_log();
                (if celsius.is_some() { Way::HealthLog } else { Way::None }, celsius)
            }
            Some(BUS_ATA | BUS_SATA) => (Way::Smart, drive.smart()),
            _ => (Way::None, None),
        }
    }

    /// The temperature `drive` gives this way.
    fn read(self, drive: &impl Asked) -> Option<f32> {
        match self {
            Way::Property => drive.property(),
            Way::HealthLog => drive.health_log(),
            Way::Smart => drive.smart(),
            Way::None => None,
        }
    }
}

/// The NVMe bus, as STORAGE_BUS_TYPE numbers it.
const BUS_NVME: i32 = 17;
/// What an NVMe drive is asked for its health log with: the property (StorageDeviceProtocolSpecificProperty), the
/// protocol (ProtocolTypeNvme), what is asked (NVMeDataTypeLogPage), and which page (SMART / health information).
const PROTOCOL_PROPERTY: i32 = 50;
const PROTOCOL_NVME: u32 = 3;
const NVME_LOG_PAGE: u32 = 2;
const NVME_HEALTH_LOG: u32 = 2;
const NVME_LOG_SIZE: usize = 512;

/// STORAGE_PROTOCOL_SPECIFIC_DATA.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProtocolData {
    protocol: u32,
    kind: u32,
    request: u32,
    sub_request: u32,
    offset: u32,
    length: u32,
    fixed_return: u32,
    sub_request2: u32,
    sub_request3: u32,
    sub_request4: u32,
}

/// An NVMe drive's temperature from its health log (its composite temperature, in kelvin, in bytes 1 and 2), for a
/// drive whose driver gives Windows none as a property (some vendors' drivers, and RAID ones, do not). Asked as
/// Windows documents for NVMe drives, through the handle opened for properties. (Opened to read and write besides,
/// for drivers that might want it, it looked to virus scanners like a program writing to the drive's boot record.)
fn health_celsius(handle: HANDLE) -> Option<f32> {
    // The query (its property and kind), what is asked of the protocol, and room for the page after it.
    #[repr(C)]
    struct Asked {
        property: i32,
        query: i32,
        data: ProtocolData,
        page: [u8; NVME_LOG_SIZE],
    }
    let mut asked = Asked {
        property: PROTOCOL_PROPERTY,
        query: PropertyStandardQuery.0,
        data: ProtocolData { protocol: PROTOCOL_NVME, kind: NVME_LOG_PAGE, request: NVME_HEALTH_LOG, offset: size_of::<ProtocolData>() as u32, length: NVME_LOG_SIZE as u32, ..Default::default() },
        page: [0; NVME_LOG_SIZE],
    };
    let mut returned = 0u32;
    let size = size_of::<Asked>() as u32;
    let pointer = &mut asked as *mut Asked as *mut core::ffi::c_void;
    unsafe { DeviceIoControl(handle, IOCTL_STORAGE_QUERY_PROPERTY, Some(pointer), size, Some(pointer), size, Some(&mut returned), None) }.ok()?;
    // Answered as STORAGE_PROTOCOL_DATA_DESCRIPTOR: its version and size, then the protocol's data as given back,
    // whose offset says where the page is from there.
    let answered: &[u8] = unsafe { std::slice::from_raw_parts(pointer as *const u8, size as usize) };
    let data: ProtocolData = unsafe { std::ptr::read_unaligned(answered.as_ptr().add(8) as *const ProtocolData) };
    let at = 8 + data.offset as usize;
    let kelvin = u16::from_le_bytes([*answered.get(at + 1)?, *answered.get(at + 2)?]);
    let celsius = kelvin as i32 - 273;
    (kelvin > 0 && plausible(celsius as i16)).then_some(celsius as f32)
}

/// Each physical drive, for a report: its number, model and bus (as STORAGE_BUS_TYPE numbers it), and which way
/// its temperature is read, if any is.
pub fn describe() -> Vec<String> {
    let mut found = Vec::new();
    each_drive(|index, _, handle| {
        let read = match Way::learn(&Opened { index, handle }) {
            (Way::Property, _) => "its property",
            (Way::HealthLog, _) => "its NVMe health log",
            (Way::Smart, Some(_)) => "its SMART attributes",
            (Way::Smart | Way::None, _) => "none",
        };
        found.push(format!("Drive {index}: {}, bus {}, temperature: {read}", model(handle).unwrap_or_default(), bus(handle).map_or("unknown".to_string(), |bus| bus.to_string())));
    });
    found
}

/// How often a SATA drive is asked for its SMART attributes.
const SMART_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
/// The SATA and (parallel) ATA buses, as STORAGE_BUS_TYPE numbers them.
const BUS_ATA: i32 = 3;
const BUS_SATA: i32 = 11;

/// A SATA drive's temperature from its SMART attributes, read at most every `SMART_EVERY` (what was read last
/// stands between), and only while Windows has the drive powered: one it has put to sleep is left there, unread.
/// The attributes as Windows' disk driver reads them for failure prediction, through the handle opened for
/// properties. (Asked of the drive itself, opened to read and write as its commands need, it looked to virus
/// scanners like a program writing to the drive's boot record.)
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
        let mut on = windows::core::BOOL(0);
        if !unsafe { windows::Win32::System::Power::GetDevicePowerState(properties, &mut on) }.as_bool() || !on.as_bool() {
            return None;
        }
        let mut predicted = STORAGE_PREDICT_FAILURE::default();
        let mut returned = 0u32;
        let size = size_of::<STORAGE_PREDICT_FAILURE>() as u32;
        unsafe { DeviceIoControl(properties, IOCTL_STORAGE_PREDICT_FAILURE, None, 0, Some(&mut predicted as *mut _ as *mut _), size, Some(&mut returned), None) }.ok()?;
        smart_celsius(&predicted.VendorSpecific)
    })();
    last.retain(|(drive, ..)| *drive != index);
    last.push((index, std::time::Instant::now(), read));
    read
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

/// The buses a drive is plugged into from outside the PC on, as STORAGE_BUS_TYPE numbers them: USB, FireWire, an SD
/// card's and an MMC card's.
const BUSES_OUTSIDE: [i32; 4] = [7, 4, 12, 13];

/// Whether the drive at `letter` is one plugged into the PC from outside (see `BUSES_OUTSIDE`), whatever Windows calls
/// it: a USB hard drive is a fixed drive to it, as the PC's own are. Asked of the volume opened for its properties only
/// (no access to its data), which Windows answers itself, asking the drive nothing; one that cannot say is taken for
/// the PC's own.
pub fn outside(letter: char) -> bool {
    let path = windows::core::HSTRING::from(format!(r"\\.\{letter}:"));
    let Ok(handle) = (unsafe { CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }) else { return false };
    let on = bus(handle);
    let _ = unsafe { CloseHandle(handle) };
    on.is_some_and(|bus| BUSES_OUTSIDE.contains(&bus))
}

/// The bus a drive is on, as STORAGE_BUS_TYPE numbers it.
fn bus(handle: HANDLE) -> Option<i32> {
    let mut buf = [0u64; 128];
    if !query(handle, StorageDeviceProperty, &mut buf) {
        return None;
    }
    Some(unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) }.BusType.0)
}

/// Calls `visit` with each physical drive (by its number, as `\.\PhysicalDriveN` names it, and the path Windows lists
/// it by, which stays its own while it is there), opened for its properties only. Each drive Windows lists, by its own number, whatever numbers are free between them (tried one
/// number after another, those after a gap of four were missed: drives taken out, a card reader's empty slots).
fn each_drive(mut visit: impl FnMut(u32, &str, HANDLE)) {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
        SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    };
    use windows::Win32::System::Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
    let mut drives: Vec<(u32, String, HANDLE)> = Vec::new();
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(Some(&GUID_DEVINTERFACE_DISK), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) else { return };
        for member in 0.. {
            let mut interface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVINTERFACE_DISK, member, &mut interface).is_err() {
                break;
            }
            // Its path, in a buffer as large as Windows says it needs (aligned for the structure it starts with).
            let mut size = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &interface, None, 0, Some(&mut size), None);
            let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &interface, Some(detail), size, None, None).is_err() {
                continue;
            }
            // Access 0: properties only, never the data.
            let path = PCWSTR((*detail).DevicePath.as_ptr());
            let opened = CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None);
            let Ok(handle) = opened else { continue };
            let mut number = STORAGE_DEVICE_NUMBER::default();
            let mut returned = 0u32;
            let numbered = DeviceIoControl(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, 0, Some(&mut number as *mut _ as *mut _), size_of::<STORAGE_DEVICE_NUMBER>() as u32, Some(&mut returned), None);
            if numbered.is_ok() {
                drives.push((number.DeviceNumber, path.to_string().unwrap_or_default(), handle));
            } else {
                let _ = CloseHandle(handle);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    drives.sort_by_key(|(number, ..)| *number);
    for (number, path, handle) in drives {
        visit(number, &path, handle);
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
    /// This PC's drives, by their numbers, as Windows lists them:
    /// `cargo test --lib -- --ignored lists_this_pcs_drives --nocapture`.
    #[test]
    #[ignore = "reads this PC's drives"]
    fn lists_this_pcs_drives() {
        let drives = super::models();
        println!("{drives:?}");
        assert!(!drives.is_empty());
        assert!(drives.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

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

    /// Which of this PC's drives are plugged in from outside:
    /// `cargo test --lib -- --ignored tells_this_pcs_drives_from_outside --nocapture`.
    #[test]
    #[ignore = "reads this PC's drives"]
    fn tells_this_pcs_drives_from_outside() {
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for letter in (0..26u8).filter(|bit| mask & (1 << bit) != 0).map(|bit| (b'A' + bit) as char) {
            println!("{letter}: outside {}", super::outside(letter));
        }
    }

    /// A drive that answers as it is made to, counting what it is asked.
    struct Made {
        property: Option<f32>,
        bus: Option<i32>,
        health_log: Option<f32>,
        smart: std::cell::Cell<Option<f32>>,
        asked: std::cell::Cell<u32>,
    }

    impl Made {
        fn new(property: Option<f32>, bus: i32, health_log: Option<f32>, smart: Option<f32>) -> Made {
            Made { property, bus: Some(bus), health_log, smart: smart.into(), asked: 0.into() }
        }
        fn ask<T>(&self, answer: T) -> T {
            self.asked.set(self.asked.get() + 1);
            answer
        }
    }

    impl super::Asked for Made {
        fn property(&self) -> Option<f32> {
            self.ask(self.property)
        }
        fn bus(&self) -> Option<i32> {
            self.ask(self.bus)
        }
        fn health_log(&self) -> Option<f32> {
            self.ask(self.health_log)
        }
        fn smart(&self) -> Option<f32> {
            self.ask(self.smart.get())
        }
    }

    /// A drive that gives no temperature (a USB drive's bridge, asked every second, stalled the drive's copying) is
    /// asked the first time only, while one that gives one is asked it the way it gave it, and nothing else.
    #[test]
    fn a_drive_that_gives_no_temperature_is_not_asked_again() {
        let mut ways = Vec::new();
        let usb = Made::new(None, 7, None, None);
        for _ in 0..5 {
            assert_eq!(super::asked(&mut ways, "usb", &usb), None);
        }
        assert_eq!(usb.asked.get(), 2, "its property and its bus, once");

        let nvme = Made::new(Some(40.0), super::BUS_NVME, Some(41.0), None);
        for _ in 0..3 {
            assert_eq!(super::asked(&mut ways, "nvme", &nvme), Some(40.0));
        }
        assert_eq!(nvme.asked.get(), 3, "its property alone, each time");

        let log = Made::new(None, super::BUS_NVME, Some(38.0), None);
        for _ in 0..3 {
            assert_eq!(super::asked(&mut ways, "log", &log), Some(38.0));
        }
        assert_eq!(log.asked.get(), 3 + 2, "its health log alone after the first time");

        // A SATA drive asleep the first time is asked again, as it wakes.
        let sata = Made::new(None, super::BUS_SATA, None, None);
        assert_eq!(super::asked(&mut ways, "sata", &sata), None);
        sata.smart.set(Some(35.0));
        assert_eq!(super::asked(&mut ways, "sata", &sata), Some(35.0));
        assert_eq!(sata.asked.get(), 3 + 1);
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

#[cfg(test)]
mod nvme_tests {
    /// Each NVMe drive's temperature, as its property says and as its health log does:
    /// `cargo test --lib -- --ignored reads_nvme_logs --nocapture`.
    #[test]
    #[ignore = "reads this PC's drives"]
    fn reads_nvme_logs() {
        super::each_drive(|index, _, handle| {
            if super::bus(handle) == Some(super::BUS_NVME) {
                println!("drive {index} {:?}: property {:?}, health log {:?}", super::model(handle), super::temperature(handle), super::health_celsius(handle));
            }
        });
    }
}

