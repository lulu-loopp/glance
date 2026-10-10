//! The installed memory modules, from the hardware inventory the firmware
//! hands Windows (SMBIOS). Readable without administrator rights.

use windows::Win32::System::SystemInformation::{GetSystemFirmwareTable, RSMB};

/// SMBIOS structure type of one memory device (a module, or an empty slot).
const MEMORY_DEVICE: u8 = 17;
const END_OF_TABLE: u8 = 127;
/// Bytes before the table in what Windows returns: calling method, version
/// (major, minor, revision) and the table's length.
const RAW_HEADER: usize = 8;

#[derive(Debug, PartialEq)]
pub struct Module {
    pub megabytes: u64,
    pub kind: &'static str,
    /// The speed the module runs at, in MT/s, if the firmware says (else
    /// what it is rated for).
    pub speed: Option<u16>,
    /// The speed it is rated for (MT/s).
    pub rated: Option<u16>,
}

/// Every populated memory slot.
pub fn memory_modules() -> Vec<Module> {
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
    let mut raw = vec![0u8; size as usize];
    if size == 0 || unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut raw)) } != size {
        return Vec::new();
    }
    parse(&raw[RAW_HEADER.min(raw.len())..])
}

fn parse(table: &[u8]) -> Vec<Module> {
    let mut modules = Vec::new();
    let mut at = 0;
    while at + 4 <= table.len() {
        let kind = table[at];
        let length = table[at + 1] as usize;
        if kind == END_OF_TABLE || length < 4 || at + length > table.len() {
            break;
        }
        let fields = &table[at..at + length];
        if kind == MEMORY_DEVICE {
            if let Some(module) = memory_device(fields) {
                modules.push(module);
            }
        }
        // The formatted part is followed by its strings, ended by two zeros.
        let mut next = at + length;
        while next + 1 < table.len() && !(table[next] == 0 && table[next + 1] == 0) {
            next += 1;
        }
        at = next + 2;
    }
    modules
}

fn word(fields: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(fields.get(at..at + 2)?.try_into().ok()?))
}

fn memory_device(fields: &[u8]) -> Option<Module> {
    let size = word(fields, 0x0C)?;
    let megabytes = match size {
        // No module in this slot, or unknown.
        0 | 0xFFFF => return None,
        // The real size is in the extended field, in MB.
        0x7FFF => u32::from_le_bytes(fields.get(0x1C..0x20)?.try_into().ok()?) as u64 & 0x7FFF_FFFF,
        // Bit 15 set: the size is in KB.
        size if size & 0x8000 != 0 => (size & 0x7FFF) as u64 / 1024,
        size => size as u64,
    };
    let kind = match *fields.get(0x12)? {
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => "",
    };
    // The configured speed (what it runs at) where given, else the rated one.
    let rated = word(fields, 0x15).filter(|&s| s != 0);
    let speed = word(fields, 0x20).filter(|&s| s != 0).or(rated);
    Some(Module { megabytes, kind, speed, rated })
}

/// What the memory runs at (MT/s, the first module's), and what it is
/// rated for, where that is more: run below it (XMP or EXPO off, often).
pub fn speed(modules: &[Module]) -> Option<(u16, Option<u16>)> {
    let first = modules.first()?;
    Some((first.speed?, first.rated.filter(|&rated| rated > first.speed.unwrap_or(0))))
}

/// "2 × 32 GB DDR5-6000", or as much of it as is known.
pub fn describe(modules: &[Module]) -> Option<String> {
    let first = modules.first()?;
    let alike = modules.iter().all(|m| m.megabytes == first.megabytes);
    let size = |mb: u64| if mb % 1024 == 0 { format!("{} GB", mb / 1024) } else { format!("{mb} MB") };
    let mut text = if alike {
        format!("{} × {}", modules.len(), size(first.megabytes))
    } else {
        size(modules.iter().map(|m| m.megabytes).sum())
    };
    if !first.kind.is_empty() {
        text.push(' ');
        text.push_str(first.kind);
        if let Some(speed) = first.speed {
            text.push_str(&format!("-{speed}"));
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_machines_memory() {
        let modules = memory_modules();
        println!("{modules:?} -> {:?}, {:?}", describe(&modules), speed(&modules));
        assert!(!modules.is_empty());
    }

    #[test]
    fn tells_a_speed_below_the_rated_one() {
        let module = |speed, rated| Module { megabytes: 16384, kind: "DDR5", speed: Some(speed), rated: Some(rated) };
        assert_eq!(speed(&[module(4800, 6000)]), Some((4800, Some(6000))));
        assert_eq!(speed(&[module(6000, 6000)]), Some((6000, None)));
        assert_eq!(speed(&[]), None);
    }

    #[test]
    fn describes_modules() {
        let ddr5 = |mb| Module { megabytes: mb, kind: "DDR5", speed: Some(6000), rated: Some(6000) };
        assert_eq!(describe(&[ddr5(32768), ddr5(32768)]).unwrap(), "2 × 32 GB DDR5-6000");
        assert_eq!(describe(&[ddr5(16384), ddr5(32768)]).unwrap(), "48 GB DDR5-6000");
        assert_eq!(describe(&[]), None);
    }
}
