//! Where the PC's sound goes, and how loud each program is there: the outputs it can play on (the one it plays on now
//! among them, another chosen for it), and of that one, each program that plays on it or did lately, how loud it is
//! and whether it is muted, set for it alone, and how loud it sounds this moment.
//!
//! The output played on now is watched as long as Glance runs (the panel's volume row names it: see `output`). The
//! rest is read only while something shows it (see `mixer`), on a thread of its own that reads it again and again and
//! sets what is asked, in the order asked, and stops a little after it is no longer shown. Nothing of another program
//! is opened: what plays is known by its sound's session, which says the program's file (see `media::named`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, Result, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionStateActive, AudioSessionStateExpired, EDataFlow, ERole, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PropVariantToUInt32};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

/// What an output is, as its driver says (Windows' form factor of the endpoint): what it is drawn as.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Form {
    Speakers,
    Headphones,
    Headset,
    /// A phone's handset (a hands-free call's).
    Handset,
    /// A screen's own sound (over HDMI or DisplayPort).
    Screen,
    /// A line out, or a digital one (S/PDIF): to another box that plays it.
    Line,
}

/// How an output is joined to the PC: what is said under its name.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Via {
    Bluetooth,
    Usb,
    /// On the PC's own sound (its board's, its graphics card's), by that one's name ("Realtek(R) Audio").
    Card(String),
}

/// An output the sound may play on: Windows' id of it, its name and how it is joined, what it is.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Output {
    pub id: String,
    pub name: String,
    pub via: Via,
    pub form: Form,
}

/// A program as it plays on the output played on now (several sounds of one program, as a browser's, together): what
/// it is known by (its file, as its sound says it), its name, its file on its drive (for its picture), how loud it is
/// there (0 to 100, of the output's own level) and whether it is muted, whether it sounds now (else it did lately,
/// and has its sound still open), and whether it is Windows' own sounds.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct App {
    pub key: String,
    pub name: String,
    pub file: Option<PathBuf>,
    pub level: u8,
    pub muted: bool,
    pub sounding: bool,
    pub system: bool,
}

/// What `mixer` reads: the outputs, the one played on now (its id), the programs on it.
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct Mixer {
    pub outputs: Vec<Output>,
    pub current: Option<String>,
    pub apps: Vec<App>,
    /// Read yet (else nothing of it is known, not that there is nothing).
    pub read: bool,
}

/// Windows' own sounds' key among the programs.
pub const SYSTEM: &str = "system";

/// The output played on now, as last told (none before it is first read, or where there is none). Watched from its
/// first asking on (see `watch_output`).
pub fn output() -> Option<Output> {
    watch_output();
    CURRENT.lock().unwrap().clone()
}

static CURRENT: Mutex<Option<Output>> = Mutex::new(None);

/// The output played on now read, and read again each time Windows tells of another (or of a change to one: its
/// name), on a thread that waits on that for as long as Glance runs.
fn watch_output() {
    static WATCHING: OnceLock<()> = OnceLock::new();
    WATCHING.get_or_init(|| {
        std::thread::Builder::new()
            .name("glance: sound output".into())
            .spawn(|| {
                crate::actions::com();
                let (told, telling) = std::sync::mpsc::channel();
                let watch: IMMNotificationClient = Watch(Mutex::new(told)).into();
                let Ok(devices) = enumerator() else { return };
                let _ = unsafe { devices.RegisterEndpointNotificationCallback(&watch) };
                loop {
                    let now = unsafe { devices.GetDefaultAudioEndpoint(eRender, eConsole) }.ok().and_then(|device| described(&device));
                    *CURRENT.lock().unwrap() = now;
                    // (Told several times at once, as an output comes: read once after.)
                    if telling.recv().is_err() {
                        return;
                    }
                    while telling.recv_timeout(Duration::from_millis(50)).is_ok() {}
                }
            })
            .expect("a thread started");
    });
}

/// Told by Windows of a change to the outputs (another played on, one come, gone, renamed): sent on (only that; what
/// changed is read after, not in Windows' call, which must not wait on it).
#[implement(IMMNotificationClient)]
struct Watch(Mutex<Sender<()>>);

impl IMMNotificationClient_Impl for Watch_Impl {
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.0.lock().unwrap().send(());
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        let _ = self.0.lock().unwrap().send(());
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, key: &PROPERTYKEY) -> Result<()> {
        if [DEVICE_DESCRIPTION, INTERFACE_NAME].contains(key) {
            let _ = self.0.lock().unwrap().send(());
        }
        Ok(())
    }
}

/// The outputs and the programs on the one played on now, as last read; read on (see the top) while it is asked for,
/// a moment after it last was. Where something was set since the last reading, that shows as it was set.
pub fn mixer() -> Mixer {
    asked();
    MIXER.lock().unwrap().clone()
}

/// How loud each program sounds this moment (0 to 1, by its key), as last read with `mixer`.
pub fn peaks() -> HashMap<String, f32> {
    asked();
    PEAKS.lock().unwrap().clone().unwrap_or_default()
}

/// The sound played on `output` (its id) from now on: for every use (programs' sounds, games', calls'), as Windows'
/// own choosing of it does. Shown so at once.
pub fn choose(output: &str) {
    {
        let mut mixer = MIXER.lock().unwrap();
        if mixer.current.as_deref() == Some(output) {
            return;
        }
        mixer.current = Some(output.to_string());
        // (The programs are another output's now: read again for it.)
        mixer.apps.clear();
        mixer.read = false;
    }
    set(Set::Output(output.to_string()));
}

/// The program `key` (see `App::key`) made as loud as `level` and muted or not as `muted` (each, where it is said), on
/// the output played on now. Shown so at once.
pub fn set_app(key: &str, level: Option<u8>, muted: Option<bool>) {
    {
        let mut mixer = MIXER.lock().unwrap();
        let Some(app) = mixer.apps.iter_mut().find(|app| app.key == key) else { return };
        if level.is_none_or(|level| level == app.level) && muted.is_none_or(|muted| muted == app.muted) {
            return;
        }
        if let Some(level) = level {
            app.level = level.min(100);
        }
        if let Some(muted) = muted {
            app.muted = muted;
        }
    }
    set(Set::App(key.to_string(), level, muted));
}

static MIXER: Mutex<Mixer> = Mutex::new(Mixer { outputs: Vec::new(), current: None, apps: Vec::new(), read: false });
static PEAKS: Mutex<Option<HashMap<String, f32>>> = Mutex::new(None);

/// What is asked to be set, on the thread that reads and sets (see the top).
enum Set {
    Output(String),
    App(String, Option<u8>, Option<bool>),
}

/// How many things were asked to be set: a reading begun before the latest was asked is not shown (it was read before
/// that was set, and would show it as it was).
static ASKED_TO_SET: AtomicU64 = AtomicU64::new(0);

fn set(set: Set) {
    let _ = sets().0.lock().unwrap().send(set);
    ASKED_TO_SET.fetch_add(1, Ordering::AcqRel);
    asked();
}

/// What is asked to be set, sent to the thread that sets it (which holds the receiving end while it runs).
fn sets() -> &'static (Mutex<Sender<Set>>, Mutex<Receiver<Set>>) {
    static SETS: OnceLock<(Mutex<Sender<Set>>, Mutex<Receiver<Set>>)> = OnceLock::new();
    SETS.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        (Mutex::new(sender), Mutex::new(receiver))
    })
}

/// When it was last asked for, and whether its thread runs; how long it runs on after.
static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
const KEPT_ON: Duration = Duration::from_secs(2);
/// How often the outputs and programs are read again, and how often how loud each sounds.
const READ_EVERY: Duration = Duration::from_millis(500);
const PEAKS_EVERY: Duration = Duration::from_millis(33);

fn asked() {
    *ASKED.lock().unwrap() = Some(Instant::now());
    if !RUNNING.swap(true, Ordering::AcqRel) {
        std::thread::Builder::new()
            .name("glance: sound mixer".into())
            .spawn(|| {
                run(&sets().1.lock().unwrap());
                RUNNING.store(false, Ordering::Release);
                // (Asked again as it stopped: run again.)
                if ASKED.lock().unwrap().is_some_and(|at| at.elapsed() < KEPT_ON) {
                    asked();
                }
            })
            .expect("a thread started");
    }
}

/// A program's sounds held from one reading to the next, for how loud each sounds (by its key).
type Held = Vec<(String, Vec<IAudioMeterInformation>)>;

/// Reading and setting (see the top), till it is no longer asked for.
fn run(sets: &Receiver<Set>) {
    crate::actions::com();
    let Ok(devices) = enumerator() else { return };
    let mut held: Held = Vec::new();
    let mut read_at: Option<Instant> = None;
    while ASKED.lock().unwrap().is_some_and(|at| at.elapsed() < KEPT_ON) {
        // (Counted before what was asked is set: see `ASKED_TO_SET`.)
        let asked_before = ASKED_TO_SET.load(Ordering::Acquire);
        let mut chosen = None;
        let mut apps: Vec<(String, Option<u8>, Option<bool>)> = Vec::new();
        while let Ok(set) = sets.try_recv() {
            match set {
                Set::Output(id) => chosen = Some(id),
                Set::App(key, level, muted) => match apps.iter_mut().find(|(of, _, _)| *of == key) {
                    Some(app) => *app = (key, level.or(app.1), muted.or(app.2)),
                    None => apps.push((key, level, muted)),
                },
            }
        }
        let had_sets = chosen.is_some() || !apps.is_empty();
        if let Some(id) = chosen {
            let _ = make_default(&id);
        }
        if !apps.is_empty() {
            set_apps(&devices, &apps);
        }
        if had_sets || read_at.is_none_or(|at| at.elapsed() >= READ_EVERY) {
            read_at = Some(Instant::now());
            if let Ok((mixer, meters)) = read(&devices) {
                held = meters;
                if ASKED_TO_SET.load(Ordering::Acquire) == asked_before {
                    *MIXER.lock().unwrap() = mixer;
                }
            }
        }
        let peaks = held.iter().map(|(key, meters)| (key.clone(), meters.iter().filter_map(|meter| unsafe { meter.GetPeakValue() }.ok()).fold(0.0f32, f32::max))).collect();
        *PEAKS.lock().unwrap() = Some(peaks);
        std::thread::sleep(PEAKS_EVERY);
    }
    *PEAKS.lock().unwrap() = None;
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

/// The outputs, the one played on now, and its programs (with what tells how loud each sounds).
fn read(devices: &IMMDeviceEnumerator) -> Result<(Mixer, Held)> {
    unsafe {
        let list = devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let outputs: Vec<Output> = (0..list.GetCount()?).filter_map(|at| list.Item(at).ok()).filter_map(|device| described(&device)).collect();
        let Ok(device) = devices.GetDefaultAudioEndpoint(eRender, eConsole) else {
            return Ok((Mixer { outputs, current: None, apps: Vec::new(), read: true }, Vec::new()));
        };
        let current = id_of(&device);
        let (apps, held) = programs(&device)?;
        Ok((Mixer { outputs, current, apps, read: true }, held))
    }
}

/// Windows' id of `device`.
fn id_of(device: &IMMDevice) -> Option<String> {
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const _));
        text
    }
}

/// The properties an output is told by (see `described`): what its driver calls it ("扬声器", a screen's name), and
/// what the device it is on is called (the card, or a Bluetooth or USB device's own name).
const DEVICE_DESCRIPTION: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 2 };
const INTERFACE_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822), pid: 2 };
/// What bus lists the device it is on ("BTHENUM", "USB", "HDAUDIO").
const ENUMERATOR: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 24 };
const FORM_FACTOR: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e), pid: 0 };

/// `device` as an output: named as Windows' own sound settings name it. One on a device of its own (Bluetooth, USB) by
/// that device's name, said to be on its bus; one on the PC's own sound by what its driver calls it, said to be on
/// that card.
fn described(device: &IMMDevice) -> Option<Output> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let text = |key: &PROPERTYKEY| -> Option<String> {
            let value = store.GetValue(key).ok()?;
            let text = PropVariantToStringAlloc(&value).ok()?;
            let said = text.to_string().ok();
            CoTaskMemFree(Some(text.0 as *const _));
            said.map(|said| said.trim().to_string()).filter(|said| !said.is_empty())
        };
        let description = text(&DEVICE_DESCRIPTION);
        let interface = text(&INTERFACE_NAME);
        let form = store.GetValue(&FORM_FACTOR).ok().and_then(|value| PropVariantToUInt32(&value).ok()).map_or(Form::Speakers, form_of);
        let (name, via) = match bus(text(&ENUMERATOR).as_deref()) {
            Some(via) => (interface.or(description)?, via),
            None => (description.or_else(|| interface.clone())?, Via::Card(interface.unwrap_or_default())),
        };
        Some(Output { id: id_of(device)?, name, via, form })
    }
}

/// The bus a device is on, by what lists it (see `ENUMERATOR`), where it is a device of its own: Bluetooth's (its
/// classic profiles', its hands-free one's, Low Energy's), USB's; none for the PC's own sound.
fn bus(enumerator: Option<&str>) -> Option<Via> {
    match enumerator?.to_ascii_uppercase().as_str() {
        "BTHENUM" | "BTHHFENUM" | "BTHLEDEVICE" | "BTHLE" => Some(Via::Bluetooth),
        "USB" => Some(Via::Usb),
        _ => None,
    }
}

/// What an output is, by its form factor (Windows' `EndpointFormFactor`).
fn form_of(factor: u32) -> Form {
    match factor {
        3 => Form::Headphones,
        5 => Form::Headset,
        6 => Form::Handset,
        9 => Form::Screen,
        // (A line out; a digital one, S/PDIF or passed through.)
        2 | 7 | 8 => Form::Line,
        // (Speakers; a network's, or one it does not say.)
        _ => Form::Speakers,
    }
}

/// The programs on `device`, a program's sounds together; with what tells how loud each sounds. Windows' own sounds
/// last; a program whose sound is gone (its session expired) not among them.
fn programs(device: &IMMDevice) -> Result<(Vec<App>, Held)> {
    unsafe {
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        let mut apps: Vec<App> = Vec::new();
        let mut held: Held = Vec::new();
        for at in 0..sessions.GetCount()? {
            let Ok(session) = sessions.GetSession(at) else { continue };
            let Ok(session) = session.cast::<IAudioSessionControl2>() else { continue };
            let Ok(state) = session.GetState() else { continue };
            if state == AudioSessionStateExpired {
                continue;
            }
            let Ok(volume) = session.cast::<ISimpleAudioVolume>() else { continue };
            let (Ok(level), Ok(muted)) = (volume.GetMasterVolume(), volume.GetMute()) else { continue };
            let system = session.IsSystemSoundsSession() == HRESULT(0);
            let Some((key, name, file)) = (if system { Some((SYSTEM.to_string(), String::new(), None)) } else { known(&session) }) else { continue };
            let sounding = state == AudioSessionStateActive;
            let level = (level * 100.0).round() as u8;
            match apps.iter_mut().find(|app| app.key == key) {
                // (Several sounds of one program: as loud as the loudest, muted where all are, sounding where one is.)
                Some(app) => {
                    app.level = app.level.max(level);
                    app.muted = app.muted && muted.as_bool();
                    app.sounding = app.sounding || sounding;
                }
                None => apps.push(App { key: key.clone(), name, file, level, muted: muted.as_bool(), sounding, system }),
            }
            if let Ok(meter) = session.cast::<IAudioMeterInformation>() {
                match held.iter_mut().find(|(of, _)| *of == key) {
                    Some((_, meters)) => meters.push(meter),
                    None => held.push((key, vec![meter])),
                }
            }
        }
        // (Windows' own sounds last, the others as Windows lists them.)
        apps.sort_by_key(|app| app.system);
        Ok((apps, held))
    }
}

/// The program a session is of, by what the session says (see `media::named`): its key (its file's path, from the
/// device up, as the session says it, whatever its case), its name (see `media::described`), its file on its drive.
fn known(session: &IAudioSessionControl2) -> Option<(String, String, Option<PathBuf>)> {
    unsafe {
        let called = session.GetSessionInstanceIdentifier().ok()?;
        let identifier = called.to_string().unwrap_or_default();
        CoTaskMemFree(Some(called.0 as *const _));
        let (file_name, path) = crate::media::named(&identifier)?;
        let name = crate::media::described(&path).unwrap_or(file_name);
        Some((path.to_lowercase(), name, crate::media::on_drives(&path).map(PathBuf::from)))
    }
}

/// The programs `apps` asks of (each by its key: see `App::key`) set as asked, on the output played on now: each of a
/// program's sounds.
fn set_apps(devices: &IMMDeviceEnumerator, apps: &[(String, Option<u8>, Option<bool>)]) {
    unsafe {
        let Ok(device) = devices.GetDefaultAudioEndpoint(eRender, eConsole) else { return };
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return };
        let Ok(sessions) = manager.GetSessionEnumerator() else { return };
        for at in 0..sessions.GetCount().unwrap_or(0) {
            let Ok(session) = sessions.GetSession(at).and_then(|session| session.cast::<IAudioSessionControl2>()) else { continue };
            let key = if session.IsSystemSoundsSession() == HRESULT(0) { Some(SYSTEM.to_string()) } else { known(&session).map(|(key, _, _)| key) };
            let Some((_, level, muted)) = key.and_then(|key| apps.iter().find(|(of, _, _)| *of == key)) else { continue };
            let Ok(volume) = session.cast::<ISimpleAudioVolume>() else { continue };
            if let Some(level) = level {
                let _ = volume.SetMasterVolume(*level as f32 / 100.0, std::ptr::null());
            }
            if let Some(muted) = muted {
                let _ = volume.SetMute(*muted, std::ptr::null());
            }
        }
    }
}

/// Windows' own setting of the output played on, as its sound settings and its quick settings use it. Not documented
/// (it is the one way there is): the policy client's interface, as Windows 7 to 11 have it, of which only the setting
/// of the default is used.
#[allow(non_snake_case)]
mod policy {
    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;

    #[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows_core::IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, device: PCWSTR, endpoint: *mut core::ffi::c_void, mix: *mut core::ffi::c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, default_period: *mut i64, least_period: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn GetPropertyValue(&self, device: PCWSTR, key: *const PROPERTYKEY, value: *mut core::ffi::c_void) -> HRESULT;
        fn SetPropertyValue(&self, device: PCWSTR, key: *const PROPERTYKEY, value: *mut core::ffi::c_void) -> HRESULT;
        fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }

    /// `device` made the default for `role` (its methods are this module's alone).
    pub unsafe fn set_default(config: &IPolicyConfig, device: PCWSTR, role: ERole) -> windows::core::Result<()> {
        unsafe { config.SetDefaultEndpoint(device, role).ok() }
    }
}
use policy::IPolicyConfig;

/// The policy client the interface is had of.
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// The output `id` made the one played on, for every use.
fn make_default(id: &str) -> Result<()> {
    let id: Vec<u16> = id.encode_utf16().chain([0]).collect();
    unsafe {
        let config: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        for role in [eConsole, eMultimedia, eCommunications] {
            policy::set_default(&config, PCWSTR(id.as_ptr()), role)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_on_a_device_of_its_own_is_said_to_be_on_its_bus() {
        assert_eq!(bus(Some("BTHENUM")), Some(Via::Bluetooth));
        assert_eq!(bus(Some("BthLEDevice")), Some(Via::Bluetooth));
        assert_eq!(bus(Some("USB")), Some(Via::Usb));
        assert_eq!(bus(Some("HDAUDIO")), None);
        assert_eq!(bus(None), None);
    }

    #[test]
    fn what_an_output_is_follows_its_form_factor() {
        assert_eq!(form_of(1), Form::Speakers);
        assert_eq!(form_of(3), Form::Headphones);
        assert_eq!(form_of(5), Form::Headset);
        assert_eq!(form_of(9), Form::Screen);
        assert_eq!(form_of(8), Form::Line);
        assert_eq!(form_of(10), Form::Speakers);
    }

    /// The outputs and programs of the PC it runs on, printed (nothing set).
    #[test]
    #[ignore = "reads this PC's sound"]
    fn reads_this_pc() {
        crate::actions::com();
        let devices = enumerator().unwrap();
        let (mixer, _) = read(&devices).unwrap();
        for output in &mixer.outputs {
            println!("{}{output:?}", if mixer.current.as_ref() == Some(&output.id) { "* " } else { "  " });
        }
        for app in &mixer.apps {
            println!("  {app:?}");
        }
    }

    /// Another output chosen as the page does it (`choose`) is the one played on for every use, read back as Windows
    /// has it, as the page is told it and as the panel's row names it; the one before is chosen again after. It changes
    /// the PC's sound: run on a test PC only (`--ignored chooses_another_output_and_back`).
    #[test]
    #[ignore = "changes this PC's output: a test PC only"]
    fn chooses_another_output_and_back() {
        crate::actions::com();
        let devices = enumerator().unwrap();
        let defaults = || -> Vec<Option<String>> {
            [eConsole, eMultimedia, eCommunications].into_iter().map(|role| unsafe { devices.GetDefaultAudioEndpoint(eRender, role) }.ok().and_then(|device| id_of(&device))).collect()
        };
        let (read_now, _) = read(&devices).unwrap();
        let before = read_now.current.clone().expect("an output played on");
        let other = read_now.outputs.iter().map(|output| output.id.clone()).find(|id| *id != before).expect("a second output");
        let told = |id: &str| {
            let start = Instant::now();
            loop {
                let shown = mixer();
                if shown.read && shown.current.as_deref() == Some(id) && output().is_some_and(|output| output.id == id) {
                    return;
                }
                assert!(start.elapsed() < Duration::from_secs(5), "not told {id} is played on");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        told(&before);
        choose(&other);
        told(&other);
        assert_eq!(defaults(), vec![Some(other.clone()); 3]);
        choose(&before);
        told(&before);
        assert_eq!(defaults(), vec![Some(before.clone()); 3]);
        println!("chose {other} and back to {before}");
    }

    /// A program's level and muting set as the page does it (`set_app`, the system's sounds here: always listed) are
    /// what Windows has for it, read back; then set back as they were. It changes the PC's sound: run on a test PC only
    /// (`--ignored sets_a_programs_level_and_back`).
    #[test]
    #[ignore = "changes this PC's sound: a test PC only"]
    fn sets_a_programs_level_and_back() {
        crate::actions::com();
        let devices = enumerator().unwrap();
        let system = || read(&devices).unwrap().0.apps.into_iter().find(|app| app.key == SYSTEM).expect("the system's sounds listed");
        let before = system();
        let read_as = |level: u8, muted: bool| {
            let start = Instant::now();
            loop {
                let now = system();
                if (now.level, now.muted) == (level, muted) {
                    return;
                }
                assert!(start.elapsed() < Duration::from_secs(5), "read {}% {}, not {level}% {muted}", now.level, now.muted);
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let start = Instant::now();
        while !mixer().read {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(50));
        }
        set_app(SYSTEM, Some(37), Some(!before.muted));
        read_as(37, !before.muted);
        set_app(SYSTEM, Some(before.level), Some(before.muted));
        read_as(before.level, before.muted);
        println!("set the system's sounds to 37% and back to {}%", before.level);
    }
}
