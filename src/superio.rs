//! The motherboard's Super I/O chip, which measures the board's temperatures
//! and every fan header's speed. Read through PawnIO's LpcIO module. ITE
//! chips (Gigabyte, ASRock and others), Nuvoton's NCT6779D and NCT679x
//! (ASUS, ASRock, some MSI and Gigabyte boards), and Nuvoton's NCT6683D,
//! NCT6686D and NCT6687D (MSI's boards above all) are supported.

use crate::reading::BoardSensors;
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
/// monitor's I/O space on the NCT6791D and later (set by some firmware,
/// ASUS's among them; cleared as Linux's nct6775 driver clears it).
/// The lock decides only whether the monitor's readings can be reached:
/// fans, voltages and how the board runs are left as they were, and the
/// firmware sets it again at the next start.
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

/// Nuvoton's NCT6683D, NCT6686D and NCT6687D, read as Linux's nct6683
/// driver reads them: through an embedded controller's space, a page and an
/// index written and a byte read at these offsets from the monitor's base
/// (itself 4 past the logical device's address); 32 monitoring channels,
/// each set to a source (a temperature below 0x60, a voltage from there),
/// in 1/256 °C; 16 fan inputs, each in use if its configuration's top bit
/// is, in RPM.
const NUVOTON_EC_IDS: [u16; 3] = [0xC730, 0xD440, 0xD590];
const EC_OFFSET: u64 = 4;
const EC_PAGE: u64 = 0;
const EC_INDEX: u64 = 1;
const EC_DATA: u64 = 2;
const EC_LOGICAL_DEVICE_ENABLE: u64 = 0x30;
const EC_CHANNELS: u16 = 32;
const EC_MONITOR: u16 = 0x100;
const EC_MONITOR_SOURCE: u16 = 0x1A0;
const EC_FAN_RPM: u16 = 0x140;
const EC_FAN_INPUT: u16 = 0x1C0;
const EC_FANS: u16 = 16;
/// Bit 7: monitoring running.
const EC_CONFIG: u16 = 0x180;
const EC_CUSTOMER: u16 = 0x602;
/// The boards' makers whose firmware Linux trusts this way of reading
/// with: Intel, MiTAC, MSI's four, AMD, ASRock's seven.
const EC_CUSTOMERS: [u16; 14] = [0x805, 0xA0E, 0x201, 0x200, 0x207, 0x20D, 0x162B, 0xE2C, 0xE1B, 0x1631, 0x163E, 0x1621, 0x1633, 0x163D];

/// What a monitoring channel's source is, as Glance names inputs: the CPU
/// as the board sees it (AMD's TSI, Intel's PECI, the PCH's view of it),
/// the chipset; none for one to be numbered, or a voltage.
fn ec_source_name(source: u8) -> Option<&'static str> {
    match source {
        0x20..=0x27 | 0x30 | 0x42..=0x49 => Some("cpu"),
        0x31 | 0x33 => Some("chipset"),
        _ => None,
    }
}

/// Whether a channel's source is a temperature (and not disabled, reserved
/// or a voltage).
fn ec_source_is_temperature(source: u8) -> bool {
    matches!(source, 0x01..=0x18 | 0x20..=0x2B | 0x30..=0x41 | 0x42..=0x49 | 0x50..=0x57)
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
    /// How many of the temperature inputs and fan headers below it has.
    Nuvoton { temps: usize, fans: usize },
    /// Read through its embedded controller's space.
    NuvotonEc,
}

pub struct SuperIo {
    module: Module,
    kind: Kind,
    /// The chip's identifier, as it reports it.
    chip: u16,
    layout: Option<&'static Layout>,
    base: u64,
    /// On an embedded controller's chip: the channels that are temperatures
    /// (register, and name), and the fans in use (register, and name).
    ec_temps: Vec<(u16, String)>,
    ec_fans: Vec<(u16, String)>,
    /// The latest readings, kept while another program has the ISA bus.
    last: BoardSensors,
}

impl SuperIo {
    /// What chip was found, where, and whether this board's wiring of it is
    /// known (else its inputs go by the chip's names), for a report.
    pub fn describe(&self) -> String {
        let maker = match self.kind {
            Kind::Ite { .. } => "ITE",
            Kind::Nuvoton { .. } | Kind::NuvotonEc => "Nuvoton",
        };
        let wiring = if self.layout.is_some() { "board layout known" } else { "generic input names" };
        format!("{maker} chip {:04X} at {:#X}, {wiring}", self.chip, self.base)
    }

    /// Finds a supported chip at either configuration port; or says what
    /// stood in the way, for a report (what each port showed).
    /// `vendor` is the board's maker, which decides what the inputs are named.
    pub fn open(vendor: &str) -> Result<Self, String> {
        let mut module = Module::load(LPC_IO).map_err(|_| "PawnIO driver not available".to_string())?;
        // Entering configuration mode writes to the chip: only with the bus to ourselves.
        let _lock = NamedLock::acquire(ISA_LOCK, 1000).ok_or("ISA bus held by another program")?;
        let mut seen = Vec::new();
        for slot in 0..2u64 {
            let port = if slot == 0 { 0x2E } else { 0x4E };
            if module.call("ioctl_select_slot", &[slot], &mut []).is_err() {
                seen.push(format!("{port:#X}: not selectable"));
                continue;
            }
            let found = find_ite(&module, slot, port).or_else(|ite| find_nuvoton(&module, port).map_err(|nuvoton| ite.or(nuvoton)));
            let (kind, chip, base) = match found {
                Ok(found) => found,
                Err(passed) => {
                    seen.push(format!("{port:#X}: {}", passed.unwrap_or_else(|| "nothing".into())));
                    continue;
                }
            };
            // The embedded controller's space sits 4 into an 8-port block.
            let usable = match kind {
                Kind::NuvotonEc => base >= 0x100 && base & 0xF007 == EC_OFFSET,
                _ => base >= 0x100 && base & 0xF007 == 0,
            };
            if !usable {
                seen.push(format!("{port:#X}: chip {chip:04X} with its monitor at {base:#X}, not usable"));
                continue;
            }
            let mut found = SuperIo { module, kind, chip, layout: layout(vendor, chip), base, ec_temps: Vec::new(), ec_fans: Vec::new(), last: BoardSensors::default() };
            let failed = match kind {
                Kind::NuvotonEc => found.set_up_ec().err(),
                // A Nuvoton monitor that answers is one from Nuvoton.
                Kind::Nuvoton { .. } => {
                    let vendor = found.register(NUVOTON_VENDOR.0).zip(found.register(NUVOTON_VENDOR.1));
                    (vendor.map(|(high, low)| (high as u16) << 8 | low as u16) != Some(NUVOTON_VENDOR_ID)).then(|| "its monitor not answering as Nuvoton's".to_string())
                }
                Kind::Ite { .. } => None,
            };
            match failed {
                None => return Ok(found),
                // The other port may hold the chip to read.
                Some(why) => {
                    seen.push(format!("{port:#X}: chip {chip:04X}, {why}"));
                    module = found.module;
                }
            }
        }
        Err(seen.join("; "))
    }

    /// On an embedded controller's chip: checks that the board's maker is
    /// one whose firmware reads this way, starts monitoring if it is not
    /// running, and finds the temperature channels and the fans in use.
    fn set_up_ec(&mut self) -> Result<(), String> {
        let customer = self.register16(EC_CUSTOMER).ok_or("not answering")?;
        if !EC_CUSTOMERS.contains(&customer) {
            return Err(format!("its maker's firmware ({customer:04X}) not known to read this way"));
        }
        let config = self.register(EC_CONFIG).ok_or("not answering")?;
        if config & 0x80 == 0 {
            self.write_ec(EC_CONFIG, config | 0x80).ok_or("monitoring would not start")?;
        }
        let mut numbered = 0;
        for channel in 0..EC_CHANNELS {
            let Some(source) = self.register(EC_MONITOR_SOURCE + channel).map(|s| s & 0x7F) else { continue };
            if !ec_source_is_temperature(source) {
                continue;
            }
            // A second channel of a named source goes by a number, as the rest.
            let name = ec_source_name(source).filter(|name| !self.ec_temps.iter().any(|(_, taken)| taken == name));
            let name = name.map(str::to_string).unwrap_or_else(|| {
                numbered += 1;
                numbered.to_string()
            });
            self.ec_temps.push((EC_MONITOR + channel * 2, name));
        }
        for fan in 0..EC_FANS {
            if self.register(EC_FAN_INPUT + fan).is_some_and(|input| input & 0x80 != 0) {
                self.ec_fans.push((EC_FAN_RPM + fan * 2, (self.ec_fans.len() + 1).to_string()));
            }
        }
        Ok(())
    }

    /// Two registers on an embedded controller's chip, high byte first.
    fn register16(&self, register: u16) -> Option<u16> {
        Some((self.register(register)? as u16) << 8 | self.register(register + 1)? as u16)
    }

    /// Writes a register in an embedded controller's space.
    fn write_ec(&self, register: u16, value: u8) -> Option<()> {
        let out = |port, value: u64| self.module.call("ioctl_pio_outb", &[port, value], &mut []).ok();
        out(self.base + EC_PAGE, 0xFF)?;
        out(self.base + EC_PAGE, (register >> 8) as u64)?;
        out(self.base + EC_INDEX, (register & 0xFF) as u64)?;
        out(self.base + EC_DATA, value as u64).map(|_| ())
    }

    /// A monitor register: the low byte, in the bank of the high byte on a
    /// Nuvoton chip; on an embedded controller's, the page of the high byte
    /// (opened by writing all ones to the page port first).
    fn register(&self, register: u16) -> Option<u8> {
        let out = |port, value: u64| self.module.call("ioctl_pio_outb", &[port, value], &mut []).ok();
        if let Kind::NuvotonEc = self.kind {
            out(self.base + EC_PAGE, 0xFF)?;
            out(self.base + EC_PAGE, (register >> 8) as u64)?;
            out(self.base + EC_INDEX, (register & 0xFF) as u64)?;
            return self.module.read("ioctl_pio_inb", self.base + EC_DATA).ok().map(|v| v as u8);
        }
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
            Kind::NuvotonEc => {
                for (register, name) in &self.ec_temps {
                    // In 1/256 °C, kept to the half degree the chip measures.
                    let celsius = self.register16(*register).map(|raw| (raw as i16 / 128) as f32 / 2.0).filter(|&c| c > 0.0 && c < 127.0);
                    if let Some(value) = celsius {
                        sensors.temps.push((name.clone(), value));
                    }
                }
                for (register, name) in &self.ec_fans {
                    // A stopped fan reads zero.
                    if let Some(rpm) = self.register16(*register).filter(|&rpm| rpm > 0 && rpm < 0xFFFF) {
                        sensors.fans.push((name.clone(), rpm as f32));
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

/// What the search at a port found short of a chip to read: what it was,
/// if there was something to say.
type Passed = Option<String>;

/// Whether a chip identifier is one: an empty port reads all ones or zeros.
fn answered(chip: u16) -> bool {
    chip != 0xFFFF && chip != 0
}

/// An ITE chip at `port`: its kind, its identifier and its monitor's base.
/// Leaves configuration mode however far the search got.
fn find_ite(module: &Module, slot: u64, port: u64) -> Result<(Kind, u16, u64), Passed> {
    let found = (|| {
        // ITE's key into configuration mode; the last byte differs by port.
        for byte in [0x87, 0x01, 0x55, if slot == 0 { 0x55 } else { 0xAA }] {
            module.call("ioctl_pio_outb", &[port, byte], &mut []).map_err(|_| None)?;
        }
        let chip = module.read("ioctl_superio_inw", CHIP_ID).map_err(|_| None)? as u16;
        let Some((temps, fans)) = ite_layout(chip) else {
            // Another maker's chip answers ITE's key with nothing.
            return Err((answered(chip) && chip >> 12 == 0x8).then(|| format!("ITE chip {chip:04X}, not supported")));
        };
        module.call("ioctl_find_bars", &[], &mut []).map_err(|_| None)?;
        module.call("ioctl_superio_outb", &[DEVICE_SELECT, ITE_ENVIRONMENT], &mut []).map_err(|_| None)?;
        let base = module.read("ioctl_superio_inw", BASE_ADDRESS).map_err(|_| None)?;
        Ok((Kind::Ite { temps, fans }, chip, base))
    })();
    let _ = module.call("ioctl_superio_outb", &[CONFIG_CONTROL, 0x02], &mut []);
    found
}

/// A Nuvoton chip at `port`, as `find_ite`. A monitor the firmware has
/// locked is unlocked (see `NUVOTON_IO_LOCK`).
fn find_nuvoton(module: &Module, port: u64) -> Result<(Kind, u16, u64), Passed> {
    let found = (|| {
        // Nuvoton's key into configuration mode.
        for _ in 0..2 {
            module.call("ioctl_pio_outb", &[port, 0x87], &mut []).map_err(|_| None)?;
        }
        let chip = module.read("ioctl_superio_inw", CHIP_ID).map_err(|_| None)? as u16;
        if NUVOTON_EC_IDS.contains(&(chip & 0xFFF0)) {
            module.call("ioctl_find_bars", &[], &mut []).map_err(|_| None)?;
            module.call("ioctl_superio_outb", &[DEVICE_SELECT, NUVOTON_MONITOR], &mut []).map_err(|_| None)?;
            // Left as the firmware set it: a disabled controller is not ours to start.
            let enabled = module.read("ioctl_superio_inb", EC_LOGICAL_DEVICE_ENABLE).map_err(|_| None)? & 0x01 != 0;
            if !enabled {
                return Err(Some(format!("Nuvoton chip {chip:04X}, its embedded controller disabled")));
            }
            let address = module.read("ioctl_superio_inw", BASE_ADDRESS).map_err(|_| None)? & !7;
            return Ok((Kind::NuvotonEc, chip, address + EC_OFFSET));
        }
        let Some((temps, fans)) = nuvoton_layout(chip) else {
            return Err(answered(chip).then(|| format!("chip {chip:04X}, not supported")));
        };
        module.call("ioctl_find_bars", &[], &mut []).map_err(|_| None)?;
        module.call("ioctl_superio_outb", &[DEVICE_SELECT, NUVOTON_MONITOR], &mut []).map_err(|_| None)?;
        // The NCT6779D has no such lock.
        if chip & 0xFFF8 != 0xC560 {
            let lock = module.read("ioctl_superio_inb", NUVOTON_IO_LOCK).map_err(|_| None)?;
            if lock & 0x10 != 0 {
                module.call("ioctl_superio_outb", &[NUVOTON_IO_LOCK, lock & !0x10], &mut []).map_err(|_| None)?;
                let still = module.read("ioctl_superio_inb", NUVOTON_IO_LOCK).map_err(|_| None)? & 0x10 != 0;
                if still {
                    return Err(Some(format!("Nuvoton chip {chip:04X}, its monitor locked by the firmware and staying so")));
                }
            }
        }
        let base = module.read("ioctl_superio_inw", BASE_ADDRESS).map_err(|_| None)?;
        Ok((Kind::Nuvoton { temps, fans }, chip, base))
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
        // The NCT6687D is read through its embedded controller instead.
        assert!(super::NUVOTON_EC_IDS.contains(&(0xD592 & 0xFFF0)));
        assert_eq!(super::nuvoton_layout(0xD592), None);
    }

    #[test]
    fn names_the_embedded_controllers_sources() {
        // AMD's TSI and Intel's PECI are the CPU; thermistors are numbered.
        assert_eq!(super::ec_source_name(0x46), Some("cpu"));
        assert_eq!(super::ec_source_name(0x20), Some("cpu"));
        assert_eq!(super::ec_source_name(0x0B), None);
        assert!(super::ec_source_is_temperature(0x0B));
        // Disabled, reserved and voltages are not temperatures.
        assert!(!super::ec_source_is_temperature(0x00));
        assert!(!super::ec_source_is_temperature(0x19));
        assert!(!super::ec_source_is_temperature(0x60));
    }

    /// Needs administrator rights and the PawnIO driver.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_this_board() {
        let mut chip = super::SuperIo::open("Gigabyte Technology Co., Ltd.").expect("a supported Super I/O chip, elevated");
        println!("{:?}", chip.read());
    }
}
