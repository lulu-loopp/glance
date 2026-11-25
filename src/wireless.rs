//! The batteries of the wireless devices joined to the PC: Bluetooth headphones, keyboards, mice and speakers (the
//! level Windows itself shows in its Bluetooth settings), the devices of Logitech's receivers (Unifying, Bolt,
//! Lightspeed: asked over HID++, as Logitech's own software asks), keyboards of PMO Hub's firmware on their 2.4 GHz
//! receiver (as PMO Hub's web driver asks), and Xbox controllers (as Windows' game controller API tells). Only devices
//! joined now and whose level is known are listed (a receiver's device asleep as last read).
//!
//! All of it is read on a thread of its own: at the start, then every half minute (Logitech's devices every minute,
//! the PMO boards every five, as each is asked over the air), and a moment after a device comes or goes. Nothing is set on any device: every
//! question asked of one only reads. No rights are needed.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver as Told, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{GUID, PCWSTR};
use windows::Devices::Bluetooth::BluetoothLEDevice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{IInspectable, HSTRING};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::{EventHandler, TypedEventHandler};
use windows_collections::IIterable;
use windows::Gaming::Input::{Gamepad, RawGameController};
use windows::System::Power::BatteryStatus;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_IDW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_PropertyW, CM_Get_Parent, CM_Locate_DevNodeW, CM_Register_Notification, CM_Unregister_Notification, CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT,
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
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};

/// A wireless device joined to the PC, its battery level as last read.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Percent, 0 to 100: none where it is joined (its receiver holds it) but its charge was never read (asleep since
    /// Glance started, or not telling it).
    pub level: Option<u8>,
    /// Whether it is being charged now (a Bluetooth device's level does not say: false for them).
    pub charging: bool,
    /// The Plug and Play container of a Bluetooth device (`DEVPKEY_Device_ContainerId`), which its sound outputs
    /// share: none for the others.
    pub container: Option<u128>,
}

/// What a device is, as far as it tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Via {
    Bluetooth,
    /// Through one of Logitech's receivers.
    Logitech,
    /// As an Xbox controller (through Xbox's own wireless adapter, or Bluetooth).
    Xbox,
    /// Through a keyboard's own 2.4 GHz receiver.
    Receiver,
    /// On its cable (a wireless keyboard plugged in, as its maker's protocol still reads it).
    Cable,
}

/// The devices as last read, ordered as `now` gives them.
static DEVICES: Mutex<Vec<Device>> = Mutex::new(Vec::new());

/// How often everything is read again.
const EVERY: Duration = Duration::from_secs(30);
/// How often Logitech's devices are asked again (each answers over the air).
const LOGITECH_EVERY: Duration = Duration::from_secs(60);
/// How often the PMO boards are asked again: seldom, as each question goes over the air to the board (its charge
/// changes slowly, and between askings it is shown as last read). A receiver whose board did not answer (asleep, out of
/// reach) is asked again as often as everything is read.
const BOARDS_EVERY: Duration = Duration::from_secs(300);
/// How long Windows' telling of devices coming and going must be quiet before they are read again (a device comes
/// with several interfaces at once), and the longest they wait for that.
const SETTLED: Duration = Duration::from_secs(2);
const SETTLING_AT_MOST: Duration = Duration::from_secs(10);

/// What wakes the reader before its time: Windows telling of a device (an interface come or gone, a Bluetooth device
/// joined or left or its level changed), read once Windows has gone quiet again, someone about to look at the devices
/// (the panel opened), read at once, or the devices no longer wanted (their module switched off), the reader ended.
enum Wake {
    Told,
    Asked,
    Leave,
}

/// What the reader is woken by, while it runs.
static WAKE: Mutex<Option<Sender<Wake>>> = Mutex::new(None);
/// Whether the devices are wanted (see `want`), and whether the reader's thread runs.
static WANTED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The receivers (by vendor and product, as `hardware` gives them) a keyboard on which went unanswered at the last
/// reading: asleep, as it is woken by a key pressed on it (see `keys_from`).
static WAITING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// How soon after a keyboard heard (see `keys_from`) everything is asked again, at most: a keyboard woken by a key is
/// asked once, not at every key pressed while it is asked.
const HEARD_AGAIN: Duration = Duration::from_secs(5);

/// Whether keyboards are to be heard now (see `keys_from`): while one asleep on its receiver is waited for.
pub fn hears_keys() -> bool {
    !WAITING.lock().unwrap().is_empty()
}

/// A keyboard's input heard, from the raw input device named `device` (its path: nothing of what was pressed): where
/// it came through a receiver whose keyboard went unanswered (asleep, and woken now by the key), everything is read
/// again at once, as often as `HEARD_AGAIN`.
pub fn keys_from(device: &str) {
    static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
    if !waited_for(device, &WAITING.lock().unwrap()) {
        return;
    }
    let mut asked = ASKED.lock().unwrap();
    if asked.is_some_and(|at| at.elapsed() < HEARD_AGAIN) {
        return;
    }
    *asked = Some(Instant::now());
    ask();
}

/// Whether the device whose path (or instance id) is `device` is one of the receivers `waiting`.
fn waited_for(device: &str, waiting: &[String]) -> bool {
    hardware(device).is_some_and(|id| waiting.contains(&id))
}

/// The vendor and product a device's path or instance id carries ("vid_046d&pid_c548"), in lower case: one receiver's
/// collections, and the keyboards that come through it, carry the same.
fn hardware(device: &str) -> Option<String> {
    let lower = device.to_ascii_lowercase();
    let at = lower.find("vid_")?;
    let id = lower.get(at..at + 17)?;
    (id.get(8..13) == Some("&pid_")).then(|| id.to_string())
}

/// Whether the devices are wanted (their module on): the reader's thread started as they first are (later calls do
/// nothing while it runs); no longer, it ends, and nothing is read, heard or asked of a device till they are again
/// (none shown meanwhile, and no keyboard waited for).
pub fn want(on: bool) {
    WANTED.store(on, Ordering::Release);
    if !on {
        if let Some(wake) = WAKE.lock().unwrap().take() {
            let _ = wake.send(Wake::Leave);
        }
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let (wake, woken) = channel();
    *WAKE.lock().unwrap() = Some(wake.clone());
    std::thread::Builder::new()
        .name("glance: wireless batteries".into())
        .spawn(move || {
            run(wake, woken);
            DEVICES.lock().unwrap().clear();
            WAITING.lock().unwrap().clear();
            RUNNING.store(false, Ordering::Release);
            // (Wanted again as it ended: run again.)
            if WANTED.load(Ordering::Acquire) {
                want(true);
            }
        })
        .expect("a thread started");
}

/// Has everything read again now, every device asked (they are about to be looked at): the devices then as `now` gives
/// them a moment later. Nothing where the reader does not run.
pub fn ask() {
    if let Some(wake) = WAKE.lock().unwrap().as_ref() {
        let _ = wake.send(Wake::Asked);
    }
}

/// The devices joined now, as last read: by kind, then by name.
pub fn now() -> Vec<Device> {
    DEVICES.lock().unwrap().clone()
}

/// The reader: everything read, then read again every `EVERY`, or once Windows has told of a device coming or going
/// and gone quiet again; till it is told to leave (see `want`).
fn run(told: Sender<Wake>, telling: Told<Wake>) {
    crate::actions::com();
    let _devices = hear_devices(told.clone());
    // (Kept for as long as the thread runs: Windows lists controllers only to a program that hears of them, and tells
    // of Bluetooth devices' properties only while its watcher of them runs.)
    let _controllers = xbox::hear(told.clone());
    let _bluetooth = bluetooth::hear(told);
    let mut bluetooth = bluetooth::Reader::default();
    // (The receivers' devices as last read, before Glance last ended: one asleep shows so from the start.)
    let mut saved = kept::load();
    let mut logitech = logitech::Reader::seeded(&saved.logitech);
    let mut boards = pmo::Reader::seeded(&saved.pmo);
    let mut boards_read: Option<(Instant, Vec<Device>)> = None;
    let mut logitech_read: Option<(Instant, Vec<Device>)> = None;
    let mut asked = false;
    loop {
        if asked || logitech_read.as_ref().is_none_or(|(at, _)| at.elapsed() >= LOGITECH_EVERY) {
            logitech_read = Some((Instant::now(), logitech.read()));
        }
        let mut devices = bluetooth.read();
        devices.extend(logitech_read.as_ref().map(|(_, devices)| devices.clone()).unwrap_or_default());
        let boards_every = if boards.waiting().is_empty() { BOARDS_EVERY } else { EVERY };
        if asked || boards_read.as_ref().is_none_or(|(at, _)| at.elapsed() >= boards_every) {
            boards_read = Some((Instant::now(), boards.read()));
        }
        *WAITING.lock().unwrap() = boards.waiting().into_iter().chain(logitech.waiting()).collect();
        let now_kept = kept::Kept { pmo: boards.kept(), logitech: logitech.kept() };
        if now_kept != saved {
            kept::save(&now_kept);
            saved = now_kept;
        }
        devices.extend(boards_read.as_ref().map(|(_, devices)| devices.clone()).unwrap_or_default());
        devices.extend(xbox::read());
        *DEVICES.lock().unwrap() = ordered(devices);
        asked = match settled(&telling) {
            Some(asked) => asked,
            None => return,
        };
    }
}

/// The devices of the receivers as last read, kept on disk from one run of Glance to the next: a device asleep when
/// Glance starts shows as it was last read while its receiver is plugged in (as it would have, had Glance run on).
mod kept {
    use super::*;

    /// Each PMO board, by the path of the receiver or cable it was read through, and each Logitech device, by its
    /// receiver's instance and its slot.
    #[derive(Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
    pub(super) struct Kept {
        pub pmo: Vec<(String, Device)>,
        pub logitech: Vec<logitech::KeptDevice>,
    }

    fn file() -> std::path::PathBuf {
        crate::settings::local_dir().join("wireless.json")
    }

    /// As last saved: none where there is no file, or it does not read.
    pub(super) fn load() -> Kept {
        std::fs::read(file()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub(super) fn save(kept: &Kept) {
        if let Ok(text) = serde_json::to_vec(kept) {
            let _ = std::fs::write(file(), text);
        }
    }
}

/// Waits `EVERY`, or for Windows' telling of devices to begin and then go quiet, or for an asking: whether it was told
/// or asked (none where it is told to leave, or no one can tell any more).
fn settled(telling: &Told<Wake>) -> Option<bool> {
    match telling.recv_timeout(EVERY) {
        Ok(Wake::Told) => {}
        Ok(Wake::Asked) => return heard_out(telling),
        Ok(Wake::Leave) | Err(RecvTimeoutError::Disconnected) => return None,
        Err(RecvTimeoutError::Timeout) => return Some(false),
    }
    let since = Instant::now();
    while since.elapsed() < SETTLING_AT_MOST {
        match telling.recv_timeout(SETTLED) {
            Ok(Wake::Told) => {}
            Ok(Wake::Asked) => return heard_out(telling),
            Ok(Wake::Leave) | Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => break,
        }
    }
    Some(true)
}

/// Takes what else was told or asked meanwhile (all of it answered by the one reading about to be done): true, or none
/// where it was told to leave meanwhile.
fn heard_out(telling: &Told<Wake>) -> Option<bool> {
    let mut leave = false;
    while let Ok(wake) = telling.try_recv() {
        leave |= matches!(wake, Wake::Leave);
    }
    (!leave).then_some(true)
}

/// How long Windows is waited for to tell what is asked of it over the air (a Bluetooth device's appearance), at most:
/// the other devices are read after it.
const ASKED_AT_MOST: Duration = Duration::from_secs(3);

/// What `operation` gives, where it completes within `within`: else it is cancelled, and none.
fn completed<T: windows::core::RuntimeType + 'static>(operation: windows_future::IAsyncOperation<T>, within: Duration) -> Option<T> {
    use windows_future::AsyncStatus;
    let until = Instant::now() + within;
    loop {
        match operation.Status().ok()? {
            AsyncStatus::Completed => return operation.GetResults().ok(),
            AsyncStatus::Started if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            AsyncStatus::Started => {
                let _ = operation.Cancel();
                return None;
            }
            _ => return None,
        }
    }
}

/// The devices in the order they are shown: by kind, then by name, then by key (two of one name in a stable order).
fn ordered(mut devices: Vec<Device>) -> Vec<Device> {
    devices.sort_by(|a, b| (a.kind as u8, a.name.to_lowercase(), &a.key).cmp(&(b.kind as u8, b.name.to_lowercase(), &b.key)));
    devices.dedup_by(|a, b| a.key == b.key);
    devices
}

/// Has `told` sent to as any device interface comes or goes (a receiver plugged in, a Bluetooth device joined, whose
/// sound or input then comes), for as long as the program runs.
fn hear_devices(told: Sender<Wake>) -> DevicesHeard {
    let filter = CM_NOTIFY_FILTER {
        cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
        Flags: CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        ..Default::default()
    };
    // (Let go of as what is given back is: see `DevicesHeard`.)
    let context = Box::into_raw(Box::new(told));
    let mut heard = HCMNOTIFICATION::default();
    let registered = unsafe { CM_Register_Notification(&filter, Some(context as *const core::ffi::c_void), Some(interface_changed), &mut heard) } == CR_SUCCESS;
    DevicesHeard { heard: registered.then_some(heard), context }
}

/// Windows' telling of device interfaces heard (see `hear_devices`) while it is kept: dropped, no longer, and what it
/// told to let go of.
struct DevicesHeard {
    heard: Option<HCMNOTIFICATION>,
    context: *mut Sender<Wake>,
}

impl Drop for DevicesHeard {
    fn drop(&mut self) {
        // (Unregistered, no telling is under way or comes after: `CM_Unregister_Notification` waits for one under way.)
        if let Some(heard) = self.heard {
            unsafe { CM_Unregister_Notification(heard) };
        }
        drop(unsafe { Box::from_raw(self.context) });
    }
}

unsafe extern "system" fn interface_changed(_: HCMNOTIFICATION, context: *const core::ffi::c_void, action: CM_NOTIFY_ACTION, _: *const CM_NOTIFY_EVENT_DATA, _: u32) -> u32 {
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
        let told = unsafe { &*(context as *const Sender<Wake>) };
        let _ = told.send(Wake::Told);
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

    /// Windows' watcher of Bluetooth's device nodes, telling as a device is joined or leaves or its level changes (the
    /// stack sets the level a moment after the device is joined): stops as it is dropped.
    pub(super) struct Heard(DeviceWatcher);

    impl Drop for Heard {
        fn drop(&mut self) {
            let _ = self.0.Stop();
        }
    }

    /// Has `told` sent to as a Bluetooth device node is added or removed once the nodes there are have been listed, or
    /// as its being joined or its battery level changes.
    pub(super) fn hear(told: Sender<Wake>) -> Option<Heard> {
        let watcher = watcher().ok()?;
        // (The nodes there are as it starts are each told as added: not news.)
        let listed = Arc::new(AtomicBool::new(false));
        let tell = || {
            let (told, listed) = (told.clone(), listed.clone());
            move || {
                if listed.load(Ordering::Relaxed) {
                    let _ = told.send(Wake::Told);
                }
                Ok(())
            }
        };
        let (added, removed, updated) = (tell(), tell(), tell());
        watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, _| added())).ok()?;
        watcher.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, _| removed())).ok()?;
        watcher.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, _| updated())).ok()?;
        watcher
            .EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
                listed.store(true, Ordering::Relaxed);
                Ok(())
            }))
            .ok()?;
        watcher.Start().ok()?;
        Some(Heard(watcher))
    }

    /// A watcher of Bluetooth's device nodes (see `ENUMERATORS`), asked to tell of their being joined and their levels.
    pub(super) fn watcher() -> windows::core::Result<DeviceWatcher> {
        // (By their instance identifiers' beginning, the enumerator's name: Windows' device queries do not filter on the
        // enumerator itself.)
        let filter: Vec<String> = ENUMERATORS.iter().map(|enumerator| format!("System.Devices.DeviceInstanceId:~<\"{enumerator}\\\"")).collect();
        let properties: IIterable<HSTRING> = vec![HSTRING::from(property_name(&CONNECTED)), HSTRING::from(property_name(&BATTERY_LEVEL))].into();
        DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(&HSTRING::from(filter.join(" OR ")), &properties, DeviceInformationKind::Device)
    }

    /// A property's name as Windows' device queries take it: its format id in braces, and its number.
    pub(super) fn property_name(key: &DEVPROPKEY) -> String {
        format!("{{{:?}}} {}", key.fmtid, key.pid)
    }

    /// Reads Bluetooth devices, keeping what Low Energy devices said they are (asked of Windows once each).
    #[derive(Default)]
    pub(super) struct Reader {
        /// Each Low Energy device's appearance, as it was asked once (none where it was not told in time).
        appearances: HashMap<u64, Option<u16>>,
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
                    Device { key: format!("bt:{:012x}", record.address), name: record.name, kind, via: Via::Bluetooth, level: Some(level), charging: false, container: Some(record.container) }
                })
                .collect()
        }

        /// What a Low Energy device says it is (its GAP appearance), as Windows keeps it from its pairing: asked once.
        fn appearance(&mut self, address: u64) -> Option<u16> {
            if let Some(&appearance) = self.appearances.get(&address) {
                return appearance;
            }
            // (Asked of Windows, which may ask the device: not waited for longer than `ASKED_AT_MOST`, nor asked again.)
            let appearance = completed(BluetoothLEDevice::FromBluetoothAddressAsync(address).ok()?, ASKED_AT_MOST).and_then(|device| device.Appearance().ok()?.RawValue().ok());
            self.appearances.insert(address, appearance);
            appearance
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
            // Audio/video: headphones (and a wearable headset, as nearly every pair with a microphone says it is: its
            // sound played on it as on headphones, the sound's page draws it so), a hands-free device, a loudspeaker,
            // portable audio, hi-fi.
            0x04 => match minor {
                0x01 | 0x06 => Kind::Headphones,
                0x02 => Kind::Headset,
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

/// HID collections spoken to directly (a receiver's, a keyboard's own channel): found by their vendor, opened shared
/// for overlapped reading and writing, so that the makers' own software goes on speaking to them too.
mod hid {
    use super::*;

    /// The paths of the HID collections there are now of the vendor `vendor` (its USB vendor id, four hex digits).
    pub(super) fn paths(vendor: &str) -> Vec<String> {
        let size = || {
            let mut length = 0u32;
            (unsafe { CM_Get_Device_Interface_List_SizeW(&mut length, &GUID_DEVINTERFACE_HID, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) } == CR_SUCCESS)
                .then_some(length)
        };
        let list = |into: &mut [u16]| unsafe { CM_Get_Device_Interface_ListW(&GUID_DEVINTERFACE_HID, PCWSTR::null(), into, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        let wanted = format!("vid_{}", vendor.to_ascii_lowercase());
        multi_string(listed(size, list)).into_iter().filter(|path| path.to_ascii_lowercase().contains(&wanted)).collect()
    }

    /// The product id a collection's path names (its "PID_" and four hex digits).
    pub(super) fn product(path: &str) -> Option<u16> {
        let lower = path.to_ascii_lowercase();
        let at = lower.find("pid_")? + 4;
        u16::from_str_radix(lower.get(at..at + 4)?, 16).ok()
    }

    /// A HID collection, opened for overlapped reading and writing, with a read of it under way (into `buffer`, telling
    /// `overlapped`'s event) while `reading`.
    pub(super) struct Collection {
        pub handle: HANDLE,
        pub overlapped: Box<OVERLAPPED>,
        buffer: Vec<u8>,
        pub reading: bool,
        /// The length of its output report.
        pub output: usize,
        /// What a write tells as it ends.
        written: HANDLE,
    }

    impl Collection {
        /// The collection at `path`, opened, where its capabilities are `wanted`.
        pub fn open(path: &str, wanted: impl Fn(&HIDP_CAPS) -> bool) -> Option<Collection> {
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
            let events = (read && wanted(&caps))
                .then(|| Some((unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?, unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?)))
                .flatten();
            let Some((event, written)) = events else {
                let _ = unsafe { CloseHandle(handle) };
                return None;
            };
            Some(Collection {
                handle,
                overlapped: Box::new(OVERLAPPED { hEvent: event, ..Default::default() }),
                buffer: vec![0; caps.InputReportByteLength as usize],
                reading: false,
                output: caps.OutputReportByteLength as usize,
                written,
            })
        }

        /// A read begun, where none is under way.
        pub fn read(&mut self) {
            if self.reading {
                return;
            }
            let began = unsafe { ReadFile(self.handle, Some(&mut self.buffer), None, Some(&mut *self.overlapped)) };
            self.reading = began.is_ok() || began.is_err_and(|error| error.code() == ERROR_IO_PENDING.to_hresult());
        }

        /// The report read, once the read under way has ended (its event told): none where it failed.
        pub fn report(&mut self) -> Option<Vec<u8>> {
            self.reading = false;
            let mut length = 0u32;
            unsafe { GetOverlappedResult(self.handle, &*self.overlapped, &mut length, false) }.ok()?;
            Some(self.buffer[..length as usize].to_vec())
        }

        /// The next report, within `left`: a read begun where none is under way, and another begun once it ends.
        pub fn next(&mut self, left: Duration) -> Option<Vec<u8>> {
            self.read();
            if !self.reading || unsafe { WaitForSingleObject(self.overlapped.hEvent, left.as_millis() as u32) } != WAIT_OBJECT_0 {
                return None;
            }
            let report = self.report();
            self.read();
            report
        }

        /// Writes `report`: whether it was written within `waited`.
        pub fn write(&mut self, report: &[u8], waited: Duration) -> bool {
            let mut overlapped = OVERLAPPED { hEvent: self.written, ..Default::default() };
            let began = unsafe { WriteFile(self.handle, Some(report), None, Some(&mut overlapped)) };
            if began.is_err_and(|error| error.code() != ERROR_IO_PENDING.to_hresult()) {
                return false;
            }
            let mut length = 0u32;
            if unsafe { GetOverlappedResultEx(self.handle, &overlapped, &mut length, waited.as_millis() as u32, false) }.is_ok() {
                return length as usize == report.len();
            }
            let _ = unsafe { CancelIoEx(self.handle, Some(&overlapped)) };
            let _ = unsafe { GetOverlappedResult(self.handle, &overlapped, &mut length, true) };
            false
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
            let _ = unsafe { CloseHandle(self.written) };
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }
}

/// Keyboards of PMO Hub's firmware (PMO's and WOBKEY's: the Wave 75, the Aurora 80, the P75, the 80) through their
/// 2.4 GHz receiver (the "Wireless 2.4G Dongle", 36B0:3002, which the boards of many makers share) or on their cable,
/// asked over the keyboard's own raw HID channel (usage page 0xFF60, usage 0x61) as PMO Hub's web driver asks: a
/// session opened (as it opens, the receiver tells which board is paired to it), the battery asked, the session closed.
/// Only reads. A board that does not answer (asleep, or on its cable) is listed as last read while its receiver stays
/// plugged in.
mod pmo {
    use super::*;

    const VENDOR: u16 = 0x36B0;
    /// The receiver the boards share.
    const RECEIVER: u16 = 0x3002;
    /// The boards, by their own product id (as the receiver tells it, and as their cable is), and their names.
    const BOARDS: [(u16, &str); 4] = [(0x3009, "WAVE 75"), (0x3004, "Aurora 80"), (0x302B, "PMO P75"), (0x3012, "PMO 80")];
    /// A frame's first byte, its commands (a session opened, closed, the battery asked) and the mark of a reply that
    /// did what was asked.
    const FRAME: u8 = 0xAA;
    const START: u8 = 0x10;
    const STOP: u8 = 0x11;
    const BATTERY: u8 = 0x30;
    const DONE: u8 = 0x55;
    /// The length the battery is asked with.
    const BATTERY_LENGTH: u8 = 0x18;
    /// How long a question waits for its answer.
    const WAITED: Duration = Duration::from_millis(600);

    /// Reads the boards, keeping each as it was last read, by the receiver or cable it was read through.
    #[derive(Default)]
    pub(super) struct Reader {
        last: HashMap<String, Device>,
        /// The receivers (by `hardware`) whose board did not answer at the last reading.
        unanswered: Vec<String>,
    }

    impl Reader {
        /// A reader that has read `last` (see `kept`).
        pub fn seeded(last: &[(String, Device)]) -> Reader {
            Reader { last: last.iter().cloned().collect(), unanswered: Vec::new() }
        }

        /// The boards as last read, by where they were read through, in order.
        pub fn kept(&self) -> Vec<(String, Device)> {
            let mut kept: Vec<(String, Device)> = self.last.iter().map(|(path, device)| (path.clone(), device.clone())).collect();
            kept.sort_by(|a, b| a.0.cmp(&b.0));
            kept
        }

        /// The receivers (by `hardware`) whose board went unanswered at the last reading (to be asked again soon, and
        /// as soon as a key is pressed on it).
        pub fn waiting(&self) -> Vec<String> {
            self.unanswered.clone()
        }

        pub fn read(&mut self) -> Vec<Device> {
            let mut answers = Vec::new();
            for path in hid::paths(&format!("{VENDOR:04x}")) {
                let Some(product) = hid::product(&path).filter(|product| *product == RECEIVER || BOARDS.iter().any(|(board, _)| board == product)) else { continue };
                let Some(mut channel) = hid::Collection::open(&path, |caps| caps.UsagePage == 0xFF60 && caps.Usage == 0x61 && matches!(caps.OutputReportByteLength, 33 | 65))
                else {
                    continue;
                };
                let answer = battery(&mut channel);
                answers.push((path, product, answer));
            }
            self.took(answers)
        }

        /// Takes in what each receiver or cable plugged in answered (its path, its product, and the board it named and
        /// that board's charge, as `battery` gives them): the boards shown. A board joined is shown whether or not it
        /// answered: as read now; else as last read (through there), where its receiver names no other board than the
        /// one read; else unread, as the board its receiver names, or a keyboard where it names none. (Another board
        /// paired to it since, the one read before is forgotten there: it was shown with that one's charge.)
        pub(super) fn took(&mut self, answers: Vec<Answered>) -> Vec<Device> {
            let (mut read, mut kept) = (Vec::new(), Vec::new());
            self.unanswered.clear();
            for (path, product, (named, level)) in &answers {
                let board = named.unwrap_or(*product);
                let name = BOARDS.iter().find(|(id, _)| *id == board).map_or("", |(_, name)| name).to_string();
                let via = if *product == RECEIVER { Via::Receiver } else { Via::Cable };
                // (On its cable, it is charged by it.)
                let device = Device { key: format!("pmo:{board:04x}"), name, kind: Kind::Keyboard, via, level: *level, charging: via == Via::Cable, container: None };
                if level.is_some() {
                    // (Kept as last read through another way in, it is as read now there too.)
                    for other in self.last.values_mut().filter(|other| other.key == device.key) {
                        other.level = device.level;
                    }
                    self.last.insert(path.clone(), device.clone());
                    read.push(device);
                    continue;
                }
                if *product == RECEIVER {
                    self.unanswered.extend(hardware(path));
                }
                if named.is_some() && self.last.get(path).is_some_and(|last| last.key != device.key) {
                    self.last.remove(path);
                }
                kept.push(self.last.get(path).map_or(Device { charging: false, ..device }, |last| Device { charging: false, ..last.clone() }));
            }
            self.last.retain(|path, _| answers.iter().any(|(there, ..)| there == path));
            // (A board read on its cable, and kept as last read on its receiver: as read.)
            kept.retain(|device: &Device| !read.iter().any(|fresh: &Device| fresh.key == device.key));
            read.extend(kept);
            read
        }
    }

    /// What a receiver or cable plugged in answered: its path, its product, and the board it named and that board's
    /// charge (see `battery`).
    pub(super) type Answered = (String, u16, (Option<u16>, Option<u8>));

    /// Asked of `channel` in a session of its own: the board the receiver says is paired to it (none on a cable, or
    /// where it did not say), and the board's battery (percent; none where it did not answer).
    fn battery(channel: &mut hid::Collection) -> (Option<u16>, Option<u8>) {
        // (The length a session is opened and closed with: the report's less the frame's head, as PMO Hub's.)
        let length = channel.output.saturating_sub(9) as u8;
        // (Asked twice: the first may wake the link.)
        let started = ask(channel, START, length).or_else(|| ask(channel, START, length));
        let Some(started) = started else {
            // (Closed all the same, in case it was opened.)
            channel.write(&frame(channel.output, STOP, length), WAITED);
            return (None, None);
        };
        let level = done(&started).then(|| ask(channel, BATTERY, BATTERY_LENGTH)).flatten().and_then(|reply| level(&reply));
        ask(channel, STOP, length);
        (paired(&started), level)
    }

    /// Asks `command` of `channel` (with `length`): its reply's payload (the report id taken off), where it came in time.
    fn ask(channel: &mut hid::Collection, command: u8, length: u8) -> Option<Vec<u8>> {
        channel.read();
        if !channel.write(&frame(channel.output, command, length), WAITED) {
            return None;
        }
        let until = Instant::now() + WAITED;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            if let Some(reply) = channel.next(left).and_then(|report| answer(&report, command)) {
                return Some(reply);
            }
        }
    }

    /// The report asking `command` with `length`, `output` long (its report id, 0, first).
    pub(super) fn frame(output: usize, command: u8, length: u8) -> Vec<u8> {
        let mut report = vec![0; output.max(6)];
        report[1..6].copy_from_slice(&[FRAME, command, 0, 0, length]);
        report
    }

    /// The payload of `report` (its report id taken off), where it answers `command`: it begins as the question did.
    pub(super) fn answer(report: &[u8], command: u8) -> Option<Vec<u8>> {
        let payload = report.get(1..)?;
        payload.starts_with(&[FRAME, command, 0, 0]).then(|| payload.to_vec())
    }

    /// Whether a reply says it did what was asked.
    pub(super) fn done(reply: &[u8]) -> bool {
        reply.get(7) == Some(&DONE)
    }

    /// The board a session's opening says is paired to the receiver (its vendor's and product's ids, low byte first).
    pub(super) fn paired(started: &[u8]) -> Option<u16> {
        let id = |at: usize| Some(u16::from_le_bytes([*started.get(at)?, *started.get(at + 1)?]));
        (id(9)? == VENDOR).then(|| id(11)).flatten()
    }

    /// The battery's level a reply to the battery's question tells, where it did what was asked.
    pub(super) fn level(reply: &[u8]) -> Option<u8> {
        done(reply).then(|| reply.get(8).copied()).flatten().filter(|level| *level <= 100)
    }
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
    /// HID++ 1.0's long register read, and the receiver's register of what is paired in its slots: a slot's pairing
    /// asked by its sub-register, a Bolt receiver's from 0x50 and the slot's number, a Unifying or Lightspeed one's from
    /// 0x20 and the slot's number less one (refused for a slot that holds none).
    const GET_LONG_REGISTER: u8 = 0x83;
    const RECEIVER_INFO: u8 = 0xB5;
    const BOLT_PAIRING: u8 = 0x50;
    const UNIFYING_PAIRING: u8 = 0x20;
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

    /// What a receiver's pairing register tells of the device paired in a slot.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Paired {
        pub kind: Kind,
        pub wpid: u16,
    }

    impl Battery {
        /// The feature it is read through, and that feature's index on the device.
        fn feature(self) -> (u16, u8) {
            match self {
                Battery::Unified(index) => (UNIFIED_BATTERY, index),
                Battery::Status(index) => (BATTERY_STATUS, index),
                Battery::Voltage(index) => (BATTERY_VOLTAGE, index),
            }
        }
    }

    /// A device as last read, kept from one run of Glance to the next: by its receiver and slot, with the wireless id
    /// its receiver's pairing register told in that slot (none where it told none; none in what an older Glance kept).
    #[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
    pub(super) struct KeptDevice {
        pub receiver: String,
        pub slot: u8,
        pub device: Device,
        #[serde(default)]
        pub wpid: Option<u16>,
    }

    /// Reads the receivers' devices, keeping what is asked once of each (by its receiver and slot), and each as it was
    /// last read (see `asleep`).
    #[derive(Default)]
    pub(super) struct Reader {
        known: HashMap<(String, u8), Known>,
        last: HashMap<(String, u8), Device>,
        /// The wireless id each slot's pairing register last told (kept with what was last read: see `KeptDevice`):
        /// another there since, another device was paired in that slot.
        wpids: HashMap<(String, u8), u16>,
        /// The receivers (by `hardware`) a keyboard on which went unanswered at the last reading.
        unanswered: Vec<String>,
        /// The software id the next question carries (2 to 15, in turn: 0 is the devices' own notifications', 1 is
        /// left to others), and the byte the next ping carries, which its answer gives back.
        software: u8,
        mark: u8,
    }

    impl Reader {
        /// A reader that has read `last` (see `kept`).
        pub fn seeded(last: &[KeptDevice]) -> Reader {
            Reader {
                last: last.iter().map(|kept| ((kept.receiver.clone(), kept.slot), kept.device.clone())).collect(),
                wpids: last.iter().filter_map(|kept| Some(((kept.receiver.clone(), kept.slot), kept.wpid?))).collect(),
                ..Reader::default()
            }
        }

        /// The devices as last read, by their receiver and slot, in order, with the wireless id told in their slot.
        pub fn kept(&self) -> Vec<KeptDevice> {
            let mut kept: Vec<KeptDevice> = self.last.iter().map(|(key, device)| KeptDevice { receiver: key.0.clone(), slot: key.1, device: device.clone(), wpid: self.wpids.get(key).copied() }).collect();
            kept.sort_by(|a, b| (&a.receiver, a.slot).cmp(&(&b.receiver, b.slot)));
            kept
        }

        /// The wireless id the pairing register of the slot `key` tells now, where it tells one: another than told
        /// there before (in this run, or kept from the last), nothing known or kept of the device before is the one
        /// there now (it was shown as that one, with its charge, and told of as it ran low).
        pub(super) fn paired_now(&mut self, key: &(String, u8), wpid: Option<u16>) {
            let Some(wpid) = wpid else { return };
            if self.wpids.insert(key.clone(), wpid).is_some_and(|before| before != wpid) {
                self.known.remove(key);
                self.last.remove(key);
            }
        }

        /// The receivers (by `hardware`) a keyboard on which went unanswered at the last reading (asked again as soon as
        /// a key is pressed on it).
        pub fn waiting(&self) -> Vec<String> {
            self.unanswered.clone()
        }

        /// Each device joined to a receiver: as read now, else as last read, else unread (see `unanswered`).
        pub fn read(&mut self) -> Vec<Device> {
            let mut devices = Vec::new();
            let mut present = Vec::new();
            self.unanswered.clear();
            for mut receiver in receivers() {
                present.push(receiver.id.clone());
                if !matches!(receiver.ask(RECEIVER, GET_REGISTER, CONNECTION_STATE, [0; 3]), Answer::Reply(_) | Answer::Refused(_)) {
                    continue;
                }
                for slot in SLOTS {
                    let key = (receiver.id.clone(), slot);
                    let paired = paired(&mut receiver, slot);
                    self.paired_now(&key, paired.map(|paired| paired.wpid));
                    let answers = self.ping(&mut receiver, slot);
                    if let Some(device) = answers.then(|| self.reading(&mut receiver, slot)).flatten() {
                        self.last.insert(key, device.clone());
                        devices.push(device);
                        continue;
                    }
                    let known = self.known.get(&key);
                    let unread = || Device {
                        key: format!("hidpp:{}:{slot}", receiver.id),
                        name: known.map(|known| known.name.clone()).unwrap_or_default(),
                        kind: known.map(|known| known.kind).or(paired.map(|paired| paired.kind)).unwrap_or(Kind::Other),
                        via: Via::Logitech,
                        level: None,
                        charging: false,
                        container: None,
                    };
                    match unanswered(self.last.get(&key), answers || paired.is_some(), unread) {
                        Some(device) => {
                            if !answers && device.kind == Kind::Keyboard {
                                self.unanswered.extend(hardware(&receiver.id));
                            }
                            devices.push(device);
                        }
                        None => {
                            self.known.remove(&key);
                            self.last.remove(&key);
                            self.wpids.remove(&key);
                        }
                    }
                }
            }
            self.unanswered.dedup();
            self.known.retain(|(receiver, _), _| present.contains(receiver));
            self.last.retain(|(receiver, _), _| present.contains(receiver));
            self.wpids.retain(|(receiver, _), _| present.contains(receiver));
            devices
        }

        /// The device in `slot` (it answered a ping), read: asked what it is first where it was not yet. None where it
        /// did not tell its charge, or has no battery to tell.
        fn reading(&mut self, receiver: &mut Receiver, slot: u8) -> Option<Device> {
            let key = (receiver.id.clone(), slot);
            if !self.known.contains_key(&key) {
                let known = self.discover(receiver, slot)?;
                self.known.insert(key.clone(), known);
            }
            let known = self.known[&key].clone();
            let Some((level, charging)) = self.battery(receiver, slot, known.battery?) else {
                // (Asked anew next time: where it was another device's battery feature, it is not asked again.)
                self.known.remove(&key);
                return None;
            };
            Some(Device { key: format!("hidpp:{}:{slot}", receiver.id), name: known.name, kind: known.kind, via: Via::Logitech, level: Some(level), charging, container: None })
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
                // (A device that cannot say its name is shown as what its receiver says it is, unread.)
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

        /// The battery's level (percent) and whether it is charging, of the device in `slot`: asked only where its
        /// battery's feature is at the index it was (a function asked of another feature could set, not tell).
        fn battery(&mut self, receiver: &mut Receiver, slot: u8, battery: Battery) -> Option<(u8, bool)> {
            let (feature, index) = battery.feature();
            if self.index(receiver, slot, feature)? != Some(index) {
                return None;
            }
            match battery {
                // (Function 1: its status. Function 0 tells what it can say.)
                Battery::Unified(index) => unified_battery(&self.feature(receiver, slot, index, 1, [0; 3])?),
                Battery::Status(index) => battery_status(&self.feature(receiver, slot, index, 0, [0; 3])?),
                Battery::Voltage(index) => battery_voltage(&self.feature(receiver, slot, index, 0, [0; 3])?),
            }
        }
    }

    /// A device that gives no reading now (asleep, a keyboard or a mouse left alone; switched off; or its charge not
    /// told), where it is `there` (it answers, or its receiver still holds it paired): as it was `last` read, not
    /// charging (nothing says it is); else, never read, `unread`. Not there any more, it is not shown.
    pub(super) fn unanswered(last: Option<&Device>, there: bool, unread: impl FnOnce() -> Device) -> Option<Device> {
        there.then(|| last.map_or_else(unread, |last| Device { charging: false, ..last.clone() }))
    }

    /// What `receiver`'s pairing register says of the device paired in `slot` (asked as a Bolt receiver, and as a
    /// Unifying or Lightspeed one): none where it holds none (or does not tell).
    fn paired(receiver: &mut Receiver, slot: u8) -> Option<Paired> {
        [BOLT_PAIRING + slot, UNIFYING_PAIRING + slot - 1].into_iter().find_map(|sub| pairing(&receiver.ask(RECEIVER, GET_LONG_REGISTER, RECEIVER_INFO, [sub, 0, 0]), sub))
    }

    /// What `answer` to the pairing register's sub-register `sub` tells of a device paired there: its reply begins
    /// with that sub-register. A Bolt receiver's (from 0x50) tells what the device is in its second byte (the low four
    /// bits) and its wireless id in the third and fourth (low first); a Unifying receiver's, its wireless id in the
    /// fourth and fifth (high first), what it is in the eighth, as Solaar reads them.
    pub(super) fn pairing(answer: &Answer, sub: u8) -> Option<Paired> {
        let Answer::Reply(reply) = answer else { return None };
        if reply.first() != Some(&sub) {
            return None;
        }
        let (kind, wpid) = if sub >= BOLT_PAIRING {
            (*reply.get(1)?, u16::from_le_bytes([*reply.get(2)?, *reply.get(3)?]))
        } else {
            (*reply.get(7)?, u16::from_be_bytes([*reply.get(3)?, *reply.get(4)?]))
        };
        Some(Paired { kind: paired_kind(kind & 0x0F), wpid })
    }

    /// What a receiver's pairing register says a device is (HID++ 1.0's kinds).
    pub(super) fn paired_kind(kind: u8) -> Kind {
        match kind {
            // A keyboard, a numpad.
            1 | 3 => Kind::Keyboard,
            // A mouse, a trackball, a touchpad.
            2 | 8 | 9 => Kind::Mouse,
            // A gamepad, a joystick.
            11 | 12 => Kind::Gamepad,
            13 => Kind::Headset,
            _ => Kind::Other,
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

    /// A receiver: its USB interface's instance identifier, and its HID++ collections (short, long, very long:
    /// questions are written to the short one, or the long one where it has none; answers come on any).
    struct Receiver {
        id: String,
        collections: Vec<hid::Collection>,
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
            if !self.collections[writer].write(&request, WAITED) {
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

    }

    /// Logitech's HID++ collections there are now (see `opened`), opened, by the USB interface they are of: each such interface's, as a receiver that may be (it is one if it answers as one).
    fn receivers() -> Vec<Receiver> {
        let mut by_interface: Vec<(String, Vec<hid::Collection>)> = Vec::new();
        for path in hid::paths("046d") {
            let Some(parent) = parent_of(&path) else { continue };
            // (HID++'s: vendor-defined (usage page 0xFF00), of its short reports (usage 1) or its long ones (usage 2).)
            let Some(collection) = hid::Collection::open(&path, |caps| caps.UsagePage == 0xFF00 && matches!(caps.Usage, 1 | 2) && caps.InputReportByteLength >= 7) else { continue };
            match by_interface.iter_mut().find(|(id, _)| *id == parent) {
                Some((_, collections)) => collections.push(collection),
                None => by_interface.push((parent, vec![collection])),
            }
        }
        by_interface.into_iter().map(|(id, collections)| Receiver { id, collections }).collect()
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
    pub(super) fn hear(told: Sender<Wake>) -> Option<Heard> {
        let gone = told.clone();
        let added = Gamepad::GamepadAdded(&EventHandler::new(move |_, _| {
            let _ = told.send(Wake::Told);
            Ok(())
        }))
        .ok()?;
        let removed = Gamepad::GamepadRemoved(&EventHandler::new(move |_, _| {
            let _ = gone.send(Wake::Told);
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
            level: Some(level),
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
    use super::logitech::{answer, battery_status, battery_voltage, device_kind, paired_kind, pairing, unanswered, unified_battery, voltage_percent, Answer, Paired};
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
        // The Sony WH-1000XM5's: audio/video, a wearable headset, drawn as the headphones it is.
        assert_eq!(class_kind(0x240404), Kind::Headphones);
        assert_eq!(class_kind(0x240408), Kind::Headset);
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
        Device { key: key.into(), name: name.into(), kind, via: Via::Bluetooth, level: Some(50), charging: false, container: None }
    }

    /// A receiver's device that gives no reading now (asleep, switched off, or its charge not told) is shown while the
    /// receiver holds it: as last read (not charging), else unread, as what is known of it; not where it is gone.
    #[test]
    fn a_device_on_its_receiver_is_shown_read_or_not() {
        let keys = Device { via: Via::Logitech, level: Some(64), charging: true, ..device("hidpp:r:1", "MX Keys", Kind::Keyboard) };
        let unread = || Device { via: Via::Logitech, level: None, ..device("hidpp:r:1", "", Kind::Mouse) };
        assert_eq!(unanswered(Some(&keys), true, unread), Some(Device { charging: false, ..keys.clone() }));
        assert_eq!(unanswered(Some(&keys), false, unread), None);
        assert_eq!(unanswered(None, true, unread), Some(unread()));
        assert_eq!(unanswered(None, false, unread), None);
        // The pairing register: a slot's reply begins with its sub-register, an empty slot's is refused. A Bolt
        // receiver's tells what the device is in its second byte and its wireless id after (the owner's: its mouse in
        // slot 2); a Unifying receiver's, its wireless id in the fourth and fifth (high first), what it is in the eighth.
        let bolt = vec![0x52, 0x02, 0x42, 0xB0, 0xA7, 0x55, 0x03, 0x0D, 0x02, 0x01, 0x80, 0x14, 0x01, 0, 0, 0];
        assert_eq!(pairing(&Answer::Reply(bolt), 0x52), Some(Paired { kind: Kind::Mouse, wpid: 0xB042 }));
        let unifying = vec![0x21, 0x00, 0x08, 0x40, 0x2D, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(pairing(&Answer::Reply(unifying), 0x21), Some(Paired { kind: Kind::Keyboard, wpid: 0x402D }));
        assert_eq!(pairing(&Answer::Refused(0x08), 0x51), None);
        assert_eq!(pairing(&Answer::Reply(vec![0x53, 0x02, 0, 0]), 0x52), None);
        assert_eq!(pairing(&Answer::Silent, 0x52), None);
        assert_eq!([0x01, 0x02, 0x03, 0x08, 0x09, 0x0B, 0x0D, 0x00].map(paired_kind), [Kind::Keyboard, Kind::Mouse, Kind::Keyboard, Kind::Mouse, Kind::Mouse, Kind::Gamepad, Kind::Headset, Kind::Other]);
    }

    /// A Logitech receiver's slot paired to another device since its device was last read, in this run or before Glance
    /// last ended: nothing kept of the one before is shown for it (it was, by its name, kind and charge, and told of as
    /// it ran low). The wireless id kept with what was read; what an older Glance kept, without it, still read.
    #[test]
    fn a_slot_paired_anew_forgets_the_device_before() {
        use super::logitech::{KeptDevice, Reader};
        let keys = Device { via: Via::Logitech, level: Some(8), ..device("hidpp:r:1", "MX Keys", Kind::Keyboard) };
        let kept = [KeptDevice { receiver: "r".into(), slot: 1, device: keys.clone(), wpid: Some(0x408A) }];
        // (A later run, the slot's pairing told first: the same device, kept; another paired while Glance was not
        // running, forgotten; and in the same run, as it is paired anew.)
        let mut later = Reader::seeded(&kept);
        later.paired_now(&("r".into(), 1), None);
        later.paired_now(&("r".into(), 1), Some(0x408A));
        assert_eq!(later.kept(), kept);
        later.paired_now(&("r".into(), 1), Some(0xB042));
        assert!(later.kept().is_empty());
        let mut other = Reader::seeded(&kept);
        other.paired_now(&("r".into(), 1), Some(0xB042));
        assert!(other.kept().is_empty());
        // (What an older Glance kept, a list of three: read, with no wireless id.)
        let old = format!("[[\"r\",1,{}]]", serde_json::to_string(&keys).unwrap());
        let read: Vec<KeptDevice> = serde_json::from_str(&old).unwrap();
        assert_eq!(read, [KeptDevice { wpid: None, ..kept[0].clone() }]);
        assert_eq!(serde_json::from_str::<Vec<KeptDevice>>(&serde_json::to_string(&kept).unwrap()).unwrap(), kept);
    }

    /// A PMO board is shown while its receiver (or its cable) is plugged in: as read; as last read where it does not
    /// answer (read before Glance last ended too); unread where it never answered, as the board its receiver names, or
    /// a keyboard. Read on its cable, it is as read on its receiver too, and charging there.
    #[test]
    fn a_pmo_board_is_shown_while_its_receiver_is_in() {
        use super::pmo::Reader;
        let receiver = r"\\?\hid#vid_36b0&pid_3002&mi_01#a&11e21a08&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}".to_string();
        let cable = r"\\?\hid#vid_36b0&pid_3009&mi_01#a&22e21a08&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}".to_string();
        let mut reader = Reader::default();
        let shown = reader.took(vec![(receiver.clone(), 0x3002, (None, None))]);
        assert_eq!(shown, [Device { key: "pmo:3002".into(), name: String::new(), kind: Kind::Keyboard, via: Via::Receiver, level: None, charging: false, container: None }]);
        assert_eq!(reader.waiting(), ["vid_36b0&pid_3002"]);
        let shown = reader.took(vec![(receiver.clone(), 0x3002, (Some(0x3009), None))]);
        assert_eq!((shown[0].key.as_str(), shown[0].name.as_str(), shown[0].level), ("pmo:3009", "WAVE 75", None));
        let shown = reader.took(vec![(receiver.clone(), 0x3002, (Some(0x3009), Some(6)))]);
        assert_eq!((shown[0].key.as_str(), shown[0].level, shown[0].charging), ("pmo:3009", Some(6), false));
        assert!(reader.waiting().is_empty());
        // A later run, the board asleep: as last read.
        let mut later = Reader::seeded(&reader.kept());
        let shown = later.took(vec![(receiver.clone(), 0x3002, (None, None))]);
        assert_eq!((shown[0].key.as_str(), shown[0].level), ("pmo:3009", Some(6)));
        // On its cable at 80: once, as read there; its cable out, as read there through its receiver too.
        let shown = later.took(vec![(receiver.clone(), 0x3002, (None, None)), (cable.clone(), 0x3009, (None, Some(80)))]);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].via, shown[0].level, shown[0].charging), (Via::Cable, Some(80), true));
        let shown = later.took(vec![(receiver.clone(), 0x3002, (None, None))]);
        assert_eq!((shown[0].via, shown[0].level, shown[0].charging), (Via::Receiver, Some(80), false));
        // Another board paired to its receiver since, not answering: that board, unread (not the one read before,
        // with its charge), and still so as the receiver names none.
        let shown = later.took(vec![(receiver.clone(), 0x3002, (Some(0x3004), None))]);
        assert_eq!((shown[0].key.as_str(), shown[0].name.as_str(), shown[0].level), ("pmo:3004", "Aurora 80", None));
        let shown = later.took(vec![(receiver.clone(), 0x3002, (None, None))]);
        assert_eq!(shown[0].level, None);
        // Its receiver out: gone.
        assert!(later.took(Vec::new()).is_empty());
        assert!(later.kept().is_empty());
    }

    /// What Windows is asked over the air is waited for as long as it is meant to be, no longer: told in time, what it
    /// gives; not told in time, none, at once (the other devices are read after it).
    #[test]
    fn an_answer_is_waited_for_no_longer_than_meant() {
        let quick = windows_future::IAsyncOperation::<i32>::spawn(|| Ok(7));
        assert_eq!(completed(quick, Duration::from_secs(2)), Some(7));
        let slow = windows_future::IAsyncOperation::<i32>::spawn(|| {
            std::thread::sleep(Duration::from_secs(3));
            Ok(7)
        });
        let since = Instant::now();
        assert_eq!(completed(slow, Duration::from_millis(200)), None);
        assert!(since.elapsed() < Duration::from_secs(1), "{:?}", since.elapsed());
    }

    /// A keyboard heard is matched to the receivers waited for by the vendor and product its device's name carries.
    #[test]
    fn a_keyboard_heard_is_matched_to_its_receiver() {
        assert_eq!(hardware(r"\\?\HID#VID_36B0&PID_3002&MI_00#8&2b3c51a&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}").as_deref(), Some("vid_36b0&pid_3002"));
        assert_eq!(hardware(r"USB\VID_046D&PID_C548&MI_02\C&34ECC405&0&0002").as_deref(), Some("vid_046d&pid_c548"));
        assert_eq!(hardware(r"\\?\ACPI#PNP0303#4&1d401fb5&0#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}"), None);
        let waiting = ["vid_36b0&pid_3002".to_string()];
        assert!(waited_for(r"\\?\HID#VID_36B0&PID_3002&MI_00#8&2b3c51a&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}", &waiting));
        assert!(!waited_for(r"\\?\HID#VID_046D&PID_C548&MI_00#8&1&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}", &waiting));
        assert!(!waited_for(r"\\?\HID#VID_36B0&PID_3002&MI_00#8&2b3c51a&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}", &[]));
    }

    /// PMO Hub's frames, and its replies as the owner's Wave 75 gave them through its receiver: the session's opening
    /// naming the board paired (36B0:3009), the battery at 6 %, the session closed. A reply to another question, one
    /// that failed (0x0F) and a level past 100 tell nothing.
    #[test]
    fn a_pmo_board_is_read_through_its_receiver() {
        use super::pmo::{answer, done, frame, level, paired};
        let reply = |bytes: &[u8]| -> Vec<u8> { [&[0u8][..], bytes, &[0u8; 32][..32 - bytes.len()]].concat() };
        assert_eq!(frame(33, 0x10, 0x18)[..7], [0x00, 0xAA, 0x10, 0x00, 0x00, 0x18, 0x00]);
        assert_eq!(frame(33, 0x10, 0x18).len(), 33);
        let started = answer(&reply(&[0xAA, 0x10, 0x00, 0x00, 0x18, 0x00, 0x00, 0x55, 0x00, 0xB0, 0x36, 0x09, 0x30]), 0x10).unwrap();
        assert!(done(&started));
        assert_eq!(paired(&started), Some(0x3009));
        let battery = answer(&reply(&[0xAA, 0x30, 0x00, 0x00, 0x18, 0x00, 0x00, 0x55, 0x06]), 0x30).unwrap();
        assert_eq!(level(&battery), Some(6));
        assert!(done(&answer(&reply(&[0xAA, 0x11, 0x00, 0x00, 0x18, 0x00, 0x00, 0x55]), 0x11).unwrap()));
        assert_eq!(answer(&reply(&[0xAA, 0x30, 0x00, 0x00, 0x18, 0x00, 0x00, 0x55, 0x06]), 0x10), None);
        assert_eq!(level(&answer(&reply(&[0xAA, 0x30, 0x00, 0x00, 0x18, 0x00, 0x00, 0x0F, 0x06]), 0x30).unwrap()), None);
        assert_eq!(level(&answer(&reply(&[0xAA, 0x30, 0x00, 0x00, 0x18, 0x00, 0x00, 0x55, 0xC8]), 0x30).unwrap()), None);
        // On its cable, no board is named.
        assert_eq!(paired(&answer(&reply(&[0xAA, 0x10, 0x00, 0x00, 0x38, 0x00, 0x00, 0x55]), 0x10).unwrap()), None);
        assert_eq!(super::hid::product(r"\\?\hid#vid_36b0&pid_3002&mi_01#a&11e21a08&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}"), Some(0x3002));
    }

    #[test]
    fn devices_are_ordered_by_kind_then_name() {
        let ordered = ordered(vec![
            device("c", "mx master", Kind::Mouse),
            device("b", "WH-1000XM5", Kind::Headphones),
            device("a", "MX Keys", Kind::Keyboard),
            device("d", "Buds", Kind::Earbuds),
            device("e", "Anker", Kind::Mouse),
        ]);
        let keys: Vec<&str> = ordered.iter().map(|device| device.key.as_str()).collect();
        assert_eq!(keys, ["b", "d", "a", "e", "c"]);
    }

    /// An asking (the panel opened) has the devices read at once, also while Windows is still telling of devices; a
    /// telling alone waits for Windows to go quiet; no one left to wake the reader ends it.
    #[test]
    fn an_asking_is_read_at_once() {
        let (wake, woken) = channel();
        wake.send(Wake::Asked).unwrap();
        let since = Instant::now();
        assert_eq!(settled(&woken), Some(true));
        assert!(since.elapsed() < SETTLED / 4, "{:?}", since.elapsed());
        wake.send(Wake::Told).unwrap();
        wake.send(Wake::Asked).unwrap();
        let since = Instant::now();
        assert_eq!(settled(&woken), Some(true));
        assert!(since.elapsed() < SETTLED / 4, "{:?}", since.elapsed());
        wake.send(Wake::Told).unwrap();
        let since = Instant::now();
        assert_eq!(settled(&woken), Some(true));
        assert!(since.elapsed() >= SETTLED);
        // Asked several times at once (the panel and widgets opening together): read once.
        for _ in 0..3 {
            wake.send(Wake::Asked).unwrap();
        }
        wake.send(Wake::Told).unwrap();
        assert_eq!(settled(&woken), Some(true));
        assert!(woken.try_recv().is_err());
        drop(wake);
        assert_eq!(settled(&woken), None);
        // Told to leave (the devices no longer wanted): ended at once, whether waiting, settling, or asked with it.
        let (wake, woken) = channel();
        for wakes in [vec![Wake::Leave], vec![Wake::Told, Wake::Leave], vec![Wake::Asked, Wake::Leave]] {
            for one in wakes {
                wake.send(one).unwrap();
            }
            let since = Instant::now();
            assert_eq!(settled(&woken), None);
            assert!(since.elapsed() < SETTLED / 4, "{:?}", since.elapsed());
        }
    }

    /// The properties the Bluetooth watcher is asked to tell of, named as Windows' device queries name them.
    #[test]
    fn bluetooth_properties_are_named_as_windows_names_them() {
        use super::bluetooth::property_name;
        let connected = DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };
        assert_eq!(property_name(&connected), "{83DA6326-97A6-4088-9453-A1923F573B29} 15");
    }

    /// The watcher of Bluetooth's device nodes starts with its filter and properties, lists the nodes there are, and
    /// tells each one's being joined and its level (printed: run with a Bluetooth device paired).
    #[test]
    #[ignore = "watches this machine's Bluetooth devices"]
    fn watches_the_bluetooth_devices() {
        let watcher = super::bluetooth::watcher().unwrap();
        let (seen, saw) = channel();
        watcher
            .Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, information| {
                let information = information.as_ref().unwrap();
                let properties = information.Properties()?;
                let told = |name: &str| properties.Lookup(&HSTRING::from(name)).map(|value| format!("{value:?}")).unwrap_or_default();
                let _ = seen.send(format!(
                    "{} | joined {} | level {}",
                    information.Name()?,
                    told("{83DA6326-97A6-4088-9453-A1923F573B29} 15"),
                    told("{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2")
                ));
                Ok(())
            }))
            .unwrap();
        watcher.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(|_, _| Ok(()))).unwrap();
        watcher.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(|_, _| Ok(()))).unwrap();
        watcher.Start().unwrap();
        std::thread::sleep(Duration::from_secs(3));
        let listed: Vec<String> = saw.try_iter().collect();
        for node in &listed {
            println!("{node}");
        }
        let _ = watcher.Stop();
        assert!(!listed.is_empty());
    }

    #[test]
    #[ignore = "reads this machine's wireless devices"]
    fn reads_the_devices() {
        want(true);
        // (Controllers are listed only after Windows has told of them.)
        std::thread::sleep(Duration::from_secs(8));
        for device in now() {
            println!("{device:?}");
        }
        // No longer wanted: the reader ends, nothing shown or waited for; wanted again, it reads again.
        want(false);
        let since = Instant::now();
        while RUNNING.load(Ordering::Acquire) && since.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("ended in {:?}", since.elapsed());
        assert!(!RUNNING.load(Ordering::Acquire) && now().is_empty() && !hears_keys());
        want(true);
        std::thread::sleep(Duration::from_secs(8));
        println!("again: {} devices", now().len());
        assert!(RUNNING.load(Ordering::Acquire));
    }
}
