//! The memory modules' own temperature sensors, on the SMBus: the SPD5118
//! hub of each DDR5 module, or the JC-42.4 sensor of a DDR4 one. Read
//! through PawnIO's SMBus modules, the way Linux's spd5118 and jc42 drivers
//! read them; nothing is ever written to a module.

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
    /// The latest readings, kept while another program holds the bus.
    last: Vec<f32>,
}

impl Dimms {
    /// Finds the modules' sensors, if the chipset's SMBus can be reached.
    pub fn open() -> Option<Self> {
        let module = Module::load(SMBUS_PIIX4).or_else(|_| Module::load(SMBUS_I801)).ok()?;
        let _lock = NamedLock::acquire(SMBUS_LOCK, 1000)?;
        let byte = |address, register| transfer(&module, address, register, BYTE_DATA);
        // A DDR5 hub names its type in its first two registers, and its
        // sensor is in use only when not switched off.
        let spd5118: Vec<u64> = SPD5118_ADDRESSES
            .filter(|&address| byte(address, SPD5118_TYPE.0) == Some(0x51) && byte(address, SPD5118_TYPE.1) == Some(0x18))
            .filter(|&address| byte(address, SPD5118_TEMP_CONFIG).is_some_and(|config| config & 1 == 0))
            .collect();
        if !spd5118.is_empty() {
            return Some(Dimms { module, kind: Kind::Spd5118, last: Vec::new(), sensors: spd5118 });
        }
        // A DDR4 sensor: as Linux's jc42 checks, reserved bits clear and a
        // manufacturer set.
        let word = |address, register| transfer(&module, address, register, WORD_DATA).map(|w| (w as u16).swap_bytes());
        let jc42: Vec<u64> = JC42_ADDRESSES
            .filter(|&address| {
                let (Some(capability), Some(config), Some(maker)) =
                    (word(address, JC42_CAPABILITY), word(address, JC42_CONFIG), word(address, JC42_MANUFACTURER))
                else {
                    return false;
                };
                capability & 0xFE00 == 0 && config & 0xF800 == 0 && maker != 0 && maker != 0xFFFF
            })
            .collect();
        (!jc42.is_empty()).then(|| Dimms { module, kind: Kind::Jc42, last: Vec::new(), sensors: jc42 })
    }

    /// Each module's temperature (°C), in slot order. While another program
    /// is using the bus, the previous readings stand.
    pub fn read(&mut self) -> Vec<f32> {
        let Some(_lock) = NamedLock::acquire(SMBUS_LOCK, LOCK_WAIT) else {
            return self.last.clone();
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
        if let Some(readings) = readings {
            self.last = readings;
        }
        self.last.clone()
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

    /// Run as administrator with `cargo test reads_these_modules -- --nocapture`.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_these_modules() {
        let mut dimms = Dimms::open().expect("no module sensors (administrator needed)");
        println!("{:?}", dimms.read());
    }
}
