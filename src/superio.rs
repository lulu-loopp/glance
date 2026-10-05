//! The motherboard's Super I/O chip, which measures the board's temperatures
//! and every fan header's speed. Read through PawnIO's LpcIO module. ITE
//! chips (Gigabyte, ASRock and others) and Nuvoton's NCT6779D and NCT679x
//! (ASUS, ASRock, some MSI and Gigabyte boards) are supported.

use crate::pawnio::Module;
use crate::sensors::{NamedLock, LOCK_WAIT};

const LPC_IO: &[u8] = include_bytes!("../pawnio-modules/LpcIO.bin");

/// Chip registers in configuration mode.
const CHIP_ID: u64 = 0x20;
const DEVICE_SELECT: u64 = 0x07;
const BASE_ADDRESS: u64 = 0x60;
const CONFIG_CONTROL: u64 = 0x02;
/// ITE's environment controller (temperatures, fans, voltages).
const ITE_ENVIRONMENT: u64 = 0x04;
/// Nuvoton's hardware monitor, and its register whose bit 4 locks the
/// monitor's I/O space (set by some firmware; Glance leaves it be).
const NUVOTON_MONITOR: u64 = 0x0B;
const NUVOTON_IO_LOCK: u64 = 0x28;

/// Monitor registers, reached through an address and a data port at these
/// offsets from its base.
const ADDRESS_OFFSET: u64 = 0x05;
const DATA_OFFSET: u64 = 0x06;
const ITE_TEMPERATURE_BASE: u16 = 0x29;
const ITE_FAN_LOW: [u16; 6] = [0x0D, 0x0E, 0x0F, 0x80, 0x82, 0x4C];
const ITE_FAN_HIGH: [u16; 6] = [0x18, 0x19, 0x1A, 0x81, 0x83, 0x4D];
/// Nuvoton's registers are in banks: the bank in the high byte, chosen
/// through the bank register, the register in the low byte. As Linux's
/// nct6775 driver reads these chips: each input's temperature in whole
/// degrees, each fan's speed in RPM (high byte first), and the vendor.
const NUVOTON_BANK: u16 = 0x4E;
const NUVOTON_TEMPERATURES: [(u16, &str); 7] =
    [(0x490, "system"), (0x491, "cpu_socket"), (0x492, "AUXTIN0"), (0x493, "AUXTIN1"), (0x494, "AUXTIN2"), (0x495, "AUXTIN3"), (0x496, "AUXTIN4")];
const NUVOTON_FANS: [u16; 7] = [0x4C0, 0x4C2, 0x4C4, 0x4C6, 0x4C8, 0x4CA, 0x4CE];
const NUVOTON_VENDOR: (u16, u16) = (0x804F, 0x004F);
const NUVOTON_VENDOR_ID: u16 = 0x5CA3;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BoardSensors {
    /// Temperatures by what they measure: a name from the board's layout
    /// (see `layout`), the chip's name for the input, or its number.
    pub temps: Vec<(String, f32)>,
    /// Fan headers that report a speed, named the same way, in RPM.
    pub fans: Vec<(String, f32)>,
}

/// What each of the chip's inputs is wired to on a given board, as the
/// board's maker labels them. Names are keys the page translates.
struct Layout {
    temps: &'static [&'static str],
    fans: &'static [&'static str],
}

/// Gigabyte's AM5 boards with an IT8689E all wire it the same way.
const GIGABYTE_IT8689E: Layout = Layout {
    temps: &["system", "chipset", "cpu_socket", "pcie_x16", "vrm", "vsoc"],
    fans: &["cpu_fan", "system_fan_1", "system_fan_2", "system_fan_3", "system_fan_4_pump", "cpu_opt_fan"],
};

fn layout(vendor: &str, chip: u16) -> Option<&'static Layout> {
    (vendor.starts_with("Gigabyte") && chip == 0x8689).then_some(&GIGABYTE_IT8689E)
}

#[derive(Clone, Copy)]
enum Kind {
    /// With its number of temperature inputs and fan headers.
    Ite { temps: u8, fans: usize },
    /// With its number of fan headers.
    /// How many of the temperature inputs and fan headers below it has.
    Nuvoton { temps: usize, fans: usize },
}

pub struct SuperIo {
    module: Module,
    kind: Kind,
    layout: Option<&'static Layout>,
    base: u64,
    /// The latest readings, kept while another program has the ISA bus.
    last: BoardSensors,
}

impl SuperIo {
    /// Finds a supported chip at either configuration port.
    /// `vendor` is the board's maker, which decides what the inputs are named.
    pub fn open(vendor: &str) -> Option<Self> {
        let module = Module::load(LPC_IO).ok()?;
        // Entering configuration mode writes to the chip: only with the bus to ourselves.
        let _lock = NamedLock::acquire(ISA_LOCK, 1000)?;
        for slot in 0..2u64 {
            module.call("ioctl_select_slot", &[slot], &mut []).ok()?;
            let port = if slot == 0 { 0x2E } else { 0x4E };
            let found = find_ite(&module, slot, port).or_else(|| find_nuvoton(&module, port));
            let Some((kind, chip, base)) = found else { continue };
            if base < 0x100 || base & 0xF007 != 0 {
                continue;
            }
            let chip = SuperIo { module, kind, layout: layout(vendor, chip), base, last: BoardSensors::default() };
            // A Nuvoton monitor that answers is one from Nuvoton.
            if let Kind::Nuvoton { .. } = kind {
                let vendor = chip.register(NUVOTON_VENDOR.0).zip(chip.register(NUVOTON_VENDOR.1));
                if vendor.map(|(high, low)| (high as u16) << 8 | low as u16) != Some(NUVOTON_VENDOR_ID) {
                    return None;
                }
            }
            return Some(chip);
        }
        None
    }

    /// A monitor register: the low byte, in the bank of the high byte on a
    /// Nuvoton chip.
    fn register(&self, register: u16) -> Option<u8> {
        let out = |port, value: u64| self.module.call("ioctl_pio_outb", &[port, value], &mut []).ok();
        if let Kind::Nuvoton { .. } = self.kind {
            out(self.base + ADDRESS_OFFSET, NUVOTON_BANK as u64)?;
            out(self.base + DATA_OFFSET, (register >> 8) as u64)?;
        }
        out(self.base + ADDRESS_OFFSET, (register & 0xFF) as u64)?;
        self.module.read("ioctl_pio_inb", self.base + DATA_OFFSET).ok().map(|v| v as u8)
    }

    /// The board's temperatures and fans. The chip is reached through an
    /// address and a data port, so only under the ISA lock other programs
    /// share; while another program has the bus, the last readings stand.
    pub fn read(&mut self) -> BoardSensors {
        let Some(_lock) = NamedLock::acquire(ISA_LOCK, LOCK_WAIT) else {
            return self.last.clone();
        };
        let mut sensors = BoardSensors::default();
        // Unconnected inputs read 0 or −128 / 127.
        let celsius = |raw: u8| Some(raw as i8).filter(|&c| c > 0 && c < 127).map(f32::from);
        match self.kind {
            Kind::Ite { temps, fans } => {
                for input in 0..temps as usize {
                    if let Some(value) = self.register(ITE_TEMPERATURE_BASE + input as u16).and_then(celsius) {
                        sensors.temps.push((self.name(input, |l| l.temps), value));
                    }
                }
                for fan in 0..fans {
                    let (Some(low), Some(high)) = (self.register(ITE_FAN_LOW[fan]), self.register(ITE_FAN_HIGH[fan])) else {
                        continue;
                    };
                    let count = (high as u32) << 8 | low as u32;
                    // A stopped or absent fan reads all ones.
                    if count > 0 && count < 0xFFFF {
                        sensors.fans.push((self.name(fan, |l| l.fans), 1.35e6 / (count as f32 * 2.0)));
                    }
                }
            }
            Kind::Nuvoton { temps, fans } => {
                for (register, name) in NUVOTON_TEMPERATURES[..temps].iter().copied() {
                    if let Some(value) = self.register(register).and_then(celsius) {
                        sensors.temps.push((name.to_string(), value));
                    }
                }
                for (fan, register) in NUVOTON_FANS[..fans].iter().enumerate() {
                    let (Some(high), Some(low)) = (self.register(*register), self.register(register + 1)) else {
                        continue;
                    };
                    let rpm = (high as u32) << 8 | low as u32;
                    // A stopped or absent fan reads zero.
                    if rpm > 0 && rpm < 0xFFFF {
                        sensors.fans.push(((fan + 1).to_string(), rpm as f32));
                    }
                }
            }
        }
        self.last = sensors.clone();
        sensors
    }

    /// The input's name on this board, or its number from 1.
    fn name(&self, index: usize, names: fn(&Layout) -> &'static [&'static str]) -> String {
        self.layout
            .and_then(|layout| names(layout).get(index))
            .map_or_else(|| (index + 1).to_string(), |name| name.to_string())
    }
}

const ISA_LOCK: windows::core::PCWSTR = windows::core::w!("Global\\Access_ISABUS.HTP.Method");

/// An ITE chip at `port`: its kind, its identifier and its monitor's base.
/// Leaves configuration mode however far the search got.
fn find_ite(module: &Module, slot: u64, port: u64) -> Option<(Kind, u16, u64)> {
    let found = (|| {
        // ITE's key into configuration mode; the last byte differs by port.
        for byte in [0x87, 0x01, 0x55, if slot == 0 { 0x55 } else { 0xAA }] {
            module.call("ioctl_pio_outb", &[port, byte], &mut []).ok()?;
        }
        let chip = module.read("ioctl_superio_inw", CHIP_ID).ok()? as u16;
        let (temps, fans) = ite_layout(chip)?;
        module.call("ioctl_find_bars", &[], &mut []).ok()?;
        module.call("ioctl_superio_outb", &[DEVICE_SELECT, ITE_ENVIRONMENT], &mut []).ok()?;
        let base = module.read("ioctl_superio_inw", BASE_ADDRESS).ok()?;
        Some((Kind::Ite { temps, fans }, chip, base))
    })();
    let _ = module.call("ioctl_superio_outb", &[CONFIG_CONTROL, 0x02], &mut []);
    found
}

/// A Nuvoton chip at `port`, as `find_ite`. One whose monitor the firmware
/// has locked is left alone: unlocking it would change how the board is set up.
fn find_nuvoton(module: &Module, port: u64) -> Option<(Kind, u16, u64)> {
    let found = (|| {
        // Nuvoton's key into configuration mode.
        for _ in 0..2 {
            module.call("ioctl_pio_outb", &[port, 0x87], &mut []).ok()?;
        }
        let chip = module.read("ioctl_superio_inw", CHIP_ID).ok()? as u16;
        let (temps, fans) = nuvoton_layout(chip)?;
        module.call("ioctl_find_bars", &[], &mut []).ok()?;
        module.call("ioctl_superio_outb", &[DEVICE_SELECT, NUVOTON_MONITOR], &mut []).ok()?;
        let locked = module.read("ioctl_superio_inb", NUVOTON_IO_LOCK).ok()? & 0x10 != 0;
        let base = module.read("ioctl_superio_inw", BASE_ADDRESS).ok()?;
        (!locked).then_some((Kind::Nuvoton { temps, fans }, chip, base))
    })();
    // Nuvoton's key out of configuration mode.
    let _ = module.call("ioctl_pio_outb", &[port, 0xAA], &mut []);
    found
}

/// Temperature inputs and fan headers of the ITE environment controllers
/// that report 16-bit fan counts in this register layout.
fn ite_layout(chip: u16) -> Option<(u8, usize)> {
    Some(match chip {
        0x8686 | 0x8688 | 0x8689 | 0x8696 => (6, 6),
        0x8620 | 0x8628 | 0x8625 => (6, 5),
        0x8613 | 0x8631 | 0x8638 => (3, 3),
        0x8790 | 0x8792 | 0x8695 => (3, 3),
        0x8665 | 0x8655 => (6, 3),
        0x8721 | 0x8728 | 0x8771 | 0x8772 => (3, 3),
        _ => return None,
    })
}

/// Temperature inputs and fan headers of the Nuvoton chips read here,
/// whose monitor keeps the registers above. The identifier's low three bits
/// are the revision (as Linux's nct6775 driver reads them).
fn nuvoton_layout(chip: u16) -> Option<(usize, usize)> {
    Some(match chip & 0xFFF8 {
        // NCT6779D: up to AUXTIN3.
        0xC560 => (6, 5),
        // NCT6791D, NCT6792D, NCT6793D, NCT6795D.
        0xC800 | 0xC910 | 0xD120 | 0xD350 => (6, 6),
        // NCT6796D, which adds AUXTIN4.
        0xD420 => (7, 6),
        // NCT6797D, NCT6798D, NCT6799D.
        0xD450 | 0xD428 | 0xD800 => (7, 7),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn knows_nuvoton_chips() {
        assert_eq!(super::nuvoton_layout(0xD42B), Some((7, 7)));
        // NCT6798D's other revisions, and NCT6796D's beside them.
        assert_eq!(super::nuvoton_layout(0xD428), Some((7, 7)));
        assert_eq!(super::nuvoton_layout(0xD423), Some((7, 6)));
        assert_eq!(super::nuvoton_layout(0xC562), Some((6, 5)));
        assert_eq!(super::nuvoton_layout(0x8689), None);
    }

    /// Needs administrator rights and the PawnIO driver.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_this_board() {
        let mut chip = super::SuperIo::open("Gigabyte Technology Co., Ltd.").expect("a supported Super I/O chip, elevated");
        println!("{:?}", chip.read());
    }
}
