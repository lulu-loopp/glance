//! An NTFS volume's files and folders, every one of them, as its master file table lists them: read through the
//! volume's change journal (FSCTL_ENUM_USN_DATA), the way Everything reads them. The volume is opened to read only,
//! never a drive itself, never to write: its journal is asked for as it is, never made where it has none. Reading it so
//! needs an administrator's rights, which Glance's sensors have.

use memchr::memmem::Finder;
use std::time::{Duration, Instant};
use windows::core::{Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, MFT_ENUM_DATA_V0, USN_JOURNAL_DATA_V0, USN_RECORD_V2};
use windows::Win32::System::IO::DeviceIoControl;

/// A fixed drive, as GetDriveTypeW names one.
const DRIVE_FIXED: u32 = 3;

/// How much of the table is asked for at once.
const CHUNK: usize = 1 << 20;

/// The record of a file reference number in the table, without the sequence number above it (the table is listed in
/// the order of its records).
const RECORD: u64 = (1 << 48) - 1;

/// Where an entry has no folder listed (the root's, which is its own and is not listed).
const NO_PARENT: u32 = u32::MAX;

/// Where an entry's name is as written in `Volume::folded` (case folded, it is the same).
const AS_FOLDED: u32 = u32::MAX;

/// A file or folder on the volume (24 bytes).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// Its file reference number: its record in the table, with the record's sequence number above.
    pub id: u64,
    /// Its folder's place in `Volume::entries`.
    parent: u32,
    /// Where its name, case folded, starts in `Volume::folded`.
    at: u32,
    /// Where its name as written is in `Volume::written`, and how long it is, where it is not as folded.
    written: (u32, u16),
    pub directory: bool,
}

/// What is on a volume: its files and folders in the order of their records, and where its journal had come to when
/// they were read (what changed after, it lists from there).
pub struct Volume {
    /// Its letter.
    pub letter: char,
    pub journal: u64,
    pub next_usn: i64,
    /// Every entry, in the order of their records.
    pub entries: Vec<Entry>,
    /// Every entry's name case folded, in the same order, each ended by a 0 (which no name holds): searched whole at
    /// once, a match never runs from one name into the next.
    folded: Vec<u8>,
    /// The names that are not written as folded (most are written so, all lower case or in a script without cases).
    written: String,
    /// How long it took to read.
    pub read_in: Duration,
}

/// The letters of this PC's fixed drives that are NTFS.
pub fn ntfs_letters() -> Vec<char> {
    let present = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| present & (1 << bit) != 0)
        .map(|bit| (b'A' + bit) as char)
        .filter(|letter| {
            let root = HSTRING::from(format!("{letter}:\\"));
            if unsafe { GetDriveTypeW(&root) } != DRIVE_FIXED {
                return false;
            }
            let mut system = [0u16; 16];
            unsafe { GetVolumeInformationW(&root, None, None, None, None, Some(&mut system)) }.is_ok()
                && String::from_utf16_lossy(&system[..system.iter().position(|&c| c == 0).unwrap_or(0)]) == "NTFS"
        })
        .collect()
}

/// The volume opened to read only, closed when dropped.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Every file and folder on the NTFS volume `letter`.
pub fn read(letter: char) -> Result<Volume> {
    let began = Instant::now();
    let path = HSTRING::from(format!("\\\\.\\{letter}:"));
    let volume = Handle(unsafe {
        CreateFileW(PCWSTR(path.as_ptr()), GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)?
    });
    let mut journal = USN_JOURNAL_DATA_V0::default();
    let mut got = 0u32;
    unsafe {
        DeviceIoControl(
            volume.0,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut journal as *mut _ as *mut _),
            size_of::<USN_JOURNAL_DATA_V0>() as u32,
            Some(&mut got),
            None,
        )?;
    }
    let mut entries = Vec::new();
    // (Each entry's folder by its file reference number, till all are read and it can be found among them.)
    let mut parents = Vec::new();
    let (mut folded, mut written) = (Vec::new(), String::new());
    let mut asked = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: 0, LowUsn: 0, HighUsn: journal.NextUsn };
    // (u64s, that the records in it, each on an 8-byte boundary, are read where they are.)
    let mut buffer = vec![0u64; CHUNK / 8];
    let mut name = String::new();
    loop {
        let read = unsafe {
            DeviceIoControl(
                volume.0,
                FSCTL_ENUM_USN_DATA,
                Some(&asked as *const _ as *const _),
                size_of::<MFT_ENUM_DATA_V0>() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                CHUNK as u32,
                Some(&mut got),
                None,
            )
        };
        // (Past the last record: ERROR_HANDLE_EOF.)
        if read.is_err() || got <= 8 {
            break;
        }
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, got as usize) };
        let mut at = 8usize;
        while at + size_of::<USN_RECORD_V2>() <= bytes.len() {
            let record = unsafe { &*(bytes.as_ptr().add(at) as *const USN_RECORD_V2) };
            if record.RecordLength == 0 {
                break;
            }
            let utf16 = unsafe {
                std::slice::from_raw_parts(
                    bytes.as_ptr().add(at + record.FileNameOffset as usize) as *const u16,
                    record.FileNameLength as usize / 2,
                )
            };
            name.clear();
            name.extend(char::decode_utf16(utf16.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));
            let starts = folded.len();
            for c in name.chars().flat_map(char::to_lowercase) {
                folded.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            }
            let written_at = if folded[starts..] == *name.as_bytes() {
                (AS_FOLDED, 0)
            } else {
                written.push_str(&name);
                ((written.len() - name.len()) as u32, name.len() as u16)
            };
            folded.push(0);
            entries.push(Entry {
                id: record.FileReferenceNumber,
                parent: NO_PARENT,
                at: starts as u32,
                written: written_at,
                directory: record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
            });
            parents.push(record.ParentFileReferenceNumber);
            at += record.RecordLength as usize;
        }
        asked.StartFileReferenceNumber = buffer[0];
    }
    // Each entry's folder, found among them by its record (its sequence number too, or it is a folder since gone).
    let places: Vec<u32> = parents
        .iter()
        .map(|&parent| {
            entries
                .binary_search_by_key(&(parent & RECORD), |entry| entry.id & RECORD)
                .ok()
                .filter(|&place| entries[place].id == parent)
                .map_or(NO_PARENT, |place| place as u32)
        })
        .collect();
    drop(parents);
    for (entry, place) in entries.iter_mut().zip(places) {
        entry.parent = place;
    }
    entries.shrink_to_fit();
    folded.shrink_to_fit();
    written.shrink_to_fit();
    Ok(Volume { letter, journal: journal.UsnJournalID, next_usn: journal.NextUsn, entries, folded, written, read_in: began.elapsed() })
}

impl Volume {
    /// An entry's name.
    pub fn name(&self, entry: &Entry) -> &str {
        if entry.written.0 != AS_FOLDED {
            return &self.written[entry.written.0 as usize..][..entry.written.1 as usize];
        }
        let name = &self.folded[entry.at as usize..];
        // (Folded from a str, it is UTF-8.)
        std::str::from_utf8(&name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())]).unwrap()
    }

    /// An entry's whole path, from its letter.
    pub fn path(&self, entry: &Entry) -> String {
        let mut parts = vec![self.name(entry)];
        let mut parent = entry.parent;
        // (Up to the root, which is not listed; a loop in a broken table goes no further.)
        while parent != NO_PARENT && parts.len() < 1024 {
            let up = &self.entries[parent as usize];
            parts.push(self.name(up));
            parent = up.parent;
        }
        let mut path = format!("{}:", self.letter);
        for part in parts.iter().rev() {
            path.push('\\');
            path.push_str(part);
        }
        path
    }

    /// The places in `entries` of those whose names hold `words` (case folded), in order, looked for by `threads`
    /// threads at once, each through its share of the names.
    pub fn find(&self, words: &str, threads: usize) -> Vec<u32> {
        let words = words.to_lowercase();
        if words.is_empty() {
            return (0..self.entries.len() as u32).collect();
        }
        let finder = Finder::new(words.as_bytes());
        // Shares of about the same size, each from the start of a name.
        let mut bounds = vec![0usize];
        for share in 1..threads {
            let from = self.folded.len() * share / threads;
            let start = self.folded[from..].iter().position(|&b| b == 0).map_or(self.folded.len(), |end| from + end + 1);
            if start > *bounds.last().unwrap() {
                bounds.push(start);
            }
        }
        bounds.push(self.folded.len());
        let shares: Vec<Vec<u32>> = std::thread::scope(|scope| {
            let finder = &finder;
            let looks: Vec<_> = bounds
                .windows(2)
                .map(|share| {
                    let (from, to) = (share[0], share[1]);
                    scope.spawn(move || {
                        let mut found = Vec::new();
                        let mut at = from;
                        while let Some(hit) = finder.find(&self.folded[at..to]) {
                            // The entry whose name it is in, and on from the next.
                            let place = self.entries.partition_point(|entry| entry.at as usize <= at + hit) - 1;
                            found.push(place as u32);
                            at = self.entries.get(place + 1).map_or(to, |next| next.at as usize);
                        }
                        found
                    })
                })
                .collect();
            looks.into_iter().map(|look| look.join().unwrap()).collect()
        });
        shares.concat()
    }

    /// About how much memory it holds.
    pub fn bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>() + self.folded.capacity() + self.written.capacity()
    }

    /// How many names are kept as written besides folded.
    pub fn written(&self) -> usize {
        self.entries.iter().filter(|entry| entry.written.0 != AS_FOLDED).count()
    }
}
