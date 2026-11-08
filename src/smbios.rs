//! The installed memory modules, and whether the machine is a laptop, from
//! the hardware inventory the firmware hands Windows (SMBIOS). Readable
//! without administrator rights, and read from what Windows keeps of it: no
//! device is asked anything.

use windows::Win32::System::SystemInformation::{GetSystemFirmwareTable, RSMB};

/// SMBIOS structure types of the system's enclosure (its chassis), and of one
/// memory device (a module, or an empty slot).
const ENCLOSURE: u8 = 3;
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
    /// What it is, as SMBIOS numbers them: its form (a DIMM 0x09, an SO-DIMM 0x0D, chips on the board 0x0B, or
    /// "other" as soldered memory is listed, 0x01) and its memory's type (DDR4 0x1A, DDR5 0x22, LPDDR5 0x23).
    pub form_factor: u8,
    pub memory_type: u8,
}

/// The firmware's table, as Windows keeps it (none where it gives none).
pub fn table() -> Vec<u8> {
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
    let mut raw = vec![0u8; size as usize];
    if size == 0 || unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut raw)) } != size {
        return Vec::new();
    }
    raw.split_off(RAW_HEADER.min(raw.len()))
}

/// Every populated memory slot.
pub fn memory_modules() -> Vec<Module> {
    modules(&table())
}

/// Every populated memory slot `table` lists.
pub fn modules(table: &[u8]) -> Vec<Module> {
    structures(table).filter(|(kind, _)| *kind == MEMORY_DEVICE).filter_map(|(_, fields)| memory_device(fields)).collect()
}

/// The chassis type of each enclosure `table` lists (its lock, the top bit, left out).
pub fn chassis(table: &[u8]) -> Vec<u8> {
    structures(table).filter(|(kind, _)| *kind == ENCLOSURE).filter_map(|(_, fields)| fields.get(0x05)).map(|kind| kind & 0x7F).collect()
}

/// The chassis types SMBIOS has for a machine carried about: portable, laptop, notebook, hand-held, sub-notebook,
/// tablet, convertible, detachable, and a docking station, which is of one.
const PORTABLE: [u8; 9] = [0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0E, 0x1E, 0x1F, 0x20];

/// Whether the machine whose enclosures are of the chassis types `chassis` is a laptop (or a tablet, a
/// convertible): one of them is of a kind carried about. (A battery does not tell: a desktop on a UPS says it has
/// one, and some say so of their own.)
pub fn portable(chassis: &[u8]) -> bool {
    chassis.iter().any(|kind| PORTABLE.contains(kind))
}

/// Each structure of `table`: its type, and its formatted part.
fn structures(table: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        if at + 4 > table.len() {
            return None;
        }
        let kind = table[at];
        let length = table[at + 1] as usize;
        if kind == END_OF_TABLE || length < 4 || at + length > table.len() {
            return None;
        }
        let fields = &table[at..at + length];
        // The formatted part is followed by its strings, ended by two zeros.
        let mut next = at + length;
        while next + 1 < table.len() && !(table[next] == 0 && table[next + 1] == 0) {
            next += 1;
        }
        at = next + 2;
        Some((kind, fields))
    })
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
    let (form_factor, memory_type) = (*fields.get(0x0E)?, *fields.get(0x12)?);
    let kind = match memory_type {
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
    Some(Module { megabytes, kind, speed, rated, form_factor, memory_type })
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

/// Firmware tables as two machines have them, their enclosures and memory devices alone (the fields read here, the
/// rest zero, and no strings).
#[cfg(test)]
pub mod examples {
    /// One structure: its type, the length of its formatted part, and the fields set in it (offset, bytes).
    fn structure(kind: u8, length: u8, set: &[(usize, &[u8])]) -> Vec<u8> {
        let mut bytes = vec![0u8; length as usize];
        (bytes[0], bytes[1]) = (kind, length);
        for (at, value) in set {
            bytes[*at..*at + value.len()].copy_from_slice(value);
        }
        bytes.extend([0, 0]);
        bytes
    }

    fn enclosure(chassis: u8) -> Vec<u8> {
        structure(3, 22, &[(0x05, &[chassis])])
    }

    /// A memory device: its length, size field, form, memory type, extended size and speed (rated, and run at).
    fn memory(length: u8, size: u16, form: u8, kind: u8, extended: u32, speed: u16) -> Vec<u8> {
        structure(17, length, &[(0x0C, &size.to_le_bytes()), (0x0E, &[form]), (0x12, &[kind]), (0x15, &speed.to_le_bytes()), (0x1C, &extended.to_le_bytes()), (0x20, &speed.to_le_bytes())])
    }

    /// An ASUS ROG Zephyrus G14 (2025: Radeon 880M, RTX 5070 Ti Laptop GPU): a notebook, its 32 GB of LPDDR5X soldered
    /// on, listed as four LPDDR5 devices of 8 GB of the form "other".
    pub fn g14() -> Vec<u8> {
        let mut table = enclosure(0x0A);
        for _ in 0..4 {
            table.extend(memory(92, 0x2000, 0x01, 0x23, 0, 8000));
        }
        table.extend([127, 4, 0, 0, 0, 0]);
        table
    }

    /// A desktop (Gigabyte X870E AORUS MASTER): two DDR5 DIMMs of 32 GB, and two slots empty.
    pub fn desktop() -> Vec<u8> {
        let mut table = enclosure(0x03);
        for _ in 0..2 {
            table.extend(memory(100, 0, 0x02, 0x02, 0, 0));
            table.extend(memory(100, 0x7FFF, 0x09, 0x22, 32768, 6000));
        }
        table.extend([127, 4, 0, 0, 0, 0]);
        table
    }

    /// `desktop`, with DDR4 DIMMs in its slots in place of DDR5 ones.
    pub fn ddr4_desktop() -> Vec<u8> {
        let mut table = enclosure(0x03);
        for _ in 0..2 {
            table.extend(memory(84, 0x4000, 0x09, 0x1A, 0, 3200));
        }
        table
    }

    /// A laptop with a DDR5 SO-DIMM in its slot.
    pub fn so_dimm_laptop() -> Vec<u8> {
        let mut table = enclosure(0x0A);
        table.extend(memory(92, 0x4000, 0x0D, 0x22, 0, 5600));
        table
    }

    /// `desktop` in another enclosure (a chassis type).
    pub fn in_enclosure(chassis: u8) -> Vec<u8> {
        let mut table = enclosure(chassis);
        table.extend(desktop().split_off(24));
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A laptop by its enclosure's chassis type, as its firmware says: the G14, a notebook, is one, and the
    /// desktop is not, nor an all-in-one, a mini PC or a server. Tablets, convertibles and detachables are, and the
    /// lock bit some firmware sets beside the type changes nothing.
    #[test]
    fn tells_a_laptop_by_its_chassis() {
        assert_eq!(chassis(&examples::g14()), [0x0A]);
        assert_eq!(chassis(&examples::desktop()), [0x03]);
        assert!(portable(&chassis(&examples::g14())));
        assert!(!portable(&chassis(&examples::desktop())));
        for laptop in [0x08, 0x09, 0x0A, 0x0C, 0x0E, 0x1E, 0x1F, 0x20, 0x89] {
            assert!(portable(&chassis(&examples::in_enclosure(laptop))), "{laptop:#X}");
        }
        for desktop in [0x01, 0x02, 0x03, 0x06, 0x07, 0x0D, 0x11, 0x17, 0x23, 0x83] {
            assert!(!portable(&chassis(&examples::in_enclosure(desktop))), "{desktop:#X}");
        }
        assert!(!portable(&[]));
    }

    /// What each module is: the G14's soldered LPDDR5 as of form "other", the desktop's DDR5 DIMMs, its empty slots
    /// none.
    #[test]
    fn tells_what_each_module_is() {
        let g14 = modules(&examples::g14());
        assert_eq!(g14.len(), 4);
        assert!(g14.iter().all(|m| (m.megabytes, m.kind, m.form_factor, m.memory_type) == (8192, "LPDDR5", 0x01, 0x23)));
        let desktop = modules(&examples::desktop());
        assert_eq!(desktop.len(), 2);
        assert!(desktop.iter().all(|m| (m.megabytes, m.kind, m.form_factor, m.memory_type, m.speed) == (32768, "DDR5", 0x09, 0x22, Some(6000))));
        assert_eq!(describe(&desktop).unwrap(), "2 × 32 GB DDR5-6000");
    }

    #[test]
    fn reads_this_machines_enclosure() {
        let table = table();
        println!("{:?}", chassis(&table));
        assert!(!chassis(&table).is_empty());
    }

    #[test]
    fn reads_this_machines_memory() {
        let modules = memory_modules();
        println!("{modules:?} -> {:?}, {:?}", describe(&modules), speed(&modules));
        assert!(!modules.is_empty());
    }

    #[test]
    fn tells_a_speed_below_the_rated_one() {
        let module = |speed, rated| Module { megabytes: 16384, kind: "DDR5", speed: Some(speed), rated: Some(rated), form_factor: 0x09, memory_type: 0x22 };
        assert_eq!(speed(&[module(4800, 6000)]), Some((4800, Some(6000))));
        assert_eq!(speed(&[module(6000, 6000)]), Some((6000, None)));
        assert_eq!(speed(&[]), None);
    }

    #[test]
    fn describes_modules() {
        let ddr5 = |mb| Module { megabytes: mb, kind: "DDR5", speed: Some(6000), rated: Some(6000), form_factor: 0x09, memory_type: 0x22 };
        assert_eq!(describe(&[ddr5(32768), ddr5(32768)]).unwrap(), "2 × 32 GB DDR5-6000");
        assert_eq!(describe(&[ddr5(16384), ddr5(32768)]).unwrap(), "48 GB DDR5-6000");
        assert_eq!(describe(&[]), None);
    }
}
