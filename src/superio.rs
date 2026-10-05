//! The motherboard's Super I/O chip, which measures the board's temperatures
//! and every fan header's speed. Read through PawnIO's LpcIO module. ITE
//! chips (Gigabyte, ASRock and others) are supported.

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

/// Environment controller registers, reached through an address and a data
/// port at these offsets from its base.
const ADDRESS_OFFSET: u64 = 0x05;
const DATA_OFFSET: u64 = 0x06;
const ITE_TEMPERATURE_BASE: u8 = 0x29;
const ITE_FAN_LOW: [u8; 6] = [0x0D, 0x0E, 0x0F, 0x80, 0x82, 0x4C];
const ITE_FAN_HIGH: [u8; 6] = [0x18, 0x19, 0x1A, 0x81, 0x83, 0x4D];

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BoardSensors {
    /// Temperatures by what they measure: a name from the board's layout
    /// (see `layout`), or the input's number when the board is unknown.
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

pub struct SuperIo {
    module: Module,
    layout: Option<&'static Layout>,
    base: u64,
    temp_count: u8,
    fan_count: usize,
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
            // ITE's key into configuration mode; the last byte differs by port.
            for byte in [0x87, 0x01, 0x55, if slot == 0 { 0x55 } else { 0xAA }] {
                module.call("ioctl_pio_outb", &[port, byte], &mut []).ok()?;
            }
            let chip = module.read("ioctl_superio_inw", CHIP_ID).ok()? as u16;
            let Some((temp_count, fan_count)) = ite_layout(chip) else {
                exit(&module);
                continue;
            };
            module.call("ioctl_find_bars", &[], &mut []).ok()?;
            module.call("ioctl_superio_outb", &[DEVICE_SELECT, ITE_ENVIRONMENT], &mut []).ok()?;
            let base = module.read("ioctl_superio_inw", BASE_ADDRESS).ok()?;
            exit(&module);
            if base < 0x100 || base & 0xF007 != 0 {
                continue;
            }
            return Some(SuperIo { module, layout: layout(vendor, chip), base, temp_count, fan_count, last: BoardSensors::default() });
        }
        None
    }

    fn register(&self, register: u8) -> Option<u8> {
        self.module.call("ioctl_pio_outb", &[self.base + ADDRESS_OFFSET, register as u64], &mut []).ok()?;
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
        for input in 0..self.temp_count {
            if let Some(raw) = self.register(ITE_TEMPERATURE_BASE + input) {
                let celsius = raw as i8;
                // Unconnected inputs read 0 or −128 / 127.
                if celsius > 0 && celsius < 127 {
                    sensors.temps.push((self.name(input as usize, |l| l.temps), celsius as f32));
                }
            }
        }
        for fan in 0..self.fan_count {
            let (Some(low), Some(high)) =
                (self.register(ITE_FAN_LOW[fan]), self.register(ITE_FAN_HIGH[fan]))
            else {
                continue;
            };
            let count = (high as u32) << 8 | low as u32;
            // A stopped or absent fan reads all ones.
            if count > 0 && count < 0xFFFF {
                sensors.fans.push((self.name(fan, |l| l.fans), 1.35e6 / (count as f32 * 2.0)));
            }
        }
        self.last = sensors.clone();
        sensors
    }
}

impl SuperIo {
    /// The input's name on this board, or its number from 1.
    fn name(&self, index: usize, names: fn(&Layout) -> &'static [&'static str]) -> String {
        self.layout
            .and_then(|layout| names(layout).get(index))
            .map_or_else(|| (index + 1).to_string(), |name| name.to_string())
    }
}

const ISA_LOCK: windows::core::PCWSTR = windows::core::w!("Global\\Access_ISABUS.HTP.Method");

fn exit(module: &Module) {
    let _ = module.call("ioctl_superio_outb", &[CONFIG_CONTROL, 0x02], &mut []);
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

#[cfg(test)]
mod tests {
    /// Needs administrator rights and the PawnIO driver.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_this_board() {
        let mut chip = super::SuperIo::open("Gigabyte Technology Co., Ltd.").expect("a supported Super I/O chip, elevated");
        println!("{:?}", chip.read());
    }
}
