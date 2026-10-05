//! Sensors only a driver can reach, read through PawnIO: the CPU's own
//! temperature and power. Needs administrator rights and the PawnIO driver;
//! without either there are simply no readings.

use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use crate::pawnio::Module;

/// The modules, compiled by the PawnIO project and shipped beside the program.
const AMD_FAMILY_17: &[u8] = include_bytes!("../pawnio-modules/AMDFamily17.bin");

// AMD Zen register map (families 17h, 19h, 1Ah).
/// Tctl, the control temperature: CUR_TEMP in bits 31–21, in eighths of a
/// degree; bit 19 selects the range that starts at −49 °C.
const THM_TCON_CUR_TMP: u64 = 0x0005_9800;
/// Each CCD's temperature, one word per CCD, from Zen 4 on; and before that.
const CCD_TEMP_ZEN4: u64 = 0x0005_9B08;
const CCD_TEMP_ZEN2: u64 = 0x0005_9954;
const MAX_CCDS: u64 = 8;
const MSR_PWR_UNIT: u64 = 0xC001_0299;
const MSR_PKG_ENERGY_STAT: u64 = 0xC001_029B;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CpuSensors {
    pub temp: Option<f32>,
    /// Each chiplet's temperature, on CPUs that have several.
    pub ccds: Vec<f32>,
    pub power: Option<f32>,
}

/// A lock that every hardware monitor on Windows agrees to take before
/// touching a shared piece of hardware: the PCI configuration registers
/// (SMN reads go through a pair of them) or the ISA bus (Super I/O chips).
/// Two programs must not interleave their accesses.
pub struct NamedLock(HANDLE);

/// How long to wait for another program's use of a bus to end (ms): longer
/// than its transfers take, short enough not to hold the readings up.
pub const LOCK_WAIT: u32 = 50;

/// The PCI bus lock's name.
const PCI_LOCK: PCWSTR = w!("Global\\Access_PCI");

impl NamedLock {
    /// Waits up to `timeout_ms` for the lock; `None` if another program has it.
    pub fn acquire(name: PCWSTR, timeout_ms: u32) -> Option<Self> {
        let mutex = unsafe { CreateMutexW(None, false, name) }.ok()?;
        let waited = unsafe { WaitForSingleObject(mutex, timeout_ms) };
        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            Some(NamedLock(mutex))
        } else {
            let _ = unsafe { CloseHandle(mutex) };
            None
        }
    }
}

impl Drop for NamedLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct AmdCpu {
    module: Module,
    ccd_base: u64,
    /// Joules per count of the energy counter.
    energy_unit: f64,
    last_energy: Option<(u32, Instant)>,
    /// The latest temperatures, kept while another program has the PCI bus.
    last: (Option<f32>, Vec<f32>),
}

impl AmdCpu {
    /// The CPU's sensors, if it is an AMD Zen and the driver can be used.
    pub fn open() -> Option<Self> {
        let module = Module::load(AMD_FAMILY_17).ok()?;
        let (family, model) = cpu_family_model();
        // Raphael (Zen 4, 19h model 61h) and later use the newer CCD block.
        let ccd_base = if family >= 0x1A || (family == 0x19 && model >= 0x60) { CCD_TEMP_ZEN4 } else { CCD_TEMP_ZEN2 };
        let units = module.read("ioctl_read_msr", MSR_PWR_UNIT).ok()?;
        let energy_unit = 1.0 / (1u64 << ((units >> 8) & 0x1F)) as f64;
        Some(AmdCpu { module, ccd_base, energy_unit, last_energy: None, last: (None, Vec::new()) })
    }

    /// One SMN register. Reached through an index and a data register on
    /// the PCI bus, so only under the PCI lock other programs share.
    fn smn(&self, address: u64) -> Option<u64> {
        self.module.read("ioctl_read_smn", address).ok()
    }

    pub fn read(&mut self) -> CpuSensors {
        let mut sensors = CpuSensors::default();
        // While another program has the bus, the last temperatures stand.
        if let Some(_lock) = NamedLock::acquire(PCI_LOCK, LOCK_WAIT) {
            let mut ccds = Vec::new();
            let temp = self.smn(THM_TCON_CUR_TMP).map(|raw| {
                let offset = if raw & (1 << 19) != 0 { 49.0 } else { 0.0 };
                ((raw >> 21) & 0x7FF) as f32 * 0.125 - offset
            });
            for ccd in 0..MAX_CCDS {
                let Some(raw) = self.smn(self.ccd_base + ccd * 4) else { continue };
                let raw = raw & 0xFFF;
                let celsius = raw as f32 * 0.125 - 305.0;
                // An absent CCD reads zero.
                if raw > 0 && celsius < 125.0 {
                    ccds.push(celsius);
                }
            }
            self.last = (temp, ccds);
        }
        (sensors.temp, sensors.ccds) = self.last.clone();
        // The package's energy counter, 32 bits wide: power is its rate.
        if let Ok(raw) = self.module.read("ioctl_read_msr", MSR_PKG_ENERGY_STAT) {
            let now = Instant::now();
            let count = raw as u32;
            if let Some((before, at)) = self.last_energy {
                let joules = count.wrapping_sub(before) as f64 * self.energy_unit;
                sensors.power = Some((joules / now.duration_since(at).as_secs_f64()) as f32);
            }
            self.last_energy = Some((count, now));
        }
        sensors
    }
}

/// An Intel CPU's sensors, read from its model-specific registers through
/// PawnIO's IntelMSR module: the package's temperature (its hottest core's)
/// and its power. Registers per Intel's Software Developer's Manual, vol. 4.
pub struct IntelCpu {
    module: Module,
    /// The temperature at which the CPU throttles, which readings count down from.
    tj_max: f32,
    /// Joules per count of the energy counter.
    energy_unit: f64,
    last_energy: Option<(u32, Instant)>,
}

const INTEL_MSR: &[u8] = include_bytes!("../pawnio-modules/IntelMSR.bin");
const IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;
const MSR_RAPL_POWER_UNIT: u64 = 0x606;
const MSR_PKG_ENERGY_STATUS: u64 = 0x611;

impl IntelCpu {
    /// The CPU's sensors, if it is an Intel one and the driver can be used
    /// (the module declines to load on other CPUs).
    pub fn open() -> Option<Self> {
        let module = Module::load(INTEL_MSR).ok()?;
        let target = module.read("ioctl_read_msr", IA32_TEMPERATURE_TARGET).ok()?;
        let tj_max = ((target >> 16) & 0xFF) as f32;
        // A CPU that does not report one has nothing to count down from.
        if tj_max == 0.0 {
            return None;
        }
        let energy_unit = module
            .read("ioctl_read_msr", MSR_RAPL_POWER_UNIT)
            .map_or(0.0, |units| 1.0 / (1u64 << ((units >> 8) & 0x1F)) as f64);
        Some(IntelCpu { module, tj_max, energy_unit, last_energy: None })
    }

    pub fn read(&mut self) -> CpuSensors {
        let mut sensors = CpuSensors::default();
        // How far below the throttling point the hottest core is.
        if let Ok(status) = self.module.read("ioctl_read_msr", IA32_PACKAGE_THERM_STATUS) {
            sensors.temp = Some(self.tj_max - ((status >> 16) & 0x7F) as f32);
        }
        if self.energy_unit > 0.0 {
            if let Ok(raw) = self.module.read("ioctl_read_msr", MSR_PKG_ENERGY_STATUS) {
                let now = Instant::now();
                let count = raw as u32;
                if let Some((before, at)) = self.last_energy {
                    let joules = count.wrapping_sub(before) as f64 * self.energy_unit;
                    sensors.power = Some((joules / now.duration_since(at).as_secs_f64()) as f32);
                }
                self.last_energy = Some((count, now));
            }
        }
        sensors
    }
}

/// The CPU's own sensors, whichever maker's.
pub enum CpuReader {
    Amd(AmdCpu),
    Intel(IntelCpu),
}

impl CpuReader {
    pub fn open() -> Option<Self> {
        AmdCpu::open().map(CpuReader::Amd).or_else(|| IntelCpu::open().map(CpuReader::Intel))
    }

    pub fn read(&mut self) -> CpuSensors {
        match self {
            CpuReader::Amd(cpu) => cpu.read(),
            CpuReader::Intel(cpu) => cpu.read(),
        }
    }
}

/// The CPU's family and model, as AMD defines them from CPUID leaf 1.
fn cpu_family_model() -> (u32, u32) {
    let leaf = std::arch::x86_64::__cpuid(1);
    let base_family = (leaf.eax >> 8) & 0xF;
    let base_model = (leaf.eax >> 4) & 0xF;
    let family = if base_family == 0xF { base_family + ((leaf.eax >> 20) & 0xFF) } else { base_family };
    let model = if base_family == 0xF { base_model | (((leaf.eax >> 16) & 0xF) << 4) } else { base_model };
    (family, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs administrator rights and the PawnIO driver.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_this_cpu() {
        println!("family/model {:x?}", cpu_family_model());
        let mut cpu = AmdCpu::open().expect("PawnIO and an AMD Zen CPU, elevated");
        cpu.read();
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("{:?}", cpu.read());
    }
}

