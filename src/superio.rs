//! The motherboard's Super I/O chip, which measures the board's temperatures
//! and every fan header's speed. Read through PawnIO's LpcIO module. ITE
//! chips (Gigabyte, ASRock and others), Nuvoton's NCT6779D and NCT679x
//! (ASUS, ASRock, some MSI and Gigabyte boards), and Nuvoton's NCT6683D,
//! NCT6686D and NCT6687D (MSI's boards above all) are supported.

use crate::reading::BoardSensors;
use crate::pawnio::Module;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::sensors::{NamedLock, LOCK_WAIT};
use windows::Win32::System::Power::POWER_PLATFORM_ROLE;

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

/// Gigabyte's 800-series AM5 boards with an IT8696E wire its first five
/// temperature inputs and fan headers as the IT8689E boards do, the fifth
/// header aside; what the rest are wired to differs from board to board,
/// and they keep their numbers (as LibreHardwareMonitor's tables of those
/// boards have them).
const GIGABYTE_IT8696E: Layout = Layout {
    temps: &["system", "chipset", "cpu_socket", "pcie_x16", "vrm"],
    fans: &["cpu_fan", "system_fan_1", "system_fan_2", "system_fan_3", "cpu_opt_fan"],
};

/// Gigabyte's B760M GAMING WIFI6 PLUS GEN5, an Intel board with an IT8689E, wires its first five temperature inputs
/// and its first four fan headers as the AM5 boards do, and its sixth input to an auxiliary sensor (as AIDA64 named each
/// of them, read beside Glance on that board, its second header turning as its first case fan; the third and fourth the
/// case fans after it, as its owner told). Its other fan headers keep their numbers, what they are wired to not seen.
const GIGABYTE_B760M_GAMING_WIFI6_PLUS_GEN5: Layout = Layout {
    temps: &["system", "chipset", "cpu_socket", "pcie_x16", "vrm", "aux"],
    fans: &["cpu_fan", "system_fan_1", "system_fan_2", "system_fan_3"],
};

/// MSI's boards with an NCT6687D wire its first seven monitoring channels
/// and its eight fan inputs the same way (as Linux's nct6687d driver for
/// those boards and LibreHardwareMonitor name them): by the channel's and
/// the input's own number, not by which of them are in use.
const MSI_NCT6687D: Layout = Layout {
    temps: &["cpu", "system", "vrm", "chipset", "cpu_socket", "pcie_x1", "m2_1"],
    fans: &["cpu_fan", "pump_fan", "system_fan_1", "system_fan_2", "system_fan_3", "system_fan_4", "system_fan_5", "system_fan_6"],
};

/// What the inputs of the chip `chip` are wired to on the board `board` made by `vendor`, where that is known.
/// (Gigabyte's by the board's socket: its Intel boards with the same chips wire them otherwise, and were shown the AM5
/// boards' names. An Intel board is named only where how it wires them was seen on it.)
fn layout(vendor: &str, board: &str, chip: u16) -> Option<&'static Layout> {
    if vendor.starts_with("Gigabyte") {
        return match chip {
            0x8689 if am5(board) => Some(&GIGABYTE_IT8689E),
            0x8689 if board.eq_ignore_ascii_case("B760M GAMING WIFI6 PLUS GEN5") => Some(&GIGABYTE_B760M_GAMING_WIFI6_PLUS_GEN5),
            0x8696 if am5(board) => Some(&GIGABYTE_IT8696E),
            _ => None,
        };
    }
    // (The NCT6687D, whatever its revision.)
    if vendor.starts_with("Micro-Star") && chip & 0xFFF0 == 0xD590 {
        return Some(&MSI_NCT6687D);
    }
    None
}

/// Whether `board` (its maker's name for it: "X870E AORUS MASTER", "B650M DS3H") is an AM5 board: by its chipset,
/// which leads its name.
fn am5(board: &str) -> bool {
    let chipset = board.split_whitespace().next().unwrap_or_default().to_ascii_uppercase();
    ["A620", "B650", "X670", "B840", "B850", "X870"].iter().any(|am5| chipset.starts_with(am5))
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
    /// The configuration port it was found at (0x2E, 0x4E), and its slot as the driver numbers them.
    port: u64,
    slot: u64,
    /// On an embedded controller's chip: the channels that are temperatures
    /// (register, and name), and the fans in use (register, and name).
    ec_temps: Vec<(u16, String)>,
    ec_fans: Vec<(u16, String)>,
    /// The latest readings and when they were read, kept while another program has the ISA bus (no longer than
    /// `STALE_AFTER`).
    last: (BoardSensors, Instant),
    /// The fans seen turning since it was found, by their number (a bit each): one of them read as stopped is
    /// stopped, 0 RPM (as its firmware stops it, cool); one never seen turning reads as stopped as an empty header
    /// does, and is none.
    spun: u32,
}

/// How long the last readings stand for the board's while another program has the bus: after, none (they would show
/// a reading frozen as if live).
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

impl SuperIo {
    /// What chip was found, where, and whether this board's wiring of it is
    /// known (else its inputs go by the chip's names), for a report.
    pub fn describe(&self) -> String {
        let maker = match self.kind {
            Kind::Ite { .. } => "ITE",
            Kind::Nuvoton { .. } | Kind::NuvotonEc => "Nuvoton",
        };
        let wiring = if self.layout.is_some() { "board layout known" } else { "generic input names" };
        let raw = self.raw().map(|raw| format!("; raw {raw}")).unwrap_or_default();
        format!("{maker} chip {:04X} at {:#X}, {wiring}{raw}", self.chip, self.base)
    }

    /// A Nuvoton monitor's registers as they read, for a report: where a
    /// board gives no reading, they say whether its inputs are unconnected,
    /// its values elsewhere, or nothing answering. Each run of registers by
    /// its first, in hex; `--` for one that could not be read.
    fn raw(&self) -> Option<String> {
        let _lock = NamedLock::acquire(ISA_LOCK, LOCK_WAIT)?;
        // On an embedded controller's chip: each monitoring channel that is set to anything (its number, what
        // it is of, what it reads) and each fan input (its number, how it is set, its speed): which is which
        // on a board whose wiring is not known shows by them.
        if let Kind::NuvotonEc = self.kind {
            let hex = |value: Option<u16>, digits: usize| value.map_or("-".repeat(digits), |value| format!("{value:0digits$X}"));
            let channels: Vec<String> = (0..EC_CHANNELS)
                .filter_map(|channel| {
                    let source = self.register(EC_MONITOR_SOURCE + channel)?;
                    (source & 0x7F != 0).then(|| format!("{channel}:{source:02X}={}", hex(self.register16(EC_MONITOR + channel * 2), 4)))
                })
                .collect();
            let fans: Vec<String> = (0..EC_FANS).map(|fan| format!("{fan}:{}={}", hex(self.register(EC_FAN_INPUT + fan).map(u16::from), 2), hex(self.register16(EC_FAN_RPM + fan * 2), 4))).collect();
            return Some(format!("channels {} | fans {}", channels.join(" "), fans.join(" ")));
        }
        let Kind::Nuvoton { .. } = self.kind else { return None };
        let run = |from: u16, count: u16| {
            let bytes: Vec<String> = (from..from + count).map(|register| self.register(register).map_or("--".to_string(), |value| format!("{value:02X}"))).collect();
            format!("{from:03X}: {}", bytes.join(" "))
        };
        // The temperatures by fixed input and the fans' speeds (as read here); the temperatures the chip itself
        // monitors, by source, with which source each is of; the fans' counts; and whether monitoring runs. Then
        // what tells how the board wires its inputs: each input's kind (a diode, a thermistor: 0x05D, 0x05E), the
        // sources the fans are run by (0x200, 0x300, 0x800, 0x900, 0xA00) and the inputs' own selects (0x621, 0x622).
        Some(
            [run(0x490, 7), run(0x4C0, 16), run(0x027, 1), run(0x073, 12), run(0x100, 8), run(0x150, 2), run(0x4B0, 14), run(0x040, 1), run(0x05D, 2), run(0x200, 1), run(0x300, 1), run(0x800, 1), run(0x900, 1), run(0xA00, 1), run(0x621, 2)]
                .join(" | "),
        )
    }

    /// Finds a supported chip at either configuration port; or says what
    /// stood in the way, for a report (what each port showed).
    /// `vendor` and `board` are the board's maker and its name for it, which decide what the inputs are named.
    pub fn open(vendor: &str, board: &str) -> Result<Self, String> {
        if !probed(&crate::smbios::table(), crate::laptop::role()) {
            return Err(ON_A_LAPTOP.into());
        }
        let mut module = Module::load(LPC_IO).map_err(|_| "PawnIO driver not available".to_string())?;
        // Entering configuration mode writes to the chip: only with the bus to ourselves.
        let _lock = NamedLock::taken(ISA_LOCK, 1000).map_err(|why| format!("{BUS_NOT_HAD}: {why}"))?;
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
            let mut found = SuperIo { module, kind, chip, layout: layout(vendor, board, chip), base, port, slot, ec_temps: Vec::new(), ec_fans: Vec::new(), last: (BoardSensors::default(), Instant::now()), spun: 0 };
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
            // As the board wires it, where that is known; else by what it is of (a second channel of a named
            // source by a number, as the rest).
            let wired = self.layout.and_then(|layout| layout.temps.get(channel as usize)).filter(|name| !self.ec_temps.iter().any(|(_, taken)| taken == *name));
            let name = wired.copied().or_else(|| ec_source_name(source).filter(|name| !self.ec_temps.iter().any(|(_, taken)| taken == name)));
            let name = name.map(str::to_string).unwrap_or_else(|| {
                numbered += 1;
                numbered.to_string()
            });
            self.ec_temps.push((EC_MONITOR + channel * 2, name));
        }
        for fan in 0..EC_FANS {
            if self.register(EC_FAN_INPUT + fan).is_some_and(|input| input & 0x80 != 0) {
                // (By the input's own number, as the board's maker counts its headers: not by how many are in use.)
                let name = self.layout.and_then(|layout| layout.fans.get(fan as usize)).map_or_else(|| (fan + 1).to_string(), |name| name.to_string());
                self.ec_fans.push((EC_FAN_RPM + fan * 2, name));
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
    /// share, waited for `patience` ms at most; while another program has
    /// the bus, the last readings stand.
    pub fn read(&mut self, patience: u32) -> BoardSensors {
        READS.fetch_add(1, Ordering::Relaxed);
        let Some(_lock) = NamedLock::acquire(ISA_LOCK, patience) else {
            MISSED.fetch_add(1, Ordering::Relaxed);
            let (last, at) = &self.last;
            return if at.elapsed() < STALE_AFTER { last.clone() } else { BoardSensors::default() };
        };
        let mut sensors = BoardSensors::default();
        let celsius = |raw: u8| plausible(f32::from(raw as i8));
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
                    // A stopped or absent fan reads all ones (stopped, once seen turning: see `spun`).
                    if count > 0 && count < 0xFFFF {
                        self.spun |= 1 << fan;
                        sensors.fans.push((self.name(fan, |l| l.fans), 1.35e6 / (count as f32 * 2.0)));
                    } else if count == 0xFFFF && self.spun & 1 << fan != 0 {
                        sensors.fans.push((self.name(fan, |l| l.fans), 0.0));
                    }
                }
            }
            Kind::NuvotonEc => {
                for (register, name) in &self.ec_temps {
                    // In 1/256 °C, kept to the half degree the chip measures.
                    let celsius = self.register16(*register).map(|raw| (raw as i16 / 128) as f32 / 2.0).and_then(plausible);
                    if let Some(value) = celsius {
                        sensors.temps.push((name.clone(), value));
                    }
                }
                for (register, name) in &self.ec_fans {
                    // A stopped fan reads zero, and is shown so (its header is in use: see `ec_fans`); all ones is
                    // no reading.
                    if let Some(rpm) = self.register16(*register).filter(|&rpm| rpm < 0xFFFF) {
                        sensors.fans.push((name.clone(), rpm as f32));
                    }
                }
            }
            Kind::Nuvoton { temps, fans } => {
                // Its monitor no longer answering as Nuvoton's, another program has locked it again (ASUS's own
                // software does, on its boards: every register then reads all ones, which reads as nothing
                // connected): unlocked again, as it was found.
                if !self.answers() && !self.unlocked_again() {
                    self.last = (sensors.clone(), Instant::now());
                    return sensors;
                }
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
                    // A stopped or absent fan reads zero (stopped, once seen turning: see `spun`).
                    if rpm > 0 && rpm < 0xFFFF {
                        self.spun |= 1 << fan;
                        sensors.fans.push(((fan + 1).to_string(), rpm as f32));
                    } else if rpm == 0 && self.spun & 1 << fan != 0 {
                        sensors.fans.push(((fan + 1).to_string(), 0.0));
                    }
                }
            }
        }
        self.last = (sensors.clone(), Instant::now());
        sensors
    }

    /// Whether a Nuvoton monitor answers as Nuvoton's.
    fn answers(&self) -> bool {
        let vendor = self.register(NUVOTON_VENDOR.0).zip(self.register(NUVOTON_VENDOR.1));
        vendor.map(|(high, low)| (high as u16) << 8 | low as u16) == Some(NUVOTON_VENDOR_ID)
    }

    /// Unlocks a Nuvoton monitor locked again since it was found (under the bus's lock, held by the caller), as it
    /// was then; whether it answers now. Said once in the log each time it is found locked after answering, and
    /// counted for a report.
    fn unlocked_again(&mut self) -> bool {
        let found = self.module.call("ioctl_select_slot", &[self.slot], &mut []).ok().and_then(|_| find_nuvoton(&self.module, self.port).ok());
        if let Some((Kind::Nuvoton { .. }, _, base)) = found.filter(|(_, chip, _)| *chip == self.chip) {
            self.base = base;
        }
        let answers = self.answers();
        if answers {
            RELOCKED.fetch_add(1, Ordering::Relaxed);
            if RELOCKED.load(Ordering::Relaxed) <= 3 {
                crate::journal::note(format!("board chip: its monitor was locked again by another program, unlocked (at {:#X})", self.base));
            }
        }
        answers
    }

    /// The input's name on this board, or its number from 1.
    fn name(&self, index: usize, names: fn(&Layout) -> &'static [&'static str]) -> String {
        self.layout
            .and_then(|layout| names(layout).get(index))
            .map_or_else(|| (index + 1).to_string(), |name| name.to_string())
    }
}

const ISA_LOCK: windows::core::PCWSTR = windows::core::w!("Global\\Access_ISABUS.HTP.Method");

/// How many times the chip was read, and how many of those the bus was another program's all the while it was
/// waited for (the readings then stood as they were), for a report.
static READS: AtomicU64 = AtomicU64::new(0);
static MISSED: AtomicU64 = AtomicU64::new(0);
/// How many times a Nuvoton monitor was found locked again by another program, and unlocked.
static RELOCKED: AtomicU64 = AtomicU64::new(0);

pub fn reads() -> (u64, u64, u64) {
    (READS.load(Ordering::Relaxed), MISSED.load(Ordering::Relaxed), RELOCKED.load(Ordering::Relaxed))
}

/// How often the chip is read, and how long the bus is waited for each time (other programs that monitor the
/// board, or light it, take it too, some for long stretches).
const EVERY: std::time::Duration = std::time::Duration::from_secs(1);
const PATIENCE: u32 = 900;

/// A chip read on a thread of its own, as often as a sample is taken: a wait for the bus there holds up nothing
/// else. What it read last is what is asked for.
pub struct Watched {
    latest: Arc<Mutex<BoardSensors>>,
    stop: Arc<AtomicBool>,
}

impl Watched {
    pub fn new(mut chip: SuperIo) -> Self {
        let (latest, stop) = (Arc::new(Mutex::new(BoardSensors::default())), Arc::new(AtomicBool::new(false)));
        let (kept, stopped) = (latest.clone(), stop.clone());
        std::thread::Builder::new().name("glance: board chip".into()).spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let began = std::time::Instant::now();
                let read = chip.read(PATIENCE);
                *kept.lock().unwrap() = read;
                std::thread::sleep(EVERY.saturating_sub(began.elapsed()));
            }
        }).expect("a thread started");
        Watched { latest, stop }
    }

    pub fn read(&self) -> BoardSensors {
        self.latest.lock().unwrap().clone()
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// How it is said that the bus was not to be had as the chip was looked
/// for: another program had it then, and the chip is looked for again.
const BUS_NOT_HAD: &str = "the ISA bus not to be had";

/// Whether `why` a chip was not found is that the bus was not to be had.
pub fn waits_for_the_bus(why: &str) -> bool {
    why.starts_with(BUS_NOT_HAD)
}

/// What a later look for the chip found, for a report (what the first
/// found is said in what the machine was found to have, as Glance started).
static LATER: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn found_later() -> Option<String> {
    LATER.lock().unwrap().clone()
}

pub fn say_later(what: String) {
    *LATER.lock().unwrap() = Some(what);
}

/// Whether the configuration ports are probed on the machine whose firmware's table is `table` and whose power role
/// is `role`: not on a laptop (see `laptop::laptop`). A
/// laptop's embedded controller (ITE's, as a rule) answers at those ports, and the keys written there to put a chip
/// into its configuration mode (and to take it out, found or not) reach it. Its firmware runs the keyboard, the fans
/// and the power through it, and a write it does not look for may leave it stuck till the power is cut. A laptop's
/// fans are had from its maker's firmware instead (see `asus`, `lenovo`).
fn probed(table: &[u8], role: POWER_PLATFORM_ROLE) -> bool {
    !crate::laptop::laptop(&crate::smbios::chassis(table), role)
}

/// How it is said that the chip was not looked for, on a laptop.
const ON_A_LAPTOP: &str = "not looked for on a laptop (its embedded controller may answer where the chip would)";

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

/// The hottest a board's sensor reads of what it is put by: no part a board measures runs hotter (a processor stops
/// at 105 °C at most, its chipset well before).
const HOTTEST: f32 = 110.0;

/// A board's temperature as read, if an input with something on it reads so. Unconnected inputs read 0 or −128 / 127,
/// or, with nothing on them to pull them down (as on MAXSUN's and ASUS's boards of Nuvoton chips), 111 to 125 °C: none
/// past what the board's parts run at.
fn plausible(celsius: f32) -> Option<f32> {
    Some(celsius).filter(|&c| c > 0.0 && c <= HOTTEST)
}

#[cfg(test)]
mod tests {
    /// Inputs with nothing on them are not shown: 0, −128, 127, nor the 111 to 125 °C of those left floating (as two
    /// boards' reports had them); a board's parts at their hottest are.
    #[test]
    fn floating_inputs_are_not_shown() {
        for none in [0.0, -128.0, 127.0, 111.0, 118.5, 125.0] {
            assert_eq!(super::plausible(none), None, "{none}");
        }
        for some in [0.5, 34.0, 95.0, 105.0, 110.0] {
            assert_eq!(super::plausible(some), Some(some));
        }
    }

    /// Gigabyte's inputs named by the board's socket: an Intel board with the same chip gets no AM5 board's names.
    #[test]
    fn names_gigabytes_inputs_by_its_board() {
        let gigabyte = "Gigabyte Technology Co., Ltd.";
        assert!(super::layout(gigabyte, "X870E AORUS MASTER", 0x8696).is_some());
        assert!(super::layout(gigabyte, "B650M DS3H", 0x8689).is_some());
        assert!(super::layout(gigabyte, "Z790 AORUS ELITE AX", 0x8689).is_none());
        assert!(super::layout("Micro-Star International Co., Ltd.", "MAG B650 TOMAHAWK WIFI (MS-7D75)", 0xD592).is_some());
    }

    /// The B760M GAMING WIFI6 PLUS GEN5's inputs shown as AIDA64 named them on that board: its sixth an auxiliary
    /// sensor, not the SoC's supply an AM5 board has there, and its fan headers the CPU's and the case's in turn (its
    /// second, shown as "风扇 2", AIDA64's first case fan). Gigabyte's other Intel boards with the same chip keep the
    /// inputs' numbers.
    #[test]
    fn names_the_b760m_gaming_wifi6_plus_gen5s_inputs() {
        use crate::ui::text::Lang;
        use crate::ui::view::board_sensor;
        let gigabyte = "Gigabyte Technology Co., Ltd.";
        let board = super::layout(gigabyte, "B760M GAMING WIFI6 PLUS GEN5", 0x8689).expect("the B760M GAMING WIFI6 PLUS GEN5's wiring");
        let shown = |lang| {
            let temps: Vec<String> = board.temps.iter().map(|name| board_sensor(name, "temps", lang)).collect();
            let fans: Vec<String> = board.fans.iter().map(|name| board_sensor(name, "fans", lang)).collect();
            (temps, fans)
        };
        let (temps, fans) = shown(Lang::Zh);
        assert_eq!(temps, ["系统", "芯片组", "CPU", "PCIe x16 插槽", "VRM 供电", "辅助传感器"]);
        assert_eq!(fans, ["CPU 风扇", "系统风扇 1", "系统风扇 2", "系统风扇 3"]);
        let (temps, fans) = shown(Lang::En);
        assert_eq!(temps, ["System", "Chipset", "CPU", "PCIe x16 slot", "VRM", "Auxiliary sensor"]);
        assert_eq!(fans, ["CPU fan", "System fan 1", "System fan 2", "System fan 3"]);
        for other in ["B760M GAMING X AX", "B760M AORUS ELITE AX", "Z790 AORUS ELITE AX", "B660M GAMING X DDR4"] {
            assert!(super::layout(gigabyte, other, 0x8689).is_none(), "{other}");
        }
        assert!(super::layout(gigabyte, "B760M GAMING WIFI6 PLUS GEN5", 0x8696).is_none());
    }

    /// Every input a known board's wiring names has a name in both languages (a key with none would be shown as it
    /// is spelled).
    #[test]
    fn every_wired_input_has_a_name() {
        use crate::ui::text::Lang;
        let layouts = [&super::GIGABYTE_IT8689E, &super::GIGABYTE_IT8696E, &super::GIGABYTE_B760M_GAMING_WIFI6_PLUS_GEN5, &super::MSI_NCT6687D];
        for key in layouts.iter().flat_map(|layout| layout.temps.iter().chain(layout.fans.iter())) {
            for lang in [Lang::Zh, Lang::En] {
                assert_ne!(lang.name(key), *key, "{key}");
            }
        }
    }

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

    #[test]
    fn knows_the_boards_whose_wiring_it_has() {
        // MSI's NCT6687D, whatever its revision, by the channel's and the input's own number.
        let msi = super::layout("Micro-Star International Co., Ltd.", "MAG X670E TOMAHAWK WIFI (MS-7E12)", 0xD592).expect("MSI's NCT6687D");
        assert_eq!((msi.temps[1], msi.temps[4], msi.fans[0], msi.fans[2]), ("system", "cpu_socket", "cpu_fan", "system_fan_1"));
        assert!(super::layout("Micro-Star International Co., Ltd.", "MAG X670E TOMAHAWK WIFI (MS-7E12)", 0xD42B).is_none());
        assert!(super::layout("Gigabyte Technology Co., Ltd.", "X670E AORUS MASTER", 0x8689).is_some());
        assert!(super::layout("ASUSTeK COMPUTER INC.", "ROG STRIX X670E-E GAMING WIFI", 0xD592).is_none());
        // Looked for again only where it was the bus that was not to be had.
        assert!(super::waits_for_the_bus(&format!("{}: held by another program for 1000 ms and more", super::BUS_NOT_HAD)));
        assert!(!super::waits_for_the_bus("0x2E: nothing; 0x4E: chip 5509, not supported"));
    }

    /// Not a key is written to the configuration ports of a laptop (the G14's, a notebook's, or one whose firmware
    /// lists a desktop's enclosure but gives it a mobile role), and a desktop's are probed as they were.
    #[test]
    fn no_laptops_ports_are_probed() {
        use crate::smbios::examples;
        use windows::Win32::System::Power::{PlatformRoleDesktop, PlatformRoleMobile, PlatformRoleWorkstation};
        assert!(!super::probed(&examples::g14(), PlatformRoleMobile));
        assert!(!super::probed(&examples::so_dimm_laptop(), PlatformRoleDesktop));
        assert!(!super::probed(&examples::in_enclosure(0x1F), PlatformRoleDesktop));
        assert!(!super::probed(&examples::desktop(), PlatformRoleMobile));
        assert!(super::probed(&examples::desktop(), PlatformRoleDesktop));
        assert!(super::probed(&examples::in_enclosure(0x07), PlatformRoleWorkstation));
        // Not looked for again later, either: that is for a bus another program had.
        assert!(!super::waits_for_the_bus(super::ON_A_LAPTOP));
    }

    /// Needs administrator rights and the PawnIO driver.
    #[test]
    #[ignore = "needs administrator rights and the PawnIO driver; run with --ignored"]
    fn reads_this_board() {
        let mut chip = super::SuperIo::open("Gigabyte Technology Co., Ltd.", "X870E AORUS MASTER").expect("a supported Super I/O chip, elevated");
        println!("{:?}", chip.read(crate::sensors::LOCK_WAIT));
    }
}
