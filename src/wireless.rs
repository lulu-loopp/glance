//! The batteries of the wireless devices joined to the PC: Bluetooth headphones, keyboards, mice and speakers (the
//! level Windows itself shows in its Bluetooth settings), the devices of Logitech's receivers (Unifying, Bolt,
//! Lightspeed: asked over HID++, as Logitech's own software asks), and Xbox controllers (as Windows' game controller
//! API tells). Only devices joined now and whose level is known are listed.
//!
//! All of it is read on a thread of its own: at the start, then every half minute (Logitech's devices every minute,
//! as each is asked over the air), and a moment after a device comes or goes. Nothing is set on any device: every
//! question asked of one only reads. No rights are needed.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver as Told, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{GUID, PCWSTR};
use windows::Devices::Bluetooth::BluetoothLEDevice;
use windows::Foundation::EventHandler;
use windows::Gaming::Input::{Gamepad, RawGameController};
use windows::System::Power::BatteryStatus;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_IDW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_PropertyW, CM_Get_Parent, CM_Locate_DevNodeW, CM_Register_Notification, CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT,
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_LOCATE_DEVNODE_NORMAL, CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
    CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CONFIGRET, CR_BUFFER_SMALL, CR_SUCCESS,
    HCMNOTIFICATION,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetCaps, GUID_DEVINTERFACE_HID, HIDP_CAPS, HIDP_STATUS_SUCCESS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPKEY_Device_FriendlyName, DEVPKEY_Device_InstanceId, DEVPROPTYPE, DEVPROP_TYPE_BOOLEAN, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID,
    DEVPROP_TYPE_STRING, DEVPROP_TYPE_UINT32,
};
use windows::Win32::Foundation::{CloseHandle, DEVPROPKEY, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};

/// A wireless device whose battery level is known, as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// What the device is known by from one reading to the next: "bt:" and its Bluetooth address, "hidpp:" and its
    /// receiver's device instance and the receiver's slot it is paired in, or "xbox:" and Windows' id of the
    /// controller.
    pub key: String,
    /// The device's name as the user knows it: the name Windows shows for a Bluetooth device (renamed by the user, if
    /// they renamed it), the name a Logitech device tells, or Windows' name for a controller.
    pub name: String,
    pub kind: Kind,
    pub via: Via,
    /// Percent, 0 to 100.
    pub level: u8,
    /// Whether it is being charged now (a Bluetooth device's level does not say: false for them).
    pub charging: bool,
    /// The Plug and Play container of a Bluetooth device (`DEVPKEY_Device_ContainerId`), which its sound outputs
    /// share: none for the others.
    pub container: Option<u128>,
}

/// What a device is, as far as it tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Headphones,
    Earbuds,
    Headset,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Watch,
    Other,
}

/// How a device is joined to the PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Bluetooth,
    /// Through one of Logitech's receivers.
    Logitech,
    /// As an Xbox controller (through Xbox's own wireless adapter, or Bluetooth).
    Xbox,
}

/// The devices as last read, ordered as `now` gives them.
static DEVICES: Mutex<Vec<Device>> = Mutex::new(Vec::new());

/// How often everything is read again.
const EVERY: Duration = Duration::from_secs(30);
/// How often Logitech's devices are asked again (each answers over the air).
const LOGITECH_EVERY: Duration = Duration::from_secs(60);
/// How long Windows' telling of devices coming and going must be quiet before they are read again (a device comes
/// with several interfaces at once), and the longest they wait for that.
const SETTLED: Duration = Duration::from_secs(2);
const SETTLING_AT_MOST: Duration = Duration::from_secs(10);

/// Starts the reader's thread, once (later calls do nothing).
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new().name("glance: wireless batteries".into()).spawn(run).expect("a thread started");
    });
}

/// The devices joined now whose level is known, as last read: by kind, then by name.
pub fn now() -> Vec<Device> {
    DEVICES.lock().unwrap().clone()
}

/// The reader: everything read, then read again every `EVERY`, or once Windows has told of a device coming or going
/// and gone quiet again.
fn run() {
    crate::actions::com();
    let (told, telling) = channel();
    hear_devices(told.clone());
    // (Kept for as long as the thread runs: Windows lists controllers only to a program that hears of them.)
    let _controllers = xbox::hear(told);
    let mut bluetooth = bluetooth::Reader::default();
    let mut logitech = logitech::Reader::default();
    let mut logitech_read: Option<(Instant, Vec<Device>)> = None;
    let mut asked = false;
    loop {
        if asked || logitech_read.as_ref().is_none_or(|(at, _)| at.elapsed() >= LOGITECH_EVERY) {
            logitech_read = Some((Instant::now(), logitech.read()));
        }
        let mut devices = bluetooth.read();
        devices.extend(logitech_read.as_ref().map(|(_, devices)| devices.clone()).unwrap_or_default());
        devices.extend(xbox::read());
        *DEVICES.lock().unwrap() = ordered(devices);
        asked = match settled(&telling) {
            Some(asked) => asked,
            None => return,
        };
    }
}

/// Waits `EVERY`, or for Windows' telling of devices to begin and then go quiet: whether it was told (none where no
/// one can tell any more).
fn settled(telling: &Told<()>) -> Option<bool> {
    match telling.recv_timeout(EVERY) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => return Some(false),
        Err(RecvTimeoutError::Disconnected) => return None,
    }
    let since = Instant::now();
    while since.elapsed() < SETTLING_AT_MOST && telling.recv_timeout(SETTLED).is_ok() {}
    Some(true)
}

/// The devices in the order they are shown: by kind, then by name, then by key (two of one name in a stable order).
fn ordered(mut devices: Vec<Device>) -> Vec<Device> {
    devices.sort_by(|a, b| (a.kind as u8, a.name.to_lowercase(), &a.key).cmp(&(b.kind as u8, b.name.to_lowercase(), &b.key)));
    devices.dedup_by(|a, b| a.key == b.key);
    devices
}

/// Has `told` sent to as any device interface comes or goes (a receiver plugged in, a Bluetooth device joined, whose
/// sound or input then comes), for as long as the program runs.
fn hear_devices(told: Sender<()>) {
    let filter = CM_NOTIFY_FILTER {
        cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
        Flags: CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        ..Default::default()
    };
    // (Never let go of: heard for as long as the program runs.)
    let context = Box::into_raw(Box::new(told)) as *const core::ffi::c_void;
    let mut heard = HCMNOTIFICATION::default();
    unsafe { CM_Register_Notification(&filter, Some(context), Some(interface_changed), &mut heard) };
}

unsafe extern "system" fn interface_changed(_: HCMNOTIFICATION, context: *const core::ffi::c_void, action: CM_NOTIFY_ACTION, _: *const CM_NOTIFY_EVENT_DATA, _: u32) -> u32 {
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
        let told = unsafe { &*(context as *const Sender<()>) };
        let _ = told.send(());
    }
    0
}

/// Bluetooth devices, read as Windows' device manager keeps them: the battery level the Bluetooth stack keeps of each
/// (that Windows' Bluetooth settings show), on one of the device's nodes, and whether the device is joined now, on
/// the device's own node (`BTHENUM\DEV_…` for one of classic Bluetooth, `BTHLE\DEV_…` for one of Low Energy). The two
/// are matched by their container: every node of one physical device shares it.
mod bluetooth {
    use super::*;

    /// The battery level the Bluetooth stack keeps of a device (percent, a byte), as Windows' settings show it.
    const BATTERY_LEVEL: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
    /// Whether a device is joined now (a boolean): true for a device plugged in or a Bluetooth device connected, false
    /// for a paired one that is not, whose nodes stay present (and keep the level last told).
    const CONNECTED: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };
    /// A Bluetooth device's address (twelve hex digits), and its class of device (a classic one's; under its older key
    /// before Windows 10's 2004 release).
    const ADDRESS: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 1 };
    const CLASS_OF_DEVICE: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 10 };
    const CLASS_OF_DEVICE_BEFORE: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 4 };

    /// The enumerators of Bluetooth's device nodes: the devices of classic Bluetooth and their services, those of
    /// Low Energy and their services, and the hands-free audio of classic devices.
    const ENUMERATORS: [&str; 4] = ["BTHENUM", "BTHLE", "BTHLEDEVICE", "BTHHFENUM"];

    /// A Bluetooth device's own node, as read.
    #[derive(Clone, Debug, PartialEq)]
    pub(super) struct Record {
        pub address: u64,
        pub name: String,
        pub container: u128,
        pub connected: bool,
        /// Whether it is a Low Energy device (else classic), and a classic one's class of device.
        pub low_energy: bool,
        pub class: Option<u32>,
    }

    /// Reads Bluetooth devices, keeping what Low Energy devices said they are (asked of Windows once each).
    #[derive(Default)]
    pub(super) struct Reader {
        appearances: HashMap<u64, u16>,
    }

    impl Reader {
        pub fn read(&mut self) -> Vec<Device> {
            let mut records = Vec::new();
            let mut levels = Vec::new();
            for enumerator in ENUMERATORS {
                for id in device_ids(enumerator) {
                    let Some(node) = located(&id) else { continue };
                    let Some(container) = property::<GUID>(node, &DEVPKEY_Device_ContainerId, DEVPROP_TYPE_GUID) else { continue };
                    let upper = id.to_ascii_uppercase();
                    let low_energy = upper.starts_with("BTHLE\\DEV_");
                    if upper.starts_with("BTHENUM\\DEV_") || low_energy {
                        let Some(address) = text(node, &ADDRESS).and_then(|address| u64::from_str_radix(&address, 16).ok()) else { continue };
                        records.push(Record {
                            address,
                            name: text(node, &DEVPKEY_Device_FriendlyName).unwrap_or_default(),
                            container: container.to_u128(),
                            connected: property::<u8>(node, &CONNECTED, DEVPROP_TYPE_BOOLEAN).is_some_and(|connected| connected != 0),
                            low_energy,
                            class: property::<u32>(node, &CLASS_OF_DEVICE, DEVPROP_TYPE_UINT32).or_else(|| property::<u32>(node, &CLASS_OF_DEVICE_BEFORE, DEVPROP_TYPE_UINT32)),
                        });
                    }
                    // (Kept on whichever node of the device the stack reads it through: a classic one's hands-free
                    // service's, for one.)
                    if let Some(level) = property::<u8>(node, &BATTERY_LEVEL, DEVPROP_TYPE_BYTE) {
                        levels.push((container.to_u128(), level));
                    }
                }
            }
            joined(&records, &levels)
                .into_iter()
                .map(|(record, level)| {
                    let kind = match (record.low_energy, record.class) {
                        (true, _) => self.appearance(record.address).map_or(Kind::Other, appearance_kind),
                        (false, Some(class)) => class_kind(class),
                        (false, None) => Kind::Other,
                    };
                    Device { key: format!("bt:{:012x}", record.address), name: record.name, kind, via: Via::Bluetooth, level, charging: false, container: Some(record.container) }
                })
                .collect()
        }

        /// What a Low Energy device says it is (its GAP appearance), as Windows keeps it from its pairing: asked once.
        fn appearance(&mut self, address: u64) -> Option<u16> {
            if let Some(&appearance) = self.appearances.get(&address) {
                return Some(appearance);
            }
            let device = BluetoothLEDevice::FromBluetoothAddressAsync(address).ok()?.join().ok()?;
            let appearance = device.Appearance().ok()?.RawValue().ok()?;
            self.appearances.insert(address, appearance);
            Some(appearance)
        }
    }

    /// Each device joined now that has a level, with its level: a device's nodes found by their container (where a
    /// device has two, of classic and of Low Energy, the one joined; the one of the lower address of two joined), and
    /// its level the one kept on another node of that container (the highest, where several keep one).
    pub(super) fn joined(records: &[Record], levels: &[(u128, u8)]) -> Vec<(Record, u8)> {
        let mut by_container: HashMap<u128, &Record> = HashMap::new();
        for record in records.iter().filter(|record| record.connected) {
            by_container
                .entry(record.container)
                .and_modify(|kept| {
                    if record.address < kept.address {
                        *kept = record
                    }
                })
                .or_insert(record);
        }
        let mut level_of: HashMap<u128, u8> = HashMap::new();
        for &(container, level) in levels.iter().filter(|(_, level)| *level <= 100) {
            level_of.entry(container).and_modify(|kept| *kept = (*kept).max(level)).or_insert(level);
        }
        let mut joined: Vec<(Record, u8)> = by_container.into_iter().filter_map(|(container, record)| Some((record.clone(), *level_of.get(&container)?))).collect();
        joined.sort_by_key(|(record, _)| record.address);
        joined
    }

    /// What a classic device is, by its class of device (Bluetooth's assigned numbers: the major class in bits 8 to
    /// 12, the minor in bits 2 to 7).
    pub(super) fn class_kind(class: u32) -> Kind {
        let major = (class >> 8) & 0x1F;
        let minor = (class >> 2) & 0x3F;
        match major {
            // Phone.
            0x02 => Kind::Phone,
            // Audio/video: a wearable headset, a hands-free device; headphones; a loudspeaker, portable audio, hi-fi.
            0x04 => match minor {
                0x01 | 0x02 => Kind::Headset,
                0x06 => Kind::Headphones,
                0x05 | 0x07 | 0x0A => Kind::Speaker,
                _ => Kind::Other,
            },
            // Peripheral: a joystick or gamepad (its low four bits), else a keyboard, a pointing device, or both (its
            // top two).
            0x05 => match (minor >> 4, minor & 0x0F) {
                (_, 0x01 | 0x02) => Kind::Gamepad,
                (0x01 | 0x03, _) => Kind::Keyboard,
                (0x02, _) => Kind::Mouse,
                _ => Kind::Other,
            },
            // Wearable: a wristwatch.
            0x07 if minor == 0x01 => Kind::Watch,
            _ => Kind::Other,
        }
    }

    /// What a Low Energy device is, by its appearance (Bluetooth's assigned numbers: a category in the top ten bits,
    /// a subcategory in the low six).
    pub(super) fn appearance_kind(appearance: u16) -> Kind {
        match (appearance >> 6, appearance & 0x3F) {
            (0x001, _) => Kind::Phone,
            (0x003, _) => Kind::Watch,
            // Human interface device: a keyboard; a mouse or touchpad; a joystick or gamepad.
            (0x00F, 0x01) => Kind::Keyboard,
            (0x00F, 0x02 | 0x09) => Kind::Mouse,
            (0x00F, 0x03 | 0x04) => Kind::Gamepad,
            // Audio sink: a speaker, a soundbar.
            (0x021, _) => Kind::Speaker,
            // Wearable audio device: an earbud, a headset, headphones, a neck band (earbuds on a cord).
            (0x025, 0x01 | 0x04) => Kind::Earbuds,
            (0x025, 0x02) => Kind::Headset,
            (0x025, _) => Kind::Headphones,
            _ => Kind::Other,
        }
    }

    /// The instance identifiers of the devices present under the enumerator `enumerator`.
    fn device_ids(enumerator: &str) -> Vec<String> {
        let filter: Vec<u16> = enumerator.encode_utf16().chain([0]).collect();
        let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
        let size = || {
            let mut length = 0u32;
            (unsafe { CM_Get_Device_ID_List_SizeW(&mut length, PCWSTR(filter.as_ptr()), flags) } == CR_SUCCESS).then_some(length)
        };
        let list = |into: &mut [u16]| unsafe { CM_Get_Device_ID_ListW(PCWSTR(filter.as_ptr()), into, flags) };
        multi_string(listed(size, list))
    }
}

/// The device node whose instance identifier is `id`, where it is present.
fn located(id: &str) -> Option<u32> {
    let id: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let mut node = 0u32;
    (unsafe { CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) } == CR_SUCCESS).then_some(node)
}

/// A device node's property of a fixed size, where it has it and it is of the type `kind`.
fn property<T: Copy + Default>(node: u32, key: &DEVPROPKEY, kind: DEVPROPTYPE) -> Option<T> {
    let mut value = T::default();
    let (mut found, mut size) = (DEVPROPTYPE::default(), size_of::<T>() as u32);
    let read = unsafe { CM_Get_DevNode_PropertyW(node, key, &mut found, Some((&mut value as *mut T).cast()), &mut size, 0) };
    (read == CR_SUCCESS && found == kind && size as usize == size_of::<T>()).then_some(value)
}

/// A device node's text property, where it has it.
fn text(node: u32, key: &DEVPROPKEY) -> Option<String> {
    let mut buffer = vec![0u16; 128];
    loop {
        let (mut found, mut size) = (DEVPROPTYPE::default(), (buffer.len() * 2) as u32);
        match unsafe { CM_Get_DevNode_PropertyW(node, key, &mut found, Some(buffer.as_mut_ptr().cast()), &mut size, 0) } {
            CR_SUCCESS if found == DEVPROP_TYPE_STRING => return Some(until_zero(&buffer)),
            CR_BUFFER_SMALL => buffer = vec![0u16; (size as usize).div_ceil(2)],
            _ => return None,
        }
    }
}

/// The text before the first zero.
fn until_zero(text: &[u16]) -> String {
    String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())])
}

/// A list the device manager keeps, as `list` writes it into room of the length `size` says it needs, asked again for
/// as long as it grew between the two; empty where either fails.
fn listed(mut size: impl FnMut() -> Option<u32>, mut list: impl FnMut(&mut [u16]) -> CONFIGRET) -> Vec<u16> {
    loop {
        let Some(length) = size() else { return Vec::new() };
        let mut room = vec![0u16; length as usize];
        match list(&mut room) {
            CR_SUCCESS => return room,
            CR_BUFFER_SMALL => continue,
            _ => return Vec::new(),
        }
    }
}

/// The texts of a list of them each ended by a zero (and the list by another).
fn multi_string(list: Vec<u16>) -> Vec<String> {
    list.split(|&c| c == 0).filter(|text| !text.is_empty()).map(String::from_utf16_lossy).collect()
}

/// The devices of Logitech's receivers, asked over HID++ (Logitech's protocol, as documented by Logitech and by
/// Solaar and libratbag, which speak it on Linux) through the receiver's vendor-defined HID collections, opened
/// shared, so that Logitech's own software (Options+, G Hub) goes on speaking to it too: every answer read is matched
/// to the question by device, feature and the software id each question carries, and any other is passed over.
///
/// A receiver is known by its answering a receiver's register read (HID++ 1.0) at its own index (0xFF). Each slot
/// it pairs devices in (1 to 6) is then pinged: a device that answers is joined, and HID++ 2.0; one the receiver says
/// is not there, or not joined, is not listed. Of each device, its name and type (feature 0x0005) and where its
/// battery is read (feature 0x1004, else 0x1000, else 0x1001) are asked once, and kept while it stays joined; its
/// battery is asked at each reading. Only reading functions are ever called.
mod logitech {
    use super::*;

    /// How long a question waits for its answer: a device asleep takes a moment to wake to it.
    const WAITED: Duration = Duration::from_millis(1500);
    /// The HID++ reports: short (7 bytes), long (20) and very long (64), by their report ids.
    const SHORT: u8 = 0x10;
    const LONG: u8 = 0x11;
    const VERY_LONG: u8 = 0x12;
    /// The receiver's own index, and the slots it pairs devices in.
    const RECEIVER: u8 = 0xFF;
    const SLOTS: std::ops::RangeInclusive<u8> = 1..=6;
    /// HID++ 1.0: a register read (short), and the error report its errors come in (whose code says what failed).
    const GET_REGISTER: u8 = 0x81;
    const ERROR_10: u8 = 0x8F;
    /// HID++ 2.0's error report's feature index.
    const ERROR_20: u8 = 0xFF;
    /// The receiver's connection register (its devices' state): read only to know it is a receiver.
    const CONNECTION_STATE: u8 = 0x02;
    /// HID++ 2.0's root feature (always at index 0): its functions finding a feature's index, and pinging.
    const ROOT: u8 = 0x00;
    const GET_FEATURE: u8 = 0;
    const PING: u8 = 1;
    /// The features read: a device's name and type; its battery as unified, as a level, or as a voltage.
    const DEVICE_NAME: u16 = 0x0005;
    const UNIFIED_BATTERY: u16 = 0x1004;
    const BATTERY_STATUS: u16 = 0x1000;
    const BATTERY_VOLTAGE: u16 = 0x1001;

    /// How a question was answered: with its parameters, with HID++ 1.0's error or 2.0's (their codes), or not at all.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum Answer {
        Reply(Vec<u8>),
        Refused(u8),
        Failed(u8),
        Silent,
    }

    /// Where a device's battery is read: which feature, at which index.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Battery {
        Unified(u8),
        Status(u8),
        Voltage(u8),
    }

    /// What is kept of a device while it stays joined: its name, what it is, where its battery is read (none where it
    /// has no battery feature).
    #[derive(Clone, Debug)]
    struct Known {
        name: String,
        kind: Kind,
        battery: Option<Battery>,
    }

    /// Reads the receivers' devices, keeping what is asked once of each (by its receiver and slot).
    #[derive(Default)]
    pub(super) struct Reader {
        known: HashMap<(String, u8), Known>,
        /// The software id the next question carries (2 to 15, in turn: 0 is the devices' own notifications', 1 is
        /// left to others), and the byte the next ping carries, which its answer gives back.
        software: u8,
        mark: u8,
    }

    impl Reader {
        pub fn read(&mut self) -> Vec<Device> {
            let mut devices = Vec::new();
            let mut present = Vec::new();
            for mut receiver in receivers() {
                present.push(receiver.id.clone());
                if !matches!(receiver.ask(RECEIVER, GET_REGISTER, CONNECTION_STATE, [0; 3]), Answer::Reply(_) | Answer::Refused(_)) {
                    continue;
                }
                for slot in SLOTS {
                    let key = (receiver.id.clone(), slot);
                    if !self.ping(&mut receiver, slot) {
                        self.known.remove(&key);
                        continue;
                    }
                    if !self.known.contains_key(&key) {
                        let Some(known) = self.discover(&mut receiver, slot) else { continue };
                        self.known.insert(key.clone(), known);
                    }
                    let known = &self.known[&key];
                    let Some(battery) = known.battery else { continue };
                    let (name, kind) = (known.name.clone(), known.kind);
                    match self.battery(&mut receiver, slot, battery) {
                        Some((level, charging)) => {
                            devices.push(Device { key: format!("hidpp:{}:{slot}", receiver.id), name, kind, via: Via::Logitech, level, charging, container: None })
                        }
                        // (Not answered as before: another device in its slot since, perhaps. Asked anew next time.)
                        None => {
                            self.known.remove(&key);
                        }
                    }
                }
            }
            self.known.retain(|(receiver, _), _| present.contains(receiver));
            devices
        }

        /// The software id for the next question.
        fn next(&mut self) -> u8 {
            self.software = if (2..15).contains(&self.software) { self.software + 1 } else { 2 };
            self.software
        }

        /// Whether the device in `slot` is joined and speaks HID++ 2.0.
        fn ping(&mut self, receiver: &mut Receiver, slot: u8) -> bool {
            self.mark = self.mark.wrapping_add(1);
            let mark = self.mark;
            let software = self.next();
            matches!(receiver.ask(slot, ROOT, PING << 4 | software, [0, 0, mark]), Answer::Reply(reply) if reply.get(2) == Some(&mark))
        }

        /// A feature's parameters, asked of the device in `slot`: function `function` of the feature at `index`.
        fn feature(&mut self, receiver: &mut Receiver, slot: u8, index: u8, function: u8, parameters: [u8; 3]) -> Option<Vec<u8>> {
            let software = self.next();
            match receiver.ask(slot, index, function << 4 | software, parameters) {
                Answer::Reply(reply) => Some(reply),
                _ => None,
            }
        }

        /// The index of the feature `id` on the device in `slot`: none where it has not that feature (Some(None)), or
        /// did not answer (None).
        fn index(&mut self, receiver: &mut Receiver, slot: u8, id: u16) -> Option<Option<u8>> {
            let [high, low] = id.to_be_bytes();
            let reply = self.feature(receiver, slot, ROOT, GET_FEATURE, [high, low, 0])?;
            Some(reply.first().copied().filter(|&index| index != 0))
        }

        /// What is asked once of the device in `slot`: none where it did not answer.
        fn discover(&mut self, receiver: &mut Receiver, slot: u8) -> Option<Known> {
            let (name, kind) = match self.index(receiver, slot, DEVICE_NAME)? {
                Some(index) => {
                    let length = *self.feature(receiver, slot, index, 0, [0; 3])?.first()? as usize;
                    let mut name = Vec::with_capacity(length);
                    while name.len() < length {
                        let part = self.feature(receiver, slot, index, 1, [name.len() as u8, 0, 0])?;
                        let taken = part.len().min(length - name.len());
                        if taken == 0 {
                            break;
                        }
                        name.extend_from_slice(&part[..taken]);
                    }
                    let kind = device_kind(*self.feature(receiver, slot, index, 2, [0; 3])?.first()?);
                    (String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_string(), kind)
                }
                // (A device that cannot say its name is not shown.)
                None => return Some(Known { name: String::new(), kind: Kind::Other, battery: None }),
            };
            let battery = if let Some(index) = self.index(receiver, slot, UNIFIED_BATTERY)? {
                Some(Battery::Unified(index))
            } else if let Some(index) = self.index(receiver, slot, BATTERY_STATUS)? {
                Some(Battery::Status(index))
            } else {
                self.index(receiver, slot, BATTERY_VOLTAGE)?.map(Battery::Voltage)
            };
            Some(Known { name, kind, battery })
        }

        /// The battery's level (percent) and whether it is charging, of the device in `slot`.
        fn battery(&mut self, receiver: &mut Receiver, slot: u8, battery: Battery) -> Option<(u8, bool)> {
            match battery {
                // (Function 1: its status. Function 0 tells what it can say.)
                Battery::Unified(index) => unified_battery(&self.feature(receiver, slot, index, 1, [0; 3])?),
                Battery::Status(index) => battery_status(&self.feature(receiver, slot, index, 0, [0; 3])?),
                Battery::Voltage(index) => battery_voltage(&self.feature(receiver, slot, index, 0, [0; 3])?),
            }
        }
    }

    /// What a device's type (feature 0x0005's function 2) says it is.
    pub(super) fn device_kind(kind: u8) -> Kind {
        match kind {
            // A keyboard, a numpad.
            0 | 2 => Kind::Keyboard,
            // A mouse, a touchpad, a trackball.
            3..=5 => Kind::Mouse,
            8 => Kind::Headset,
            // A joystick, a gamepad.
            11 | 12 => Kind::Gamepad,
            14 => Kind::Speaker,
            _ => Kind::Other,
        }
    }

    /// Feature 0x1004's status: its charge (percent; 0 where it does not tell one), its level as flags (full, good,
    /// low, critical, in bits 3 to 0: taken as Solaar takes them, as 90, 50, 20 and 5 percent, where it tells no
    /// charge), and whether it is charging (1) or charging slowly (2). None where it tells neither.
    pub(super) fn unified_battery(reply: &[u8]) -> Option<(u8, bool)> {
        let &[charge, flags, status, ..] = reply else { return None };
        let level = match charge {
            0 if flags & 0x08 != 0 => 90,
            0 if flags & 0x04 != 0 => 50,
            0 if flags & 0x02 != 0 => 20,
            0 if flags & 0x01 != 0 => 5,
            0 => return None,
            charge => charge.min(100),
        };
        Some((level, matches!(status, 1 | 2)))
    }

    /// Feature 0x1000's status: its level (percent; 0 where it does not know it), the next level it will tell, and
    /// whether it is recharging (1), almost full while charging (2) or charging slowly (4).
    pub(super) fn battery_status(reply: &[u8]) -> Option<(u8, bool)> {
        let &[level, _, status, ..] = reply else { return None };
        (level != 0).then(|| (level.min(100), matches!(status, 1 | 2 | 4)))
    }

    /// Feature 0x1001's status: the battery's voltage (millivolts, big-endian) and its flags, of which bit 7 says it is
    /// on external power and bits 0 and 1 then whether it is charging (0), full (1) or not charging (2).
    pub(super) fn battery_voltage(reply: &[u8]) -> Option<(u8, bool)> {
        let &[high, low, flags, ..] = reply else { return None };
        let millivolts = u16::from_be_bytes([high, low]);
        (millivolts != 0).then(|| (voltage_percent(millivolts), flags & 0x80 != 0 && flags & 0x03 == 0))
    }

    /// A lithium battery's charge (percent) at `millivolts`, between the points of Solaar's curve for Logitech's
    /// devices, in a straight line.
    pub(super) fn voltage_percent(millivolts: u16) -> u8 {
        const CURVE: [(u16, u8); 13] =
            [(4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50), (3778, 40), (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0)];
        if millivolts >= CURVE[0].0 {
            return 100;
        }
        for pair in CURVE.windows(2) {
            let ((high, at_high), (low, at_low)) = (pair[0], pair[1]);
            if millivolts >= low {
                let share = f32::from(millivolts - low) / f32::from(high - low);
                return (f32::from(at_low) + share * f32::from(at_high - at_low)).round() as u8;
            }
        }
        0
    }

    /// The answer `report` gives to the question asked of `device` with `sub` and `address` (its second and third
    /// bytes: HID++ 2.0's feature index, and function with software id; HID++ 1.0's sub id and register), where it
    /// answers that one.
    pub(super) fn answer(report: &[u8], device: u8, sub: u8, address: u8) -> Option<Answer> {
        if report.len() < 7 || ![SHORT, LONG, VERY_LONG].contains(&report[0]) || report[1] != device {
            return None;
        }
        if report[2] == sub && report[3] == address {
            return Some(Answer::Reply(report[4..].to_vec()));
        }
        match report[2] {
            ERROR_10 if report[3] == sub && report[4] == address => Some(Answer::Refused(report[5])),
            ERROR_20 if report[3] == sub && report[4] == address => Some(Answer::Failed(report[5])),
            _ => None,
        }
    }

    /// One of a receiver's HID++ collections, opened for overlapped reading and writing, with a read of it under way
    /// (into `buffer`, telling `overlapped`'s event) while `reading`.
    struct Collection {
        handle: HANDLE,
        overlapped: Box<OVERLAPPED>,
        buffer: Vec<u8>,
        reading: bool,
        /// The length of its output report.
        output: usize,
    }

    impl Collection {
        /// A read begun, where none is under way.
        fn read(&mut self) {
            if self.reading {
                return;
            }
            let began = unsafe { ReadFile(self.handle, Some(&mut self.buffer), None, Some(&mut *self.overlapped)) };
            self.reading = began.is_ok() || began.is_err_and(|error| error.code() == ERROR_IO_PENDING.to_hresult());
        }

        /// The report read, once the read under way has ended (its event told): none where it failed.
        fn report(&mut self) -> Option<Vec<u8>> {
            self.reading = false;
            let mut length = 0u32;
            unsafe { GetOverlappedResult(self.handle, &*self.overlapped, &mut length, false) }.ok()?;
            Some(self.buffer[..length as usize].to_vec())
        }
    }

    impl Drop for Collection {
        fn drop(&mut self) {
            if self.reading {
                let mut length = 0u32;
                let _ = unsafe { CancelIoEx(self.handle, Some(&*self.overlapped)) };
                let _ = unsafe { GetOverlappedResult(self.handle, &*self.overlapped, &mut length, true) };
            }
            let _ = unsafe { CloseHandle(self.overlapped.hEvent) };
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }

    /// A receiver: its USB interface's instance identifier, and its HID++ collections (short, long, very long:
    /// questions are written to the short one, or the long one where it has none; answers come on any).
    struct Receiver {
        id: String,
        collections: Vec<Collection>,
        written: HANDLE,
    }

    impl Drop for Receiver {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.written) };
        }
    }

    impl Receiver {
        /// Asks `device` the question of `sub`, `address` and `parameters` (see `answer`), and waits for its answer.
        fn ask(&mut self, device: u8, sub: u8, address: u8, parameters: [u8; 3]) -> Answer {
            for collection in &mut self.collections {
                collection.read();
            }
            let Some(writer) = self.collections.iter().position(|c| c.output == 7).or_else(|| self.collections.iter().position(|c| c.output == 20)) else {
                return Answer::Silent;
            };
            let length = self.collections[writer].output;
            let mut request = vec![0u8; length];
            request[..7].copy_from_slice(&[if length == 7 { SHORT } else { LONG }, device, sub, address, parameters[0], parameters[1], parameters[2]]);
            if !self.write(writer, &request) {
                return Answer::Silent;
            }
            let until = Instant::now() + WAITED;
            loop {
                let left = until.saturating_duration_since(Instant::now());
                let events: Vec<HANDLE> = self.collections.iter().filter(|c| c.reading).map(|c| c.overlapped.hEvent).collect();
                if events.is_empty() || left.is_zero() {
                    return Answer::Silent;
                }
                let woke = unsafe { WaitForMultipleObjects(&events, false, left.as_millis() as u32) };
                if woke == WAIT_TIMEOUT {
                    return Answer::Silent;
                }
                let told = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                let Some(collection) = events.get(told).and_then(|&event| self.collections.iter_mut().find(|c| c.reading && c.overlapped.hEvent == event)) else {
                    return Answer::Silent;
                };
                let report = collection.report();
                collection.read();
                if let Some(answer) = report.and_then(|report| answer(&report, device, sub, address)) {
                    return answer;
                }
            }
        }

        /// Writes `report` to the collection `writer`: whether it was written in time.
        fn write(&mut self, writer: usize, report: &[u8]) -> bool {
            let handle = self.collections[writer].handle;
            let mut overlapped = OVERLAPPED { hEvent: self.written, ..Default::default() };
            let began = unsafe { WriteFile(handle, Some(report), None, Some(&mut overlapped)) };
            if began.is_err_and(|error| error.code() != ERROR_IO_PENDING.to_hresult()) {
                return false;
            }
            let mut length = 0u32;
            if unsafe { GetOverlappedResultEx(handle, &overlapped, &mut length, WAITED.as_millis() as u32, false) }.is_ok() {
                return length as usize == report.len();
            }
            let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
            let _ = unsafe { GetOverlappedResult(handle, &overlapped, &mut length, true) };
            false
        }
    }

    /// Logitech's HID++ collections there are now (see `opened`), opened, by the USB interface they are of: each such interface's, as a receiver that may be (it is one if it answers as one).
    fn receivers() -> Vec<Receiver> {
        let size = || {
            let mut length = 0u32;
            (unsafe { CM_Get_Device_Interface_List_SizeW(&mut length, &GUID_DEVINTERFACE_HID, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) } == CR_SUCCESS)
                .then_some(length)
        };
        let list = |into: &mut [u16]| unsafe { CM_Get_Device_Interface_ListW(&GUID_DEVINTERFACE_HID, PCWSTR::null(), into, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        let mut by_interface: Vec<(String, Vec<Collection>)> = Vec::new();
        for path in multi_string(listed(size, list)) {
            if !path.to_ascii_lowercase().contains("vid_046d") {
                continue;
            }
            let Some(parent) = parent_of(&path) else { continue };
            let Some(collection) = opened(&path) else { continue };
            match by_interface.iter_mut().find(|(id, _)| *id == parent) {
                Some((_, collections)) => collections.push(collection),
                None => by_interface.push((parent, vec![collection])),
            }
        }
        by_interface
            .into_iter()
            .filter_map(|(id, collections)| {
                let written = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?;
                Some(Receiver { id, collections, written })
            })
            .collect()
    }

    /// The HID collection at `path`, opened, where it is one of HID++: vendor-defined (usage page 0xFF00), of its short
    /// reports (usage 1) or its long ones (usage 2).
    fn opened(path: &str) -> Option<Collection> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let handle =
            unsafe { CreateFileW(PCWSTR(wide.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None) }
                .ok()?;
        let mut caps = HIDP_CAPS::default();
        let mut data = PHIDP_PREPARSED_DATA::default();
        let read = unsafe { HidD_GetPreparsedData(handle, &mut data) } && {
            let status = unsafe { HidP_GetCaps(data, &mut caps) };
            let _ = unsafe { HidD_FreePreparsedData(data) };
            status == HIDP_STATUS_SUCCESS
        };
        let event = if read && caps.UsagePage == 0xFF00 && matches!(caps.Usage, 1 | 2) && caps.InputReportByteLength >= 7 {
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()
        } else {
            None
        };
        let Some(event) = event else {
            let _ = unsafe { CloseHandle(handle) };
            return None;
        };
        Some(Collection {
            handle,
            overlapped: Box::new(OVERLAPPED { hEvent: event, ..Default::default() }),
            buffer: vec![0; caps.InputReportByteLength as usize],
            reading: false,
            output: caps.OutputReportByteLength as usize,
        })
    }

    /// The instance identifier of the device a HID collection's interface (at `path`) is of: its USB interface.
    fn parent_of(path: &str) -> Option<String> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let mut id = [0u16; 512];
        let (mut kind, mut size) = (DEVPROPTYPE::default(), size_of_val(&id) as u32);
        let read = unsafe { CM_Get_Device_Interface_PropertyW(PCWSTR(wide.as_ptr()), &DEVPKEY_Device_InstanceId, &mut kind, Some(id.as_mut_ptr().cast()), &mut size, 0) };
        if read != CR_SUCCESS || kind != DEVPROP_TYPE_STRING {
            return None;
        }
        let node = located(&until_zero(&id))?;
        let mut parent = 0u32;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != CR_SUCCESS {
            return None;
        }
        let mut parent_id = [0u16; 512];
        (unsafe { CM_Get_Device_IDW(parent, &mut parent_id, 0) } == CR_SUCCESS).then(|| until_zero(&parent_id))
    }
}

/// Xbox controllers, as Windows' game controller API tells of them (Windows.Gaming.Input): each gamepad's battery
/// report, of one that has a battery (a wired one's says none is present).
///
/// Windows lists gamepads only to a program that hears of their coming, and only after telling it: so they are heard
/// of from the start, and each one's coming (or going) has them read again.
mod xbox {
    use super::*;

    /// What is heard while it is kept: the gamepads' coming and going.
    pub(super) struct Heard {
        added: i64,
        removed: i64,
    }

    impl Drop for Heard {
        fn drop(&mut self) {
            let _ = Gamepad::RemoveGamepadAdded(self.added);
            let _ = Gamepad::RemoveGamepadRemoved(self.removed);
        }
    }

    /// Has `told` sent to as a gamepad comes or goes, while what is given back is kept.
    pub(super) fn hear(told: Sender<()>) -> Option<Heard> {
        let gone = told.clone();
        let added = Gamepad::GamepadAdded(&EventHandler::new(move |_, _| {
            let _ = told.send(());
            Ok(())
        }))
        .ok()?;
        let removed = Gamepad::GamepadRemoved(&EventHandler::new(move |_, _| {
            let _ = gone.send(());
            Ok(())
        }))
        .ok()?;
        Some(Heard { added, removed })
    }

    /// The gamepads there are now that have a battery, with its level.
    pub(super) fn read() -> Vec<Device> {
        let Ok(gamepads) = Gamepad::Gamepads() else { return Vec::new() };
        let count = gamepads.Size().unwrap_or(0);
        (0..count).filter_map(|index| described(&gamepads.GetAt(index).ok()?)).collect()
    }

    fn described(gamepad: &Gamepad) -> Option<Device> {
        let report = gamepad.TryGetBatteryReport().ok()?;
        let status = report.Status().ok()?;
        let remaining = report.RemainingCapacityInMilliwattHours().ok()?.Value().ok()?;
        let full = report.FullChargeCapacityInMilliwattHours().ok()?.Value().ok()?;
        let level = level(status, remaining, full)?;
        let controller = RawGameController::FromGameController(gamepad).ok()?;
        Some(Device {
            key: format!("xbox:{}", controller.NonRoamableId().ok()?),
            name: controller.DisplayName().map(|name| name.to_string()).unwrap_or_default(),
            kind: Kind::Gamepad,
            via: Via::Xbox,
            level,
            charging: status == BatteryStatus::Charging,
            container: None,
        })
    }

    /// The level (percent) a battery report's capacities give, where a battery is there.
    pub(super) fn level(status: BatteryStatus, remaining: i32, full: i32) -> Option<u8> {
        (status != BatteryStatus::NotPresent && full > 0 && remaining >= 0).then(|| (i64::from(remaining) * 100 / i64::from(full)).min(100) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::bluetooth::{appearance_kind, class_kind, joined, Record};
    use super::logitech::{answer, battery_status, battery_voltage, device_kind, unified_battery, voltage_percent, Answer};
    use super::*;

    fn record(address: u64, container: u128, connected: bool) -> Record {
        Record { address, name: format!("device {address}"), container, connected, low_energy: false, class: None }
    }

    /// A device paired but not joined is not listed, though its level is still kept; one joined without a level is
    /// not listed either.
    #[test]
    fn only_joined_devices_with_a_level_are_listed() {
        let records = [record(1, 10, true), record(2, 20, false), record(3, 30, true)];
        let levels = [(10, 55), (20, 10)];
        let joined = joined(&records, &levels);
        assert_eq!(joined.len(), 1);
        assert_eq!((joined[0].0.address, joined[0].1), (1, 55));
    }

    /// A device of two nodes (classic and Low Energy) is listed once, by the node joined.
    #[test]
    fn a_device_of_two_nodes_is_listed_once() {
        let records = [record(5, 10, false), record(7, 10, true)];
        let joined = joined(&records, &[(10, 80), (10, 70)]);
        assert_eq!(joined.len(), 1);
        assert_eq!((joined[0].0.address, joined[0].1), (7, 80));
    }

    #[test]
    fn classes_of_device() {
        // The Sony WH-1000XM5's: audio/video, a wearable headset.
        assert_eq!(class_kind(0x240404), Kind::Headset);
        assert_eq!(class_kind(0x240418), Kind::Headphones);
        assert_eq!(class_kind(0x240414), Kind::Speaker);
        assert_eq!(class_kind(0x002540), Kind::Keyboard);
        assert_eq!(class_kind(0x002580), Kind::Mouse);
        assert_eq!(class_kind(0x0025C0), Kind::Keyboard);
        assert_eq!(class_kind(0x002508), Kind::Gamepad);
        assert_eq!(class_kind(0x5A020C), Kind::Phone);
        assert_eq!(class_kind(0x000704), Kind::Watch);
        assert_eq!(class_kind(0x000100), Kind::Other);
    }

    #[test]
    fn appearances() {
        assert_eq!(appearance_kind(0x03C1), Kind::Keyboard);
        assert_eq!(appearance_kind(0x03C2), Kind::Mouse);
        assert_eq!(appearance_kind(0x03C4), Kind::Gamepad);
        assert_eq!(appearance_kind(0x0941), Kind::Earbuds);
        assert_eq!(appearance_kind(0x0942), Kind::Headset);
        assert_eq!(appearance_kind(0x0943), Kind::Headphones);
        assert_eq!(appearance_kind(0x0841), Kind::Speaker);
        assert_eq!(appearance_kind(0x00C2), Kind::Watch);
        assert_eq!(appearance_kind(0x0040), Kind::Phone);
        assert_eq!(appearance_kind(0x0000), Kind::Other);
    }

    /// A long answer to a ping of slot 2 (feature index 0, function 1, software id 5), with the ping's byte given back.
    #[test]
    fn an_answer_is_matched_to_its_question() {
        let reply = [0x11, 0x02, 0x00, 0x15, 0x04, 0x05, 0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(answer(&reply, 2, 0x00, 0x15), Some(Answer::Reply(reply[4..].to_vec())));
        // Another slot's, another software id's (Options+'s), a notification's (software id 0): not this question's.
        assert_eq!(answer(&reply, 3, 0x00, 0x15), None);
        assert_eq!(answer(&reply, 2, 0x00, 0x16), None);
        let notification = [0x11, 0x02, 0x00, 0x10, 0x04, 0x05, 0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(answer(&notification, 2, 0x00, 0x15), None);
    }

    /// The receiver's refusal of a question to a device not joined (HID++ 1.0's error 0x09), and a device's HID++ 2.0
    /// error.
    #[test]
    fn errors_are_matched_to_their_question() {
        assert_eq!(answer(&[0x10, 0x03, 0x8F, 0x00, 0x1A, 0x09, 0x00], 3, 0x00, 0x1A), Some(Answer::Refused(0x09)));
        assert_eq!(answer(&[0x10, 0x03, 0x8F, 0x00, 0x1B, 0x09, 0x00], 3, 0x00, 0x1A), None);
        let failed = [0x11, 0x01, 0xFF, 0x04, 0x13, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(answer(&failed, 1, 0x04, 0x13), Some(Answer::Failed(0x02)));
        // The receiver's own register read, and a short report too short to be one.
        assert_eq!(answer(&[0x10, 0xFF, 0x81, 0x02, 0x00, 0x02, 0x00], 0xFF, 0x81, 0x02), Some(Answer::Reply(vec![0x00, 0x02, 0x00])));
        assert_eq!(answer(&[0x10, 0xFF, 0x81, 0x02], 0xFF, 0x81, 0x02), None);
        assert_eq!(answer(&[0x20, 0xFF, 0x81, 0x02, 0, 0, 0], 0xFF, 0x81, 0x02), None);
    }

    #[test]
    fn unified_battery_status() {
        // 64 %, good, discharging.
        assert_eq!(unified_battery(&[64, 0x04, 0, 0]), Some((64, false)));
        // 30 %, charging; charging slowly; charged.
        assert_eq!(unified_battery(&[30, 0x04, 1, 1]), Some((30, true)));
        assert_eq!(unified_battery(&[30, 0x04, 2, 1]), Some((30, true)));
        assert_eq!(unified_battery(&[100, 0x08, 3, 1]), Some((100, false)));
        // No charge told: its level's flags.
        assert_eq!(unified_battery(&[0, 0x08, 0, 0]), Some((90, false)));
        assert_eq!(unified_battery(&[0, 0x02, 0, 0]), Some((20, false)));
        assert_eq!(unified_battery(&[0, 0x01, 0, 0]), Some((5, false)));
        assert_eq!(unified_battery(&[0, 0x00, 0, 0]), None);
        assert_eq!(unified_battery(&[50, 0x04]), None);
    }

    #[test]
    fn battery_status_levels() {
        assert_eq!(battery_status(&[50, 20, 0]), Some((50, false)));
        assert_eq!(battery_status(&[50, 20, 1]), Some((50, true)));
        assert_eq!(battery_status(&[90, 50, 2]), Some((90, true)));
        assert_eq!(battery_status(&[100, 90, 3]), Some((100, false)));
        assert_eq!(battery_status(&[40, 20, 4]), Some((40, true)));
        assert_eq!(battery_status(&[0, 0, 0]), None);
    }

    #[test]
    fn battery_voltages() {
        // 3.811 V, discharging; 3.9 V, on power and charging; 4.2 V on power and full; on power, not charging.
        assert_eq!(battery_voltage(&[0x0E, 0xE3, 0x00]), Some((50, false)));
        assert_eq!(battery_voltage(&[0x0F, 0x3C, 0x80]), Some((67, true)));
        assert_eq!(battery_voltage(&[0x10, 0x68, 0x81]), Some((100, false)));
        assert_eq!(battery_voltage(&[0x0F, 0x3C, 0x82]), Some((67, false)));
        assert_eq!(battery_voltage(&[0x00, 0x00, 0x00]), None);
    }

    #[test]
    fn voltages_to_percent() {
        assert_eq!(voltage_percent(4300), 100);
        assert_eq!(voltage_percent(4186), 100);
        assert_eq!(voltage_percent(4067), 90);
        assert_eq!(voltage_percent(3835), 55);
        assert_eq!(voltage_percent(3500), 0);
        assert_eq!(voltage_percent(3400), 0);
        assert_eq!(voltage_percent(3612), 3);
    }

    #[test]
    fn device_types() {
        assert_eq!(device_kind(0), Kind::Keyboard);
        assert_eq!(device_kind(3), Kind::Mouse);
        assert_eq!(device_kind(5), Kind::Mouse);
        assert_eq!(device_kind(8), Kind::Headset);
        assert_eq!(device_kind(12), Kind::Gamepad);
        assert_eq!(device_kind(7), Kind::Other);
    }

    #[test]
    fn controller_levels() {
        assert_eq!(xbox::level(BatteryStatus::Discharging, 50, 100), Some(50));
        assert_eq!(xbox::level(BatteryStatus::Charging, 100, 100), Some(100));
        assert_eq!(xbox::level(BatteryStatus::NotPresent, 0, 0), None);
        assert_eq!(xbox::level(BatteryStatus::Idle, 10, 0), None);
    }

    fn device(key: &str, name: &str, kind: Kind) -> Device {
        Device { key: key.into(), name: name.into(), kind, via: Via::Bluetooth, level: 50, charging: false, container: None }
    }

    #[test]
    fn devices_are_ordered_by_kind_then_name() {
        let ordered = ordered(vec![
            device("c", "mx master", Kind::Mouse),
            device("b", "WH-1000XM5", Kind::Headset),
            device("a", "MX Keys", Kind::Keyboard),
            device("d", "Buds", Kind::Earbuds),
            device("e", "Anker", Kind::Mouse),
        ]);
        let keys: Vec<&str> = ordered.iter().map(|device| device.key.as_str()).collect();
        assert_eq!(keys, ["d", "b", "a", "e", "c"]);
    }

    #[test]
    #[ignore = "reads this machine's wireless devices"]
    fn reads_the_devices() {
        start();
        // (Controllers are listed only after Windows has told of them.)
        std::thread::sleep(Duration::from_secs(8));
        for device in now() {
            println!("{device:?}");
        }
    }
}
