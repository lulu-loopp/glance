//! The power each graphics card draws, as its maker's driver reports it:
//! NVIDIA's through NVML, AMD's through ADL, both shipped with the driver.
//! Windows' own adapter data gives power only as a share of an unstated
//! maximum. Integrated graphics are left out: what they draw is part of the
//! CPU package's power, shown with the CPU.
//!
//! Glance may run elevated, so the libraries are loaded from System32 only,
//! where the drivers put them and no ordinary program can.

use std::ffi::c_void;

use crate::reading::GpuLimit;

use windows::core::{s, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HEAP_ZERO_MEMORY};

/// Where a card sits on the PCI bus: bus, device and function numbers.
pub type PciAddress = (u32, u32, u32);

/// How one card's power is read.
#[derive(Clone, Copy)]
pub enum Reader {
    Nvidia(usize),
    Amd(usize),
}

impl Reader {
    /// Where a card's power comes from, for a report.
    pub fn describe(self) -> &'static str {
        match self {
            Reader::Nvidia(_) => "NVML",
            Reader::Amd(_) => "ADL",
        }
    }
}

/// The vendors' libraries, those the machine has.
pub struct GpuPower {
    nvml: Option<Nvml>,
    adl: Option<Adl>,
}

// The libraries' handles are used from one thread at a time (the sampler's).
unsafe impl Send for GpuPower {}

impl GpuPower {
    pub fn open() -> Self {
        GpuPower { nvml: Nvml::open(), adl: Adl::open() }
    }

    /// How to read the discrete card at `address`, if its driver says.
    pub fn reader(&self, address: PciAddress) -> Option<Reader> {
        let nvidia = self.nvml.as_ref().and_then(|nvml| nvml.devices.iter().position(|(at, _)| *at == address));
        let amd = || self.adl.as_ref().and_then(|adl| adl.adapters.iter().position(|(at, _)| *at == address));
        nvidia.map(Reader::Nvidia).or_else(|| amd().map(Reader::Amd))
    }

    /// The card's power now, in watts.
    pub fn read(&self, reader: Reader) -> Option<f32> {
        match reader {
            Reader::Nvidia(index) => self.nvml.as_ref()?.power(index),
            Reader::Amd(index) => self.adl.as_ref()?.power(index),
        }
    }

    /// What holds the card's clock back now, where its driver says (only
    /// NVIDIA's does).
    pub fn limit(&self, reader: Reader) -> Option<GpuLimit> {
        match reader {
            Reader::Nvidia(index) => self.nvml.as_ref()?.limit(index),
            Reader::Amd(_) => None,
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

/// nvmlPciInfo_t.
#[repr(C)]
struct NvmlPciInfo {
    bus_id_legacy: [u8; 16],
    domain: u32,
    bus: u32,
    device: u32,
    pci_device_id: u32,
    pci_sub_system_id: u32,
    bus_id: [u8; 32],
}

type NvmlDevice = *mut c_void;

struct Nvml {
    library: HMODULE,
    power_usage: unsafe extern "C" fn(NvmlDevice, *mut u32) -> i32,
    /// The reasons the clock is held back; absent from drivers before NVML
    /// had them.
    clock_reasons: Option<unsafe extern "C" fn(NvmlDevice, *mut u64) -> i32>,
    shutdown: unsafe extern "C" fn() -> i32,
    /// Every card, by where it sits (a card's GPU is function 0).
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
        let count: unsafe extern "C" fn(*mut u32) -> i32 = unsafe { export(library, s!("nvmlDeviceGetCount_v2")) }?;
        let handle: unsafe extern "C" fn(u32, *mut NvmlDevice) -> i32 = unsafe { export(library, s!("nvmlDeviceGetHandleByIndex_v2")) }?;
        let pci: unsafe extern "C" fn(NvmlDevice, *mut NvmlPciInfo) -> i32 = unsafe { export(library, s!("nvmlDeviceGetPciInfo_v3")) }?;
        let power_usage = unsafe { export(library, s!("nvmlDeviceGetPowerUsage")) }?;
        // Named "throttle" reasons before driver 535 ("clocks event" since).
        let clock_reasons = unsafe { export(library, s!("nvmlDeviceGetCurrentClocksEventReasons")) }
            .or_else(|| unsafe { export(library, s!("nvmlDeviceGetCurrentClocksThrottleReasons")) });
        if unsafe { init() } != NVML_SUCCESS {
            return None;
        }
        let mut devices = Vec::new();
        let mut n = 0u32;
        if unsafe { count(&mut n) } == NVML_SUCCESS {
            for index in 0..n {
                let mut device = std::ptr::null_mut();
                let mut info: NvmlPciInfo = unsafe { std::mem::zeroed() };
                if unsafe { handle(index, &mut device) } == NVML_SUCCESS && unsafe { pci(device, &mut info) } == NVML_SUCCESS {
                    devices.push(((info.bus, info.device, 0), device));
                }
            }
        }
        Some(Nvml { library, power_usage, clock_reasons, shutdown, devices })
    }

    fn power(&self, index: usize) -> Option<f32> {
        let mut milliwatts = 0u32;
        let read = unsafe { (self.power_usage)(self.devices.get(index)?.1, &mut milliwatts) };
        (read == NVML_SUCCESS).then(|| milliwatts as f32 / 1000.0)
    }

    fn limit(&self, index: usize) -> Option<GpuLimit> {
        let mut reasons = 0u64;
        let read = unsafe { (self.clock_reasons?)(self.devices.get(index)?.1, &mut reasons) };
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

// ---- AMD: ADL (adl_sdk.h, adl_structures.h, adl_defines.h) ----

const ADL_OK: i32 = 0;
const ADL_MAX_PATH: usize = 256;
const ADL_ASIC_DISCRETE: i32 = 1 << 0;
const ADL_PMLOG_MAX_SENSORS: usize = 256;
/// The whole board's power, on newer cards; the chip's alone, on the rest.
const PMLOG_BOARD_POWER: usize = 73;
const PMLOG_ASIC_POWER: usize = 23;
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
    /// Every discrete AMD card, by where it sits, with ADL's index for it.
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
        let asic: unsafe extern "C" fn(AdlContext, i32, *mut i32, *mut i32) -> i32 =
            unsafe { export(library, s!("ADL2_Adapter_ASICFamilyType_Get")) }?;
        let query = unsafe { export(library, s!("ADL2_New_QueryPMLogData_Get")) }?;
        let mut context = std::ptr::null_mut();
        // Only the adapters that are there.
        if unsafe { create(adl_allocate, 1, &mut context) } != ADL_OK {
            return None;
        }
        let mut adapters = Vec::new();
        let mut n = 0;
        if unsafe { count(context, &mut n) } == ADL_OK && n > 0 {
            let mut list: Vec<AdapterInfo> = (0..n).map(|_| unsafe { std::mem::zeroed() }).collect();
            let bytes = (list.len() * size_of::<AdapterInfo>()) as i32;
            if unsafe { infos(context, list.as_mut_ptr(), bytes) } == ADL_OK {
                for info in &list {
                    let address = (info.bus_number as u32, info.device_number as u32, info.function_number as u32);
                    // ADL lists a card once for each of its outputs.
                    if info.vendor_id != AMD_VENDOR || adapters.iter().any(|(at, _)| *at == address) {
                        continue;
                    }
                    let (mut types, mut valid) = (0, 0);
                    let read = unsafe { asic(context, info.adapter_index, &mut types, &mut valid) };
                    if read == ADL_OK && valid & ADL_ASIC_DISCRETE != 0 && types & ADL_ASIC_DISCRETE != 0 {
                        adapters.push((address, info.adapter_index));
                    }
                }
            }
        }
        Some(Adl { library, context, query, destroy, adapters })
    }

    fn power(&self, index: usize) -> Option<f32> {
        let adapter = self.adapters.get(index)?.1;
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
}

impl Drop for Adl {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.context);
            let _ = FreeLibrary(self.library);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lays_out_the_vendors_structures() {
        // As their headers lay them out, on 64-bit Windows.
        assert_eq!(size_of::<super::NvmlPciInfo>(), 16 + 5 * 4 + 32);
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

    /// Prints what this machine's cards draw.
    #[test]
    fn reads_this_machines_cards() {
        let power = super::GpuPower::open();
        if let Some(nvml) = &power.nvml {
            for (index, (at, _)) in nvml.devices.iter().enumerate() {
                println!("NVIDIA at {at:?}: {:?} W, held back by {:?}", nvml.power(index), nvml.limit(index));
            }
        }
        if let Some(adl) = &power.adl {
            println!("AMD discrete cards: {:?}", adl.adapters.iter().map(|(at, _)| at).collect::<Vec<_>>());
        }
    }
}
