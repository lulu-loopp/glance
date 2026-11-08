//! The memory modules' own temperature sensors, on the SMBus: the SPD5118
//! hub of each DDR5 module, or the JC-42.4 sensor of a DDR4 one. Read
//! through PawnIO's SMBus modules, the way Linux's spd5118 and jc42 drivers
//! read them, and nothing is ever written to a module. Only where the firmware
//! lists modules that carry such a sensor, and never on a laptop (see
//! `looked_for`).

use crate::pawnio::Module;
use crate::sensors::{NamedLock, LOCK_WAIT};

use windows::core::w;

/// The chipset's SMBus controller: AMD's (PIIX4-compatible) or Intel's.
const SMBUS_PIIX4: &[u8] = include_bytes!("../pawnio-modules/SmbusPIIX4.bin");
const SMBUS_I801: &[u8] = include_bytes!("../pawnio-modules/SmbusI801.bin");

/// The lock every program using the SMBus takes around its transfers.
const SMBUS_LOCK: windows::core::PCWSTR = w!("Global\\Access_SMBUS.HTP.Method");

const READ: u64 = 1;
const BYTE_DATA: u64 = 2;
const WORD_DATA: u64 = 3;

/// DDR5: each module's SPD hub answers at 0x50 + its slot.
const SPD5118_ADDRESSES: std::ops::Range<u64> = 0x50..0x58;
/// The hub's device type (MR0, MR1), its temperature sensor's configuration
/// (MR26, bit 0 set when the sensor is off) and reading (MR49, MR50).
const SPD5118_TYPE: (u64, u64) = (0x00, 0x01);
const SPD5118_TEMP_CONFIG: u64 = 0x1A;
const SPD5118_TEMP: u64 = 0x31;

/// DDR4: each module's sensor answers at 0x18 + its slot.
const JC42_ADDRESSES: std::ops::Range<u64> = 0x18..0x20;
/// Capabilities, configuration, temperature and manufacturer registers.
const JC42_CAPABILITY: u64 = 0x00;
const JC42_CONFIG: u64 = 0x01;
const JC42_TEMP: u64 = 0x05;
const JC42_MANUFACTURER: u64 = 0x06;

/// Forms of module put into a slot, as SMBIOS numbers them (a DIMM, an SO-DIMM, a CAMM), and the memory types whose
/// modules carry a sensor (a DDR5 module's SPD hub, a DDR4 one's where it has one). Soldered memory (LPDDR, chips on
/// the board) has none.
const DIMM: u8 = 0x09;
const SO_DIMM: u8 = 0x0D;
const CAMM: u8 = 0x11;
const DDR4: u8 = 0x1A;
const DDR5: u8 = 0x22;

/// Which modules' sensors are looked for on the bus.
#[derive(Debug, PartialEq)]
struct LookedFor {
    spd5118: bool,
    jc42: bool,
}

/// Which modules' sensors are looked for on the machine whose firmware's table is `table`: the sensors of the kinds
/// of module in its slots (DDR5's hubs, DDR4's sensors), none where it has none of either, and none on a laptop,
/// whatever it has. A laptop's SMBus is its embedded controller's too (its battery, its charger), whose firmware
/// need not take the lock programs take around the bus, and a transfer that meets one of the firmware's may leave
/// the bus stuck till the power is cut (as PawnIO's own module for AMD's chipsets warns).
fn looked_for(table: &[u8]) -> Result<LookedFor, Missing> {
    if crate::smbios::portable(&crate::smbios::chassis(table)) {
        return Err(Missing::Laptop);
    }
    let modules = crate::smbios::modules(table);
    let in_a_slot = |memory_type| modules.iter().any(|module| matches!(module.form_factor, DIMM | SO_DIMM | CAMM) && module.memory_type == memory_type);
    let looked = LookedFor { spd5118: in_a_slot(DDR5), jc42: in_a_slot(DDR4) };
    if !looked.spd5118 && !looked.jc42 {
        return Err(Missing::NoModules);
    }
    Ok(looked)
}

#[derive(Clone, Copy)]
enum Kind {
    Spd5118,
    Jc42,
}

pub struct Dimms {
    module: Module,
    kind: Kind,
    /// The bus address of each module's sensor, by slot.
    sensors: Vec<u64>,
    /// The latest readings and when they were read, kept while another program holds the bus or a module does not
    /// answer (no longer than `STALE_AFTER`).
    last: (Vec<f32>, std::time::Instant),
}

/// How long the last readings stand while they cannot be read again: after, none (a frozen temperature would show
/// as live).
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// Why no module's sensor was found.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Missing {
    /// The driver the SMBus is reached through is not there (or not to be had without rights).
    NoDriver,
    /// Another program had the bus all the while it was waited for (one that lights the modules, as a rule).
    BusHeld,
    /// None answered as a module's sensor.
    None,
    /// Not looked for: a laptop (see `looked_for`).
    Laptop,
    /// Not looked for: no DDR4 or DDR5 module in a slot, as the firmware lists them.
    NoModules,
}

impl Missing {
    pub fn describe(self) -> &'static str {
        match self {
            Missing::NoDriver => "not found (the SMBus driver not available)",
            Missing::BusHeld => "not found yet (another program had the SMBus; looked for again)",
            Missing::None => "not found",
            Missing::Laptop => "not looked for on a laptop (its SMBus is its embedded controller's too)",
            Missing::NoModules => "not looked for (no DDR4 or DDR5 module in a slot)",
        }
    }
}

impl Dimms {
    /// What was found, for a report.
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            Kind::Spd5118 => "DDR5 SPD5118",
            Kind::Jc42 => "DDR4 JC-42.4",
        };
        format!("{} {kind} sensor(s)", self.sensors.len())
    }

    /// Finds the modules' sensors, if the chipset's SMBus can be reached and there are any to look for (see
    /// `looked_for`), or else says why not.
    pub fn open() -> Result<Self, Missing> {
        let looked = looked_for(&crate::smbios::table())?;
        let module = Module::load(SMBUS_PIIX4).or_else(|_| Module::load(SMBUS_I801)).map_err(|_| Missing::NoDriver)?;
        let _lock = NamedLock::acquire(SMBUS_LOCK, 1000).ok_or(Missing::BusHeld)?;
        let byte = |address, register| transfer(&module, address, register, BYTE_DATA);
        // A DDR5 hub names its type in its first two registers, and its
        // sensor is in use only when not switched off.
        let spd5118: Vec<u64> = SPD5118_ADDRESSES
            .filter(|_| looked.spd5118)
            .filter(|&address| byte(address, SPD5118_TYPE.0) == Some(0x51) && byte(address, SPD5118_TYPE.1) == Some(0x18))
            .filter(|&address| byte(address, SPD5118_TEMP_CONFIG).is_some_and(|config| config & 1 == 0))
            .collect();
        if !spd5118.is_empty() {
            return Ok(Dimms { module, kind: Kind::Spd5118, last: (Vec::new(), std::time::Instant::now()), sensors: spd5118 });
        }
        // A DDR4 sensor: as Linux's jc42 checks, reserved bits clear and a
        // manufacturer set.
        let word = |address, register| transfer(&module, address, register, WORD_DATA).map(|w| (w as u16).swap_bytes());
        let jc42: Vec<u64> = JC42_ADDRESSES
            .filter(|_| looked.jc42)
            .filter(|&address| {
                let (Some(capability), Some(config), Some(maker)) =
                    (word(address, JC42_CAPABILITY), word(address, JC42_CONFIG), word(address, JC42_MANUFACTURER))
                else {
                    return false;
                };
                capability & 0xFE00 == 0 && config & 0xF800 == 0 && maker != 0 && maker != 0xFFFF
            })
            .collect();
        if jc42.is_empty() {
            return Err(Missing::None);
        }
        Ok(Dimms { module, kind: Kind::Jc42, last: (Vec::new(), std::time::Instant::now()), sensors: jc42 })
    }

    /// Each module's temperature (°C), in slot order. While another program
    /// is using the bus, the previous readings stand.
    pub fn read(&mut self) -> Vec<f32> {
        let Some(_lock) = NamedLock::acquire(SMBUS_LOCK, LOCK_WAIT) else {
            return self.standing();
        };
        let readings: Option<Vec<f32>> = self
            .sensors
            .iter()
            .map(|&address| match self.kind {
                // MR49 then MR50, little-endian: an 11-bit two's complement
                // count of quarter degrees in bits 12..2.
                Kind::Spd5118 => transfer(&self.module, address, SPD5118_TEMP, WORD_DATA).map(|raw| spd5118_celsius(raw as u16)),
                // Most significant byte first: a 13-bit two's complement
                // count of sixteenths of a degree.
                Kind::Jc42 => transfer(&self.module, address, JC42_TEMP, WORD_DATA).map(|raw| jc42_celsius((raw as u16).swap_bytes())),
            })
            .collect();
        match readings {
            Some(readings) => {
                self.last = (readings.clone(), std::time::Instant::now());
                readings
            }
            None => self.standing(),
        }
    }

    /// The last readings while they stand; none once they are too old.
    fn standing(&self) -> Vec<f32> {
        let (last, at) = &self.last;
        if at.elapsed() < STALE_AFTER { last.clone() } else { Vec::new() }
    }
}

/// One read transfer: a byte or a word from `register` of the device at
/// `address`. `None` when nothing answers there.
fn transfer(module: &Module, address: u64, register: u64, protocol: u64) -> Option<u64> {
    let mut out = [0u64; 1];
    module.call("ioctl_smbus_xfer", &[address, READ, register, protocol], &mut out).ok()?;
    Some(out[0])
}

fn spd5118_celsius(raw: u16) -> f32 {
    // Sign-extend the 11 bits above the two lowest.
    let count = ((raw as i16) << 3) >> 5;
    count as f32 * 0.25
}

fn jc42_celsius(raw: u16) -> f32 {
    // Sign-extend the low 13 bits.
    let count = ((raw as i16) << 3) >> 3;
    count as f32 / 16.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_temperatures() {
        // 45.25 °C: 181 quarter degrees, shifted past the two low bits.
        assert_eq!(spd5118_celsius(181 << 2), 45.25);
        // -2 °C: -8 quarter degrees, in 11 bits.
        assert_eq!(spd5118_celsius(((-8i16 as u16) & 0x7FF) << 2), -2.0);
        // JC-42: 0x01A2 is 26.125 °C; the top three bits are flags.
        assert_eq!(jc42_celsius(0xE1A2), 26.125);
        assert_eq!(jc42_celsius(0x1FF0), -1.0);
    }

    /// The bus is looked at for the sensors of the modules in the slots alone: DDR5's hubs on a desktop of DDR5
    /// DIMMs, DDR4's sensors on one of DDR4, and nothing on a laptop, soldered memory (the G14's) or a slot of an
    /// SO-DIMM alike.
    #[test]
    fn looks_for_the_sensors_of_the_modules_there_are() {
        use crate::smbios::examples;
        assert_eq!(looked_for(&examples::desktop()), Ok(LookedFor { spd5118: true, jc42: false }));
        assert_eq!(looked_for(&examples::ddr4_desktop()), Ok(LookedFor { spd5118: false, jc42: true }));
        assert_eq!(looked_for(&examples::g14()), Err(Missing::Laptop));
        assert_eq!(looked_for(&examples::so_dimm_laptop()), Err(Missing::Laptop));
        // A desktop whose firmware lists no module in a slot (soldered memory, a mini PC's): nothing.
        let mut soldered = examples::g14();
        soldered[5] = 0x03;
        assert_eq!(looked_for(&soldered), Err(Missing::NoModules));
        assert_eq!(looked_for(&[]), Err(Missing::NoModules));
    }

    /// Run as administrator with `cargo test reads_these_modules -- --nocapture`.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_these_modules() {
        let mut dimms = Dimms::open().expect("no module sensors (administrator needed)");
        println!("{:?}", dimms.read());
    }
}
