//! The power each graphics card draws, as its maker's driver reports it:
//! NVIDIA's through NVML, AMD's through ADL, Intel's through its control
//! library (IGCL), each shipped with the driver. Windows' own adapter data
//! gives power only as a share of an unstated maximum. NVIDIA's and AMD's
//! integrated graphics are left out: what they draw is part of the CPU
//! package's power, shown with the CPU. Intel's library also says how warm
//! its graphics are, their clock and their fan, which Windows' adapter data
//! does not have for them.
//!
//! Glance may run elevated, so the libraries are loaded from System32 only,
//! where the drivers put them and no ordinary program can.
//!
//! A library is opened, and a card found through it, only once every card of
//! its maker is awake, as Windows says just before it (see `GpuPower::reader`):
//! a laptop's discrete GPU sleeps while nothing draws on it, and finding it
//! through its maker's library starts it up (NVML's finding a card does, as
//! NVIDIA documents), as asking it what it is or where it sits may, which IGCL
//! and NVAPI do of each card as they look for another. Nor is a card found
//! asked anything once it sleeps: each of its readings' calls is made only as
//! it is said awake just before (see `GpuPower::more`).

use std::ffi::c_void;

use crate::reading::GpuLimit;

use windows::core::{s, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HEAP_ZERO_MEMORY};

/// Where a card sits on the PCI bus: bus, device and function numbers.
pub type PciAddress = (u32, u32, u32);

/// How one card's power is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reader {
    Nvidia(usize),
    Amd(usize),
    Intel(usize),
}

/// What a card's maker's library says of it besides its power.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct More {
    pub temp: Option<f32>,
    /// The warmest point of the chip, where the library says it apart.
    pub hot_spot: Option<f32>,
    pub clock_mhz: Option<f32>,
    pub fan_rpm: Option<u32>,
}

impl Reader {
    /// Where a card's power comes from, for a report.
    pub fn describe(self) -> &'static str {
        match self {
            Reader::Nvidia(_) => "NVML",
            Reader::Amd(_) => "ADL",
            Reader::Intel(_) => "IGCL",
        }
    }
}

/// The vendors' libraries: each none till a card of its maker is first looked up (see `reader`), then whether the
/// machine has it. Made, used and dropped on the libraries' own thread alone (see `gpu_libraries`): their handles are
/// not to be sent to another.
pub struct GpuPower {
    nvml: Option<Option<Nvml>>,
    nvapi: Option<Option<Nvapi>>,
    adl: Option<Option<Adl>>,
    igcl: Option<Option<Igcl>>,
}

/// The makers' PCI vendor ids (Intel's is `INTEL_VENDOR`).
const NVIDIA_VENDOR: u32 = 0x10DE;
const AMD_PCI_VENDOR: u32 = 0x1002;

/// The library `library`, opened by `open` the first time it is asked for.
fn opened<T>(library: &mut Option<Option<T>>, open: fn() -> Option<T>) -> Option<&mut T> {
    library.get_or_insert_with(open).as_mut()
}

impl GpuPower {
    /// None of the libraries opened yet: each is as a card of its maker is first looked up.
    pub fn new() -> Self {
        GpuPower { nvml: None, nvapi: None, adl: None, igcl: None }
    }

    fn nvml(&self) -> Option<&Nvml> {
        self.nvml.as_ref()?.as_ref()
    }

    fn nvapi(&self) -> Option<&Nvapi> {
        self.nvapi.as_ref()?.as_ref()
    }

    fn adl(&self) -> Option<&Adl> {
        self.adl.as_ref()?.as_ref()
    }

    fn igcl(&self) -> Option<&Igcl> {
        self.igcl.as_ref()?.as_ref()
    }

    /// How to read the discrete card made by `vendor` (its PCI vendor id) at `address`, whether its maker's library
    /// says (none: not looked up yet, a card of its maker asleep): that library opened first, if it is not yet, and
    /// the card found through it by where it sits (no other card is found meanwhile, though IGCL's and NVAPI's
    /// looking asks the others what they are or where they sit). Each library is asked only as `awake` says every
    /// card of its maker is, just before it (see the module's note): NVIDIA's card is looked up through NVML and then
    /// NVAPI, and NVML's opening and finding may take long enough for another of NVIDIA's cards to have gone to sleep
    /// meanwhile. One asleep by then, the card is looked up again from NVAPI's step when this is next asked, what NVML
    /// found of it kept (and NVML not asked of it again).
    pub fn reader(&mut self, vendor: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>> {
        match vendor {
            NVIDIA_VENDOR => {
                if !awake() {
                    return None;
                }
                let Some(index) = opened(&mut self.nvml, Nvml::open).and_then(|nvml| nvml.device(address)) else { return Some(None) };
                // (And its hot spot, if NVAPI tells it: see `more`.)
                if !awake() {
                    return None;
                }
                if let Some(nvapi) = opened(&mut self.nvapi, Nvapi::open) {
                    nvapi.find(address);
                }
                Some(Some(Reader::Nvidia(index)))
            }
            AMD_PCI_VENDOR => awake().then(|| opened(&mut self.adl, Adl::open).and_then(|adl| adl.adapter(address)).map(Reader::Amd)),
            INTEL_VENDOR => awake().then(|| opened(&mut self.igcl, Igcl::open).and_then(|igcl| igcl.device(address)).map(Reader::Intel)),
            _ => Some(None),
        }
    }

    /// What else the card's maker's library says of it now: Intel's, all
    /// that Windows does not have of its graphics; NVIDIA's and AMD's, the
    /// warmest point of the chip, on the cards that say it, and whether its
    /// fans stand still (AMD's, their speed). Each call into a library is
    /// made only as `awake` says the card is just before it (a call before
    /// it may have taken long), as with each of its readings (see `read`
    /// and `limit`): what is not asked is unread.
    pub fn more(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<More> {
        match reader {
            Reader::Intel(index) => self.igcl()?.more(index, awake),
            Reader::Nvidia(index) => {
                let nvml = self.nvml()?;
                let at = nvml.devices.get(index)?.0;
                // Its fans standing still: none turning, not unread (Windows' adapter data has no word for that).
                let fan_rpm = nvml.fans_still(index, awake).filter(|still| *still).map(|_| 0);
                Some(More { hot_spot: self.nvapi().and_then(|nvapi| nvapi.hot_spot(at, awake)), fan_rpm, ..More::default() })
            }
            Reader::Amd(index) => self.adl()?.more(index, awake),
        }
    }

    /// What Intel's library found, for a report: each of its graphics looked up, where it sits, and what it reads
    /// of it, where `awake` says the graphics at that place are just before they are asked (found awake, they may
    /// have gone to sleep since, a laptop's Arc card while the rest of the hardware was looked at).
    pub fn describe_intel(&self, awake: impl Fn(PciAddress) -> bool) -> Option<String> {
        let igcl = self.igcl()?;
        let of = |(index, device): (usize, &IntelDevice)| format!("{:?}: {}", device.at, igcl.told(index, &mut || awake(device.at)));
        Some(igcl.devices.iter().enumerate().map(of).collect::<Vec<_>>().join("; "))
    }

    /// Whether it is Intel's graphics built into the CPU (whose power the CPU may count apart: see
    /// `CpuSensors::graphics_power`).
    pub fn intel_integrated(&self, reader: Reader) -> bool {
        match reader {
            Reader::Intel(index) => self.igcl().and_then(|igcl| igcl.devices.get(index)).is_some_and(|device| device.integrated),
            Reader::Nvidia(_) | Reader::Amd(_) => false,
        }
    }

    /// The card's power now, in watts, asked only as `awake` says the card is just before.
    pub fn read(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        match reader {
            Reader::Nvidia(index) => self.nvml()?.power(index, awake),
            Reader::Amd(index) => self.adl()?.power(index, awake),
            Reader::Intel(index) => self.igcl()?.power(index, awake),
        }
    }

    /// What holds the card's clock back now, where its driver says (only
    /// NVIDIA's does), asked only as `awake` says the card is just before.
    pub fn limit(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<GpuLimit> {
        match reader {
            Reader::Nvidia(index) => self.nvml()?.limit(index, awake),
            Reader::Amd(_) | Reader::Intel(_) => None,
        }
    }
}

/// A function the library exports, as `F`.
unsafe fn export<F>(library: HMODULE, name: PCSTR) -> Option<F> {
    let address = unsafe { GetProcAddress(library, name) }?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

fn system_library(name: PCWSTR) -> Option<HMODULE> {
    unsafe { LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()
}

// ---- NVIDIA: NVML (nvml.h) ----

const NVML_SUCCESS: i32 = 0;

// The reasons NVML gives for a clock below the most it may run at
// (nvmlClocksEventReason*): the power limit, the GPU's own temperature and
// the board's slowdown (for heat or for its power supply, said apart by
// the two after it where the driver can).
const SW_POWER_CAP: u64 = 0x4;
const HW_SLOWDOWN: u64 = 0x8;
const SW_THERMAL_SLOWDOWN: u64 = 0x20;
const HW_THERMAL_SLOWDOWN: u64 = 0x40;
const HW_POWER_BRAKE_SLOWDOWN: u64 = 0x80;

/// What holds a clock back, from NVML's reasons; heat before power where
/// both do (the hotter card is the one to look at).
fn limit_of(reasons: u64) -> GpuLimit {
    if reasons & (SW_THERMAL_SLOWDOWN | HW_THERMAL_SLOWDOWN) != 0 {
        GpuLimit::Thermal
    } else if reasons & (SW_POWER_CAP | HW_POWER_BRAKE_SLOWDOWN) != 0 {
        GpuLimit::Power
    } else if reasons & HW_SLOWDOWN != 0 {
        GpuLimit::Hardware
    } else {
        GpuLimit::Free
    }
}

type NvmlDevice = *mut c_void;

struct Nvml {
    library: HMODULE,
    /// Finds a card by where it sits (and starts it up, as every way NVML has of finding one does).
    by_bus_id: unsafe extern "C" fn(*const u8, *mut NvmlDevice) -> i32,
    power_usage: unsafe extern "C" fn(NvmlDevice, *mut u32) -> i32,
    /// The reasons the clock is held back; absent from drivers before NVML
    /// had them.
    clock_reasons: Option<unsafe extern "C" fn(NvmlDevice, *mut u64) -> i32>,
    /// How many fans a card has, and each one's speed (a share of its most); absent from drivers before NVML had
    /// them.
    fans: Option<(unsafe extern "C" fn(NvmlDevice, *mut u32) -> i32, unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> i32)>,
    shutdown: unsafe extern "C" fn() -> i32,
    /// The cards found, by where each sits.
    devices: Vec<(PciAddress, NvmlDevice)>,
}

impl Nvml {
    fn open() -> Option<Self> {
        let library = system_library(windows::core::w!("nvml.dll"))?;
        let opened = unsafe { Self::start(library) };
        if opened.is_none() {
            let _ = unsafe { FreeLibrary(library) };
        }
        opened
    }

    unsafe fn start(library: HMODULE) -> Option<Self> {
        let init: unsafe extern "C" fn() -> i32 = unsafe { export(library, s!("nvmlInit_v2")) }?;
        let shutdown: unsafe extern "C" fn() -> i32 = unsafe { export(library, s!("nvmlShutdown")) }?;
        let by_bus_id = unsafe { export(library, s!("nvmlDeviceGetHandleByPciBusId_v2")) }?;
        let power_usage = unsafe { export(library, s!("nvmlDeviceGetPowerUsage")) }?;
        // Named "throttle" reasons before driver 535 ("clocks event" since).
        let clock_reasons = unsafe { export(library, s!("nvmlDeviceGetCurrentClocksEventReasons")) }
            .or_else(|| unsafe { export(library, s!("nvmlDeviceGetCurrentClocksThrottleReasons")) });
        let fans = unsafe { export(library, s!("nvmlDeviceGetNumFans")) }.zip(unsafe { export(library, s!("nvmlDeviceGetFanSpeed_v2")) });
        // (Which starts no card up, as NVIDIA documents it: each is, as it is found.)
        if unsafe { init() } != NVML_SUCCESS {
            return None;
        }
        Some(Nvml { library, by_bus_id, power_usage, clock_reasons, fans, shutdown, devices: Vec::new() })
    }

    /// The card at `at`, by its index among those found: found the first time it is asked for.
    fn device(&mut self, at: PciAddress) -> Option<usize> {
        if let Some(index) = self.devices.iter().position(|(known, _)| *known == at) {
            return Some(index);
        }
        // Its bus id as NVML takes one, bus:device.function in hex (in the first PCI domain, as Windows' own
        // account of where an adapter sits has it).
        let bus_id = format!("{:x}:{:x}.{:x}\0", at.0, at.1, at.2);
        let mut device = std::ptr::null_mut();
        (unsafe { (self.by_bus_id)(bus_id.as_ptr(), &mut device) } == NVML_SUCCESS).then(|| {
            self.devices.push((at, device));
            self.devices.len() - 1
        })
    }

    fn power(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        let device = self.devices.get(index)?.1;
        if !awake() {
            return None;
        }
        let mut milliwatts = 0u32;
        let read = unsafe { (self.power_usage)(device, &mut milliwatts) };
        (read == NVML_SUCCESS).then(|| milliwatts as f32 / 1000.0)
    }

    /// Whether the card's fans are all standing still (as a card's do while it is cool, at rest), where it says it
    /// has fans, and none where it has none, or does not say, or is not said awake just before one is asked of.
    fn fans_still(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<bool> {
        let (count, speed) = self.fans?;
        let device = self.devices.get(index)?.1;
        let mut fans = 0u32;
        if !awake() || unsafe { count(device, &mut fans) } != NVML_SUCCESS || fans == 0 {
            return None;
        }
        let mut still = true;
        for fan in 0..fans {
            let mut share = 0u32;
            if !awake() || unsafe { speed(device, fan, &mut share) } != NVML_SUCCESS {
                return None;
            }
            still &= share == 0;
        }
        Some(still)
    }

    fn limit(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<GpuLimit> {
        let (reasons_of, device) = (self.clock_reasons?, self.devices.get(index)?.1);
        if !awake() {
            return None;
        }
        let mut reasons = 0u64;
        let read = unsafe { reasons_of(device, &mut reasons) };
        (read == NVML_SUCCESS).then(|| limit_of(reasons))
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            (self.shutdown)();
            let _ = FreeLibrary(self.library);
        }
    }
}

// ---- NVIDIA: NVAPI, for what NVML does not have ----
//
// The warmest point of a card's chip is not in NVML, nor in what NVIDIA
// documents of NVAPI: it is asked for as other monitors ask for it, by the
// call that reads all of the card's thermal sensors, of which the second is
// that point on the cards before the RTX 50 series. From that series on the
// driver no longer tells it (the second sensor there is the chip's
// temperature again: seen on an RTX 5070 Ti, driver 617.14), so those cards
// are not asked.

const NVAPI_OK: i32 = 0;
/// The first architecture whose cards do not tell their hot spot (the RTX 50 series').
const NV_ARCHITECTURE_UNTOLD_FROM: u32 = 0x1B0;
/// Which of the thermal sensors the hot spot is, and what a sensor with nothing to say reads.
const NV_HOT_SPOT: usize = 1;
const NV_NOTHING: i32 = 255 * 256;

#[repr(C)]
struct NvThermalSensors {
    version: u32,
    mask: u32,
    reserved: [i32; 8],
    /// In 1/256 °C.
    temperatures: [i32; 32],
}

#[repr(C)]
struct NvArchitecture {
    version: u32,
    architecture: u32,
    implementation: u32,
    revision: u32,
}

/// What NVAPI is asked of a card: its handle, and where to put the answer.
type NvCardQuery<T> = unsafe extern "C" fn(*mut c_void, *mut T) -> i32;

struct Nvapi {
    sensors: NvCardQuery<NvThermalSensors>,
    bus: NvCardQuery<u32>,
    slot: NvCardQuery<u32>,
    architecture: NvCardQuery<NvArchitecture>,
    /// Every card NVAPI has.
    cards: Vec<*mut c_void>,
    /// The cards found that tell their hot spot, by where each sits.
    devices: Vec<(PciAddress, *mut c_void)>,
}

impl Nvapi {
    fn open() -> Option<Self> {
        let library = system_library(windows::core::w!("nvapi64.dll"))?;
        let opened = unsafe { Self::start(library) };
        if opened.is_none() {
            let _ = unsafe { FreeLibrary(library) };
        }
        opened
    }

    unsafe fn start(library: HMODULE) -> Option<Self> {
        // Every function is had from the one the library exports, by a number of its own.
        let query: unsafe extern "C" fn(u32) -> *const c_void = unsafe { export(library, s!("nvapi_QueryInterface")) }?;
        unsafe fn had<F>(query: unsafe extern "C" fn(u32) -> *const c_void, number: u32) -> Option<F> {
            let address = unsafe { query(number) };
            (!address.is_null()).then(|| unsafe { std::mem::transmute_copy(&address) })
        }
        let initialize: unsafe extern "C" fn() -> i32 = unsafe { had(query, 0x0150_E828) }?;
        let enumerate: unsafe extern "C" fn(*mut [*mut c_void; 64], *mut u32) -> i32 = unsafe { had(query, 0xE5AC_921F) }?;
        let bus = unsafe { had(query, 0x1BE0_B8E5) }?;
        let slot = unsafe { had(query, 0x2A0A_350F) }?;
        let architecture = unsafe { had(query, 0xD826_5D24) }?;
        let sensors = unsafe { had(query, 0x65FE_3AAD) }?;
        if unsafe { initialize() } != NVAPI_OK {
            return None;
        }
        let (mut cards, mut count) = ([std::ptr::null_mut(); 64], 0u32);
        let listed = unsafe { enumerate(&mut cards, &mut count) } == NVAPI_OK;
        let cards = if listed { cards.into_iter().take(count as usize).collect() } else { Vec::new() };
        Some(Nvapi { sensors, bus, slot, architecture, cards, devices: Vec::new() })
    }

    /// Finds the card at `at`, kept if it is one that tells its hot spot: the cards asked where each sits till it is
    /// found, and that one alone what it is.
    fn find(&mut self, at: PciAddress) {
        for &card in &self.cards {
            let (mut on_bus, mut in_slot) = (0u32, 0u32);
            let placed = unsafe { (self.bus)(card, &mut on_bus) == NVAPI_OK && (self.slot)(card, &mut in_slot) == NVAPI_OK };
            if !placed || (on_bus, in_slot, 0) != at {
                continue;
            }
            let mut about = NvArchitecture { version: size_of::<NvArchitecture>() as u32 | 2 << 16, architecture: 0, implementation: 0, revision: 0 };
            if unsafe { (self.architecture)(card, &mut about) } == NVAPI_OK && about.architecture < NV_ARCHITECTURE_UNTOLD_FROM {
                self.devices.push((at, card));
            }
            return;
        }
    }

    /// The warmest point of the chip of the card at `at` (°C), if it is one that tells it, asked only as `awake` says
    /// it is just before.
    fn hot_spot(&self, at: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        let card = self.devices.iter().find(|(theirs, _)| *theirs == at)?.1;
        if !awake() {
            return None;
        }
        let mut read = NvThermalSensors { version: size_of::<NvThermalSensors>() as u32 | 2 << 16, mask: 1 << NV_HOT_SPOT, reserved: [0; 8], temperatures: [0; 32] };
        let told = unsafe { (self.sensors)(card, &mut read) } == NVAPI_OK;
        let raw = read.temperatures[NV_HOT_SPOT];
        (told && raw > 0 && raw < NV_NOTHING).then(|| raw as f32 / 256.0)
    }
}

// (The library is left loaded: it has no call to close what `initialize` opened that every driver has.)

// ---- AMD: ADL (adl_sdk.h, adl_structures.h, adl_defines.h) ----

const ADL_OK: i32 = 0;
const ADL_MAX_PATH: usize = 256;
const ADL_ASIC_DISCRETE: i32 = 1 << 0;
const ADL_PMLOG_MAX_SENSORS: usize = 256;
/// The whole board's power, on newer cards; the chip's alone, on the rest.
const PMLOG_BOARD_POWER: usize = 73;
const PMLOG_ASIC_POWER: usize = 23;
const PMLOG_TEMPERATURE_HOTSPOT: usize = 27;
const PMLOG_FAN_RPM: usize = 14;
/// AMD's PCI vendor id 1002h, as ADL writes it: the hex digits read as a
/// decimal number (AMD's sample compares with 1002; ADL also lists other
/// makers' cards, an NVIDIA one as 10).
const AMD_VENDOR: i32 = 1002;

/// AdapterInfo, as Windows builds lay it out.
#[repr(C)]
struct AdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; ADL_MAX_PATH],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; ADL_MAX_PATH],
    display_name: [u8; ADL_MAX_PATH],
    present: i32,
    exist: i32,
    driver_path: [u8; ADL_MAX_PATH],
    driver_path_ext: [u8; ADL_MAX_PATH],
    pnp_string: [u8; ADL_MAX_PATH],
    os_display_index: i32,
}

/// ADLPMLogDataOutput.
#[repr(C)]
struct PmLogData {
    size: i32,
    /// Each sensor: whether it is supported, and its value.
    sensors: [(i32, i32); ADL_PMLOG_MAX_SENSORS],
}

type AdlContext = *mut c_void;

struct Adl {
    library: HMODULE,
    context: AdlContext,
    query: unsafe extern "C" fn(AdlContext, i32, *mut PmLogData) -> i32,
    destroy: unsafe extern "C" fn(AdlContext) -> i32,
    asic: unsafe extern "C" fn(AdlContext, i32, *mut i32, *mut i32) -> i32,
    /// Every AMD adapter ADL lists, by where it sits, with ADL's index for it.
    listed: Vec<(PciAddress, i32)>,
    /// The discrete cards found, likewise.
    adapters: Vec<(PciAddress, i32)>,
}

/// What ADL allocates with: memory it is given and never frees here (the
/// calls made return nothing allocated).
unsafe extern "system" fn adl_allocate(size: i32) -> *mut c_void {
    unsafe { HeapAlloc(GetProcessHeap().unwrap_or_default(), HEAP_ZERO_MEMORY, size.max(0) as usize) }
}

impl Adl {
    fn open() -> Option<Self> {
        let library = system_library(windows::core::w!("atiadlxx.dll"))?;
        let opened = unsafe { Self::start(library) };
        if opened.is_none() {
            let _ = unsafe { FreeLibrary(library) };
        }
        opened
    }

    unsafe fn start(library: HMODULE) -> Option<Self> {
        type Allocate = unsafe extern "system" fn(i32) -> *mut c_void;
        let create: unsafe extern "C" fn(Allocate, i32, *mut AdlContext) -> i32 = unsafe { export(library, s!("ADL2_Main_Control_Create")) }?;
        let destroy: unsafe extern "C" fn(AdlContext) -> i32 = unsafe { export(library, s!("ADL2_Main_Control_Destroy")) }?;
        let count: unsafe extern "C" fn(AdlContext, *mut i32) -> i32 = unsafe { export(library, s!("ADL2_Adapter_NumberOfAdapters_Get")) }?;
        let infos: unsafe extern "C" fn(AdlContext, *mut AdapterInfo, i32) -> i32 = unsafe { export(library, s!("ADL2_Adapter_AdapterInfo_Get")) }?;
        let asic = unsafe { export(library, s!("ADL2_Adapter_ASICFamilyType_Get")) }?;
        let query = unsafe { export(library, s!("ADL2_New_QueryPMLogData_Get")) }?;
        let mut context = std::ptr::null_mut();
        // Only the adapters that are there.
        if unsafe { create(adl_allocate, 1, &mut context) } != ADL_OK {
            return None;
        }
        let mut listed = Vec::new();
        let mut n = 0;
        if unsafe { count(context, &mut n) } == ADL_OK && n > 0 {
            let mut list: Vec<AdapterInfo> = (0..n).map(|_| unsafe { std::mem::zeroed() }).collect();
            let bytes = (list.len() * size_of::<AdapterInfo>()) as i32;
            if unsafe { infos(context, list.as_mut_ptr(), bytes) } == ADL_OK {
                for info in &list {
                    let address = (info.bus_number as u32, info.device_number as u32, info.function_number as u32);
                    // ADL lists a card once for each of its outputs.
                    if info.vendor_id == AMD_VENDOR && !listed.iter().any(|(at, _)| *at == address) {
                        listed.push((address, info.adapter_index));
                    }
                }
            }
        }
        Some(Adl { library, context, query, destroy, asic, listed, adapters: Vec::new() })
    }

    /// The discrete card at `at`, by its index among those found: found (and asked what it is) the first time it is
    /// asked for. None for graphics built into the CPU.
    fn adapter(&mut self, at: PciAddress) -> Option<usize> {
        if let Some(index) = self.adapters.iter().position(|(known, _)| *known == at) {
            return Some(index);
        }
        let &(_, adapter) = self.listed.iter().find(|(known, _)| *known == at)?;
        let (mut types, mut valid) = (0, 0);
        let read = unsafe { (self.asic)(self.context, adapter, &mut types, &mut valid) };
        (read == ADL_OK && valid & ADL_ASIC_DISCRETE != 0 && types & ADL_ASIC_DISCRETE != 0).then(|| {
            self.adapters.push((at, adapter));
            self.adapters.len() - 1
        })
    }

    fn power(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        let adapter = self.adapters.get(index)?.1;
        if !awake() {
            return None;
        }
        let mut data = PmLogData { size: size_of::<PmLogData>() as i32, sensors: [(0, 0); ADL_PMLOG_MAX_SENSORS] };
        if unsafe { (self.query)(self.context, adapter, &mut data) } != ADL_OK {
            return None;
        }
        [PMLOG_BOARD_POWER, PMLOG_ASIC_POWER]
            .into_iter()
            .map(|sensor| data.sensors[sensor])
            .find(|(supported, _)| *supported != 0)
            .map(|(_, watts)| watts as f32)
    }

    /// What its driver logs of the card now: the warmest point of its chip (°C), and its fans' speed (RPM: 0, its
    /// fans standing still, as a card's do while it is cool, at rest; not unread), each where it logs it.
    fn more(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<More> {
        let adapter = self.adapters.get(index)?.1;
        if !awake() {
            return None;
        }
        let mut data = PmLogData { size: size_of::<PmLogData>() as i32, sensors: [(0, 0); ADL_PMLOG_MAX_SENSORS] };
        if unsafe { (self.query)(self.context, adapter, &mut data) } != ADL_OK {
            return None;
        }
        let (supported, celsius) = data.sensors[PMLOG_TEMPERATURE_HOTSPOT];
        let hot_spot = (supported != 0 && celsius > 0).then_some(celsius as f32);
        let (supported, rpm) = data.sensors[PMLOG_FAN_RPM];
        let fan_rpm = (supported != 0 && rpm >= 0).then_some(rpm as u32);
        Some(More { hot_spot, fan_rpm, ..More::default() })
    }
}

impl Drop for Adl {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.context);
            let _ = FreeLibrary(self.library);
        }
    }
}

// ---- Intel: IGCL (igcl_api.h) ----

const CTL_SUCCESS: u32 = 0;
/// The version of the library this is written for: 1.1.
const CTL_VERSION: u32 = 1 << 16 | 1;
const INTEL_VENDOR: u32 = 0x8086;
type CtlHandle = *mut c_void;

#[repr(C)]
struct CtlInitArgs {
    size: u32,
    version: u8,
    app_version: u32,
    flags: u32,
    supported_version: u32,
    application: [u32; 4],
}

/// `CtlAdapterProperties::graphics_adapter_properties`: the graphics are built into the CPU.
const CTL_ADAPTER_PROPERTIES_FLAG_INTEGRATED: u32 = 1;

#[repr(C)]
struct CtlAdapterProperties {
    size: u32,
    version: u8,
    device_id: *mut c_void,
    device_id_size: u32,
    device_type: i32,
    supported: u32,
    driver_version: u64,
    firmware_version: [u64; 3],
    pci_vendor_id: u32,
    pci_device_id: u32,
    rev_id: u32,
    eus_per_sub_slice: u32,
    sub_slices_per_slice: u32,
    slices: u32,
    name: [u8; 100],
    graphics_adapter_properties: u32,
    frequency: u32,
    pci_subsys_id: u16,
    pci_subsys_vendor_id: u16,
    bdf: [u8; 3],
    xe_cores: u32,
    reserved: [u8; 108],
}

/// A value the library tells, of some type and in some unit.
#[repr(C)]
#[derive(Clone, Copy)]
struct CtlItem {
    supported: bool,
    units: i32,
    kind: i32,
    /// A union of every type a value may have, the widest of them 8 bytes.
    value: u64,
}

impl CtlItem {
    /// The value as a number, whatever type it is told in; none where the library does not tell it.
    fn number(&self) -> Option<f64> {
        if !self.supported {
            return None;
        }
        let bytes = self.value.to_ne_bytes();
        Some(match self.kind {
            0 => bytes[0] as i8 as f64,
            1 => bytes[0] as f64,
            2 => i16::from_ne_bytes([bytes[0], bytes[1]]) as f64,
            3 => u16::from_ne_bytes([bytes[0], bytes[1]]) as f64,
            4 => i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            5 => u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            6 => self.value as i64 as f64,
            7 => self.value as f64,
            8 => f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            9 => f64::from_ne_bytes(bytes),
            _ => return None,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlPsu {
    supported: bool,
    kind: i32,
    energy: CtlItem,
    voltage: CtlItem,
}

/// ctl_power_telemetry_t, as the header has it at version 1.
#[repr(C)]
struct CtlTelemetry {
    size: u32,
    version: u8,
    time_stamp: CtlItem,
    gpu_energy: CtlItem,
    gpu_voltage: CtlItem,
    gpu_clock: CtlItem,
    gpu_temperature: CtlItem,
    global_activity: CtlItem,
    render_activity: CtlItem,
    media_activity: CtlItem,
    gpu_limited: [bool; 5],
    vram_energy: CtlItem,
    vram_voltage: CtlItem,
    vram_clock: CtlItem,
    vram_effective_clock: CtlItem,
    vram_read: CtlItem,
    vram_written: CtlItem,
    vram_temperature: CtlItem,
    vram_limited: [bool; 5],
    card_energy: CtlItem,
    psu: [CtlPsu; 5],
    fans: [CtlItem; 5],
    gpu_vr_temperature: CtlItem,
    vram_vr_temperature: CtlItem,
    sa_vr_temperature: CtlItem,
    gpu_effective_clock: CtlItem,
    gpu_over_voltage: CtlItem,
    gpu_power_percent: CtlItem,
    gpu_temperature_percent: CtlItem,
    vram_read_bandwidth: CtlItem,
    vram_write_bandwidth: CtlItem,
}

struct IntelDevice {
    at: PciAddress,
    /// Built into the CPU (not a card of its own).
    integrated: bool,
    handle: CtlHandle,
    /// The energy counter as last read, and when (the library's own clock, seconds): the power is what was
    /// drawn between two readings.
    last: std::cell::Cell<Option<(f64, f64)>>,
}

struct Igcl {
    library: HMODULE,
    api: CtlHandle,
    close: unsafe extern "C" fn(CtlHandle) -> u32,
    telemetry: unsafe extern "C" fn(CtlHandle, *mut CtlTelemetry) -> u32,
    properties: unsafe extern "C" fn(CtlHandle, *mut CtlAdapterProperties) -> u32,
    /// Every device the library has.
    handles: Vec<CtlHandle>,
    /// Intel's graphics found, by where each sits.
    devices: Vec<IntelDevice>,
}

impl Igcl {
    fn open() -> Option<Self> {
        let library = system_library(windows::core::w!("ControlLib.dll"))?;
        let opened = unsafe { Self::start(library) };
        if opened.is_none() {
            let _ = unsafe { FreeLibrary(library) };
        }
        opened
    }

    unsafe fn start(library: HMODULE) -> Option<Self> {
        let init: unsafe extern "C" fn(*mut CtlInitArgs, *mut CtlHandle) -> u32 = unsafe { export(library, s!("ctlInit")) }?;
        let close: unsafe extern "C" fn(CtlHandle) -> u32 = unsafe { export(library, s!("ctlClose")) }?;
        let enumerate: unsafe extern "C" fn(CtlHandle, *mut u32, *mut CtlHandle) -> u32 = unsafe { export(library, s!("ctlEnumerateDevices")) }?;
        let properties = unsafe { export(library, s!("ctlGetDeviceProperties")) }?;
        let telemetry = unsafe { export(library, s!("ctlPowerTelemetryGet")) }?;
        let mut args = CtlInitArgs { size: size_of::<CtlInitArgs>() as u32, version: 0, app_version: CTL_VERSION, flags: 0, supported_version: 0, application: [0; 4] };
        let mut api: CtlHandle = std::ptr::null_mut();
        if unsafe { init(&mut args, &mut api) } != CTL_SUCCESS || api.is_null() {
            return None;
        }
        let mut found = Igcl { library, api, close, telemetry, properties, handles: Vec::new(), devices: Vec::new() };
        let mut count = 0u32;
        if unsafe { enumerate(api, &mut count, std::ptr::null_mut()) } != CTL_SUCCESS || count == 0 {
            return Some(found);
        }
        let mut handles = vec![std::ptr::null_mut(); count as usize];
        if unsafe { enumerate(api, &mut count, handles.as_mut_ptr()) } == CTL_SUCCESS {
            found.handles = handles.into_iter().take(count as usize).collect();
        }
        Some(found)
    }

    /// Intel's graphics at `at`, by its index among those found: the devices asked what each is till it is found,
    /// the first time it is asked for.
    fn device(&mut self, at: PciAddress) -> Option<usize> {
        if let Some(index) = self.devices.iter().position(|device| device.at == at) {
            return Some(index);
        }
        for &handle in &self.handles {
            // (The adapter's own identifier, as Windows has it, is asked for with the rest: 8 bytes for it.)
            let mut luid = [0u8; 8];
            let mut about: CtlAdapterProperties = unsafe { std::mem::zeroed() };
            about.size = size_of::<CtlAdapterProperties>() as u32;
            about.version = 2;
            about.device_id = luid.as_mut_ptr().cast();
            about.device_id_size = luid.len() as u32;
            let told = unsafe { (self.properties)(handle, &mut about) } == CTL_SUCCESS && about.pci_vendor_id == INTEL_VENDOR;
            if told && (about.bdf[0] as u32, about.bdf[1] as u32, about.bdf[2] as u32) == at {
                let integrated = about.graphics_adapter_properties & CTL_ADAPTER_PROPERTIES_FLAG_INTEGRATED != 0;
                self.devices.push(IntelDevice { at, integrated, handle, last: std::cell::Cell::new(None) });
                return Some(self.devices.len() - 1);
            }
        }
        None
    }

    /// What the library reads of the graphics now, asked only as `awake` says they are just before.
    fn read(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<CtlTelemetry> {
        if !awake() {
            return None;
        }
        self.telemetry_of(index)
    }

    /// What the library reads of the graphics now (whoever asks has said they are awake just before).
    fn telemetry_of(&self, index: usize) -> Option<CtlTelemetry> {
        let mut read: CtlTelemetry = unsafe { std::mem::zeroed() };
        read.size = size_of::<CtlTelemetry>() as u32;
        read.version = 1;
        (unsafe { (self.telemetry)(self.devices.get(index)?.handle, &mut read) } == CTL_SUCCESS).then_some(read)
    }

    /// The watts drawn since it was last asked: the energy counter's rise over the time between.
    fn power(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        let read = self.read(index, awake)?;
        let now = (read.time_stamp.number()?, read.gpu_energy.number()?);
        let before = self.devices[index].last.replace(Some(now))?;
        let (seconds, joules) = (now.0 - before.0, now.1 - before.1);
        (seconds > 0.0 && joules >= 0.0).then(|| (joules / seconds) as f32)
    }

    fn more(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> Option<More> {
        let read = self.read(index, awake)?;
        Some(More {
            // (The temperature it gives is its warmest sensor's: no other to tell apart from it.)
            hot_spot: None,
            temp: read.gpu_temperature.number().filter(|celsius| *celsius > 0.0).map(|celsius| celsius as f32),
            clock_mhz: read.gpu_clock.number().filter(|mhz| *mhz > 0.0).map(|mhz| mhz as f32),
            // (The fastest of the fans it reads; one it reads standing still is 0, not unread: a card's fans stop
            // while it is idle.)
            fan_rpm: read.fans.iter().filter_map(CtlItem::number).filter(|rpm| *rpm >= 0.0).fold(None, |fastest: Option<f64>, rpm| Some(fastest.map_or(rpm, |fastest| fastest.max(rpm)))).map(|rpm| rpm as u32),
        })
    }

    /// Which of its readings the library gives for a device, for a report, asked only as `awake` says it is just
    /// before.
    fn told(&self, index: usize, awake: &mut dyn FnMut() -> bool) -> String {
        if !awake() {
            return "asleep, not asked".into();
        }
        let Some(read) = self.telemetry_of(index) else { return "telemetry not read".into() };
        let has = |item: &CtlItem| if item.supported { "yes" } else { "no" };
        format!("temperature {}, clock {}, energy {}, fans {}", has(&read.gpu_temperature), has(&read.gpu_clock), has(&read.gpu_energy), read.fans.iter().filter(|fan| fan.supported).count())
    }
}

impl Drop for Igcl {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.api);
            let _ = FreeLibrary(self.library);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Made, it has opened none of the libraries: each waits for a card of its maker found awake.
    #[test]
    fn opens_no_library_till_a_card_is_looked_up() {
        let power = super::GpuPower::new();
        assert!(power.nvml.is_none() && power.nvapi.is_none() && power.adl.is_none() && power.igcl.is_none());
    }

    use super::*;
    use std::cell::{Cell, RefCell};

    thread_local! {
        /// What the libraries standing in for the makers' below were asked, in order.
        static ASKED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        /// Whether the Arc card standing in for one below has gone to sleep.
        static ARC_ASLEEP: Cell<bool> = const { Cell::new(false) };
        /// Whether the NVIDIA card standing in for one below has gone to sleep.
        static NVIDIA_ASLEEP: Cell<bool> = const { Cell::new(false) };
    }

    fn asked(what: &'static str) {
        ASKED.with_borrow_mut(|asked| asked.push(what));
    }

    /// What was asked since this was last taken.
    fn taken() -> Vec<&'static str> {
        ASKED.with_borrow_mut(std::mem::take)
    }

    // NVML and NVAPI as they stand for NVIDIA's below: one card, on bus 2, of an architecture that tells its hot spot.
    unsafe extern "C" fn nvml_by_bus_id(_: *const u8, device: *mut NvmlDevice) -> i32 {
        asked("NVML: found");
        unsafe { *device = 1 as NvmlDevice };
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvml_power(_: NvmlDevice, _: *mut u32) -> i32 {
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvml_shutdown() -> i32 {
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvapi_bus(_: *mut c_void, bus: *mut u32) -> i32 {
        asked("NVAPI: where");
        unsafe { *bus = 2 };
        NVAPI_OK
    }
    unsafe extern "C" fn nvapi_slot(_: *mut c_void, slot: *mut u32) -> i32 {
        unsafe { *slot = 0 };
        NVAPI_OK
    }
    unsafe extern "C" fn nvapi_architecture(_: *mut c_void, about: *mut NvArchitecture) -> i32 {
        asked("NVAPI: what");
        unsafe { (*about).architecture = 0x190 };
        NVAPI_OK
    }
    unsafe extern "C" fn nvapi_sensors(_: *mut c_void, _: *mut NvThermalSensors) -> i32 {
        NVAPI_OK
    }

    // NVML and NVAPI as they stand for NVIDIA's card being read below: each call takes long enough for the card to go
    // to sleep, and it has a fan.
    unsafe extern "C" fn nvml_power_meanwhile(_: NvmlDevice, _: *mut u32) -> i32 {
        asked("NVML: power");
        NVIDIA_ASLEEP.set(true);
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvml_fans_meanwhile(_: NvmlDevice, fans: *mut u32) -> i32 {
        asked("NVML: fans");
        NVIDIA_ASLEEP.set(true);
        unsafe { *fans = 1 };
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvml_fan_speed(_: NvmlDevice, _: u32, _: *mut u32) -> i32 {
        asked("NVML: fan speed");
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvml_reasons(_: NvmlDevice, _: *mut u64) -> i32 {
        asked("NVML: reasons");
        NVML_SUCCESS
    }
    unsafe extern "C" fn nvapi_sensors_asked(_: *mut c_void, _: *mut NvThermalSensors) -> i32 {
        asked("NVAPI: sensors");
        NVAPI_OK
    }

    /// Each call of a card's readings is made only as it is said awake just before it, not once for them all: an
    /// NVIDIA card gone to sleep as its power was read is asked nothing more (neither its fans, nor its hot spot, nor
    /// what holds its clock back), and one gone to sleep as its fans were counted has neither a fan's speed asked nor
    /// its hot spot. Awake all the while, each is asked.
    #[test]
    fn asks_each_reading_only_as_its_card_is_awake() {
        let at = (2, 0, 0);
        let nvml = Nvml {
            library: HMODULE::default(),
            by_bus_id: nvml_by_bus_id,
            power_usage: nvml_power_meanwhile,
            clock_reasons: Some(nvml_reasons),
            fans: Some((nvml_fans_meanwhile, nvml_fan_speed)),
            shutdown: nvml_shutdown,
            devices: vec![(at, 1 as NvmlDevice)],
        };
        let nvapi = Nvapi { sensors: nvapi_sensors_asked, bus: nvapi_bus, slot: nvapi_slot, architecture: nvapi_architecture, cards: Vec::new(), devices: vec![(at, 1 as *mut c_void)] };
        let power = GpuPower { nvml: Some(Some(nvml)), nvapi: Some(Some(nvapi)), adl: None, igcl: None };
        let card = Reader::Nvidia(0);
        let mut awake = || !NVIDIA_ASLEEP.get();
        assert!(power.read(card, &mut awake).is_some());
        assert!(power.more(card, &mut awake).is_some_and(|more| more.hot_spot.is_none() && more.fan_rpm.is_none()));
        assert_eq!(power.limit(card, &mut awake), None);
        assert_eq!(taken(), ["NVML: power"]);
        NVIDIA_ASLEEP.set(false);
        power.more(card, &mut awake);
        assert_eq!(taken(), ["NVML: fans"]);
        NVIDIA_ASLEEP.set(false);
        power.limit(card, &mut awake);
        power.more(card, &mut || true);
        assert_eq!(taken(), ["NVML: reasons", "NVML: fans", "NVML: fan speed", "NVAPI: sensors"]);
        // (Its libraries stand in for the makers' and have none to close.)
        std::mem::forget(power);
    }

    /// NVIDIA's card is looked up through NVML and then NVAPI, its maker's cards asked whether they are all awake just
    /// before each: one of them gone to sleep while NVML was opened and asked has NVAPI asked nothing (where it would
    /// ask each card where it sits), the look-up waiting, and once they are all awake again it goes on from NVAPI's
    /// step, NVML not asked again. A card of its maker asleep before any is asked, no library is opened, and a card of
    /// another maker waits for none.
    #[test]
    fn asks_each_library_only_as_its_makers_cards_are_awake() {
        let nvml = Nvml { library: HMODULE::default(), by_bus_id: nvml_by_bus_id, power_usage: nvml_power, clock_reasons: None, fans: None, shutdown: nvml_shutdown, devices: Vec::new() };
        let nvapi = Nvapi { sensors: nvapi_sensors, bus: nvapi_bus, slot: nvapi_slot, architecture: nvapi_architecture, cards: vec![1 as *mut c_void], devices: Vec::new() };
        let mut power = GpuPower { nvml: Some(Some(nvml)), nvapi: Some(Some(nvapi)), adl: None, igcl: None };
        let at = (2, 0, 0);
        let mut awake = [true, false].into_iter();
        assert_eq!(power.reader(NVIDIA_VENDOR, at, &mut || awake.next().unwrap()), None);
        assert_eq!(taken(), ["NVML: found"]);
        assert_eq!(power.reader(NVIDIA_VENDOR, at, &mut || true), Some(Some(Reader::Nvidia(0))));
        assert_eq!(taken(), ["NVAPI: where", "NVAPI: what"]);
        assert_eq!(power.nvapi().map(|nvapi| nvapi.devices.len()), Some(1));
        // (Its libraries stand in for the makers' and have none to close.)
        std::mem::forget(power);
        let mut none_yet = GpuPower::new();
        for vendor in [NVIDIA_VENDOR, AMD_PCI_VENDOR, INTEL_VENDOR] {
            assert_eq!(none_yet.reader(vendor, at, &mut || false), None);
        }
        assert!(none_yet.nvml.is_none() && none_yet.nvapi.is_none() && none_yet.adl.is_none() && none_yet.igcl.is_none());
        assert_eq!(none_yet.reader(0x1AF4, at, &mut || false), Some(None));
    }

    // IGCL as it stands for Intel's below: each of its graphics' telemetry read takes long enough for the Arc card to
    // go to sleep.
    unsafe extern "C" fn igcl_telemetry(_: CtlHandle, read: *mut CtlTelemetry) -> u32 {
        asked("IGCL: telemetry");
        ARC_ASLEEP.set(true);
        unsafe { (*read).gpu_temperature.supported = true };
        CTL_SUCCESS
    }
    unsafe extern "C" fn igcl_properties(_: CtlHandle, _: *mut CtlAdapterProperties) -> u32 {
        CTL_SUCCESS
    }
    unsafe extern "C" fn igcl_close(_: CtlHandle) -> u32 {
        CTL_SUCCESS
    }

    /// Intel's graphics found are asked what they read, for a report, only where they are awake just before: an Arc
    /// card found awake, gone to sleep since (as the graphics in the CPU before it were asked), is asked nothing.
    #[test]
    fn describes_intels_graphics_asking_none_asleep() {
        let device = |at, handle: usize| IntelDevice { at, integrated: handle == 1, handle: handle as CtlHandle, last: Cell::new(None) };
        let devices = vec![device((0, 2, 0), 1), device((3, 0, 0), 2)];
        let igcl = Igcl { library: HMODULE::default(), api: std::ptr::null_mut(), close: igcl_close, telemetry: igcl_telemetry, properties: igcl_properties, handles: Vec::new(), devices };
        let power = GpuPower { nvml: None, nvapi: None, adl: None, igcl: Some(Some(igcl)) };
        let told = power.describe_intel(|at| at != (3, 0, 0) || !ARC_ASLEEP.get());
        assert_eq!(taken(), ["IGCL: telemetry"]);
        assert_eq!(told.as_deref(), Some("(0, 2, 0): temperature yes, clock no, energy no, fans 0; (3, 0, 0): asleep, not asked"));
        std::mem::forget(power);
    }

    #[test]
    fn lays_out_intels_structures() {
        use super::*;
        // As igcl_api.h lays them out, on 64-bit Windows.
        assert_eq!(size_of::<CtlItem>(), 24);
        assert_eq!(size_of::<CtlPsu>(), 8 + 2 * 24);
        assert_eq!(size_of::<CtlInitArgs>(), 4 + 4 + 4 + 4 + 4 + 16);
        assert_eq!(size_of::<CtlTelemetry>(), 8 + 8 * 24 + 8 + 7 * 24 + 8 + 24 + 5 * 56 + 5 * 24 + 9 * 24);
        assert_eq!(std::mem::offset_of!(CtlAdapterProperties, pci_vendor_id), 8 + 8 + 4 + 4 + 4 + 4 + 8 + 24);
        assert_eq!(std::mem::offset_of!(CtlAdapterProperties, bdf), 64 + 6 * 4 + 100 + 4 + 4 + 2 + 2);
        assert_eq!((size_of::<NvThermalSensors>(), size_of::<NvArchitecture>()), (168, 16));
        // (And as a program compiled against the header itself says they are.)
        assert_eq!((size_of::<CtlTelemetry>(), size_of::<CtlAdapterProperties>()), (1024, 320));
        assert_eq!(std::mem::offset_of!(CtlTelemetry, time_stamp), 8);
        assert_eq!(std::mem::offset_of!(CtlTelemetry, gpu_energy), 32);
        assert_eq!(std::mem::offset_of!(CtlTelemetry, gpu_clock), 80);
        assert_eq!(std::mem::offset_of!(CtlTelemetry, gpu_temperature), 104);
        assert_eq!(std::mem::offset_of!(CtlTelemetry, fans), 688);
        // A value in whichever type it is told.
        let item = |kind: i32, value: u64| CtlItem { supported: true, units: 0, kind, value };
        assert_eq!(item(9, 61.5f64.to_bits()).number(), Some(61.5));
        assert_eq!(item(8, 2400.0f32.to_bits() as u64).number(), Some(2400.0));
        assert_eq!(item(5, 1800).number(), Some(1800.0));
        assert_eq!(CtlItem { supported: false, ..item(9, 0) }.number(), None);
    }

    #[test]
    fn lays_out_the_vendors_structures() {
        // As their headers lay them out, on 64-bit Windows.
        assert_eq!(size_of::<super::AdapterInfo>(), 4 * 2 + 256 + 4 * 4 + 256 * 2 + 4 * 2 + 256 * 3 + 4);
        assert_eq!(size_of::<super::PmLogData>(), 4 + 256 * 8);
    }

    #[test]
    fn names_what_holds_a_clock_back() {
        use super::*;
        assert_eq!(limit_of(0), GpuLimit::Free);
        // Idle, and clocks the user set: nothing holding it back.
        assert_eq!(limit_of(0x1 | 0x2), GpuLimit::Free);
        assert_eq!(limit_of(SW_POWER_CAP), GpuLimit::Power);
        assert_eq!(limit_of(HW_SLOWDOWN | HW_POWER_BRAKE_SLOWDOWN), GpuLimit::Power);
        assert_eq!(limit_of(HW_SLOWDOWN | HW_THERMAL_SLOWDOWN), GpuLimit::Thermal);
        assert_eq!(limit_of(SW_POWER_CAP | SW_THERMAL_SLOWDOWN), GpuLimit::Thermal);
        assert_eq!(limit_of(HW_SLOWDOWN), GpuLimit::Hardware);
    }
}
