//! Sensors only a driver can reach, read through PawnIO: the CPU's own
//! temperature and power. Needs administrator rights and the PawnIO driver;
//! without either there are simply no readings.

use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use crate::pawnio::Module;

/// The modules, compiled by the PawnIO project and shipped beside the program.
const AMD_FAMILY_17: &[u8] = include_bytes!("../../pawnio-modules/AMDFamily17.bin");

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

/// The PCI bus lock every hardware monitor on Windows agrees on: SMN reads
/// go through a pair of PCI registers, which two programs must not interleave.
struct PciLock(HANDLE);

impl PciLock {
    fn acquire() -> Option<Self> {
        let mutex = unsafe { CreateMutexW(None, false, w!("Global\\Access_PCI")) }.ok()?;
        let waited = unsafe { WaitForSingleObject(mutex, 10) };
        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            Some(PciLock(mutex))
        } else {
            let _ = unsafe { CloseHandle(mutex) };
            None
        }
    }
}

impl Drop for PciLock {
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
        Some(AmdCpu { module, ccd_base, energy_unit, last_energy: None })
    }

    /// One SMN register. Normally under the shared PCI lock. Some programs
    /// hold that lock for good (fan and LCD utilities that never release
    /// it); then the register is read twice and taken only if both agree, as
    /// a read interleaved with another program's would not.
    fn smn(&self, locked: bool, address: u64) -> Option<u64> {
        let read = || self.module.read("ioctl_read_smn", address).ok();
        if locked {
            return read();
        }
        let first = read()?;
        (read()? == first).then_some(first)
    }

    pub fn read(&mut self) -> CpuSensors {
        let mut sensors = CpuSensors::default();
        let lock = PciLock::acquire();
        let locked = lock.is_some();
        if let Some(raw) = self.smn(locked, THM_TCON_CUR_TMP) {
            let offset = if raw & (1 << 19) != 0 { 49.0 } else { 0.0 };
            sensors.temp = Some(((raw >> 21) & 0x7FF) as f32 * 0.125 - offset);
        }
        for ccd in 0..MAX_CCDS {
            let Some(raw) = self.smn(locked, self.ccd_base + ccd * 4) else { continue };
            let raw = raw & 0xFFF;
            let celsius = raw as f32 * 0.125 - 305.0;
            // An absent CCD reads zero.
            if raw > 0 && celsius < 125.0 {
                sensors.ccds.push(celsius);
            }
        }
        drop(lock);
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
    fn reads_this_cpu() {
        println!("family/model {:x?}", cpu_family_model());
        let mut cpu = AmdCpu::open().expect("PawnIO and an AMD Zen CPU, elevated");
        cpu.read();
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("{:?}", cpu.read());
    }
}

