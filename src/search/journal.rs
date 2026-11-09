//! What changed on an NTFS drive, as its change journal says to the user: which files and folders were made, gone,
//! renamed or moved, by their ids and their folders' (the user is not told their names). Only a hint of which
//! folders to list again: what a folder holds is always as it is listed.

use super::drive::Drive;
use windows::core::{Error, Result};
use windows::Win32::Foundation::ERROR_DEVICE_REMOVED;
use windows::Win32::System::Ioctl::{FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0, USN_RECORD_COMMON_HEADER};
use windows::Win32::System::IO::DeviceIoControl;

/// The changes that move a name: made, gone, renamed or moved (its old name and its new), a hard link made or gone,
/// a link made or changed.
const NAMED: u32 = 0x0000_0100 | 0x0000_0200 | 0x0000_1000 | 0x0000_2000 | 0x0001_0000 | 0x0010_0000;

/// ERROR_JOURNAL_ENTRY_DELETED: what was asked from is no longer kept.
const ENTRY_DELETED: u32 = 1181;

/// How much is read at once.
const CHUNK: usize = 64 * 1024;

/// Where a drive's journal is: which journal (made again, it is another), the first change it still keeps, and where
/// the next will be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Journal {
    pub id: u64,
    pub first: i64,
    pub next: i64,
}

/// What reading on from a point found.
pub enum Read {
    /// The folders the changes were in (by their ids, each once), where to read on from, and whether there may be
    /// more (what was read reached `read`'s most).
    Changed { folders: Vec<u64>, next: i64, more: bool },
    /// What was asked from is no longer kept (or the journal is another): the drive must be listed whole again.
    Lost,
}

/// The drive's journal, where it has one.
pub fn query(drive: &Drive) -> Result<Journal> {
    let mut journal = USN_JOURNAL_DATA_V0::default();
    let mut got = 0u32;
    let asked = drive.with_root(|root| unsafe {
        DeviceIoControl(root, FSCTL_QUERY_USN_JOURNAL, None, 0, Some(&mut journal as *mut _ as *mut _), size_of::<USN_JOURNAL_DATA_V0>() as u32, Some(&mut got), None)
    });
    asked.unwrap_or_else(|| Err(Error::from(ERROR_DEVICE_REMOVED)))?;
    Ok(Journal { id: journal.UsnJournalID, first: journal.FirstUsn, next: journal.NextUsn })
}

/// The changes since `from` in journal `id`: all there are now, or as many as `most` bytes of records (a journal
/// gone far on is read a part at a time, the rest after).
pub fn read(drive: &Drive, id: u64, from: i64, most: usize) -> Result<Read> {
    match query(drive) {
        Ok(now) if now.id != id || from < now.first => return Ok(Read::Lost),
        Ok(_) => {}
        Err(error) => return Err(error),
    }
    gather(from, most, |at, buffer| {
        let asked = READ_USN_JOURNAL_DATA_V0 { StartUsn: at, ReasonMask: NAMED, ReturnOnlyOnClose: 0, Timeout: 0, BytesToWaitFor: 0, UsnJournalID: id };
        let mut got = 0u32;
        drive
            .with_root(|root| unsafe {
                DeviceIoControl(
                    root,
                    FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
                    Some(&asked as *const _ as *const _),
                    size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                    Some(buffer.as_mut_ptr() as *mut _),
                    CHUNK as u32,
                    Some(&mut got),
                    None,
                )
            })
            .unwrap_or_else(|| Err(Error::from(ERROR_DEVICE_REMOVED)))
            .map(|()| got)
    })
}

/// The changes from `from` on, as `chunk` reads them (from a place, into a buffer of `CHUNK` bytes: how many it
/// read), till there are no more or `most` bytes were read.
fn gather(from: i64, most: usize, mut chunk: impl FnMut(i64, &mut [u64]) -> Result<u32>) -> Result<Read> {
    let mut folders = Vec::new();
    // (u64s, that the records in it, each on an 8-byte boundary, are read where they are.)
    let mut buffer = vec![0u64; CHUNK / 8];
    let mut at_usn = from;
    let mut read = 0usize;
    let mut more = false;
    loop {
        let got = match chunk(at_usn, &mut buffer) {
            Ok(got) => got,
            Err(error) if error.code() == windows::core::HRESULT::from_win32(ENTRY_DELETED) => return Ok(Read::Lost),
            Err(error) => return Err(error),
        };
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, (got as usize).min(CHUNK)) };
        if bytes.len() < 8 {
            break;
        }
        let next = i64::from_le_bytes(bytes[..8].try_into().unwrap());
        folders.extend(parents(&bytes[8..]));
        if next == at_usn {
            break;
        }
        at_usn = next;
        read += bytes.len();
        if read >= most {
            more = true;
            break;
        }
    }
    folders.sort_unstable();
    folders.dedup();
    Ok(Read::Changed { folders, next: at_usn, more })
}

/// The folders the records in `bytes` were in: version 2's and 3's (its ids 128 bits, of which an NTFS drive's are
/// the low 64), each record's length and fields checked to lie within them.
fn parents(bytes: &[u8]) -> Vec<u64> {
    let mut folders = Vec::new();
    let mut at = 0usize;
    while at + size_of::<USN_RECORD_COMMON_HEADER>() <= bytes.len() {
        let length = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let major = u16::from_le_bytes(bytes[at + 4..at + 6].try_into().unwrap());
        if length < size_of::<USN_RECORD_COMMON_HEADER>() || at + length > bytes.len() {
            break;
        }
        let record = &bytes[at..at + length];
        // Version 2: the file's id at 8, its folder's at 16. Version 3: 128-bit ids, the folder's at 24.
        let parent = match major {
            2 if length >= 24 => Some(u64::from_le_bytes(record[16..24].try_into().unwrap())),
            3 if length >= 40 => Some(u64::from_le_bytes(record[24..32].try_into().unwrap())),
            _ => None,
        };
        folders.extend(parent);
        at += length;
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records of both versions are read for their folders; one claiming more than there is ends the reading.
    #[test]
    fn records_are_read_within_what_was_given() {
        let mut bytes = Vec::new();
        let mut v2 = vec![0u8; 64];
        v2[..4].copy_from_slice(&64u32.to_le_bytes());
        v2[4..6].copy_from_slice(&2u16.to_le_bytes());
        v2[16..24].copy_from_slice(&7u64.to_le_bytes());
        bytes.extend(&v2);
        let mut v3 = vec![0u8; 80];
        v3[..4].copy_from_slice(&80u32.to_le_bytes());
        v3[4..6].copy_from_slice(&3u16.to_le_bytes());
        v3[24..32].copy_from_slice(&9u64.to_le_bytes());
        bytes.extend(&v3);
        let mut long = vec![0u8; 16];
        long[..4].copy_from_slice(&4096u32.to_le_bytes());
        bytes.extend(&long);
        assert_eq!(parents(&bytes), [7, 9]);
    }

    /// A journal gone far on is read a part at a time (as much as is asked at most), each read going on where the one
    /// before stopped, till all is read: never all of it at once.
    #[test]
    fn a_journal_gone_far_on_is_read_a_part_at_a_time() {
        // 100 changes, one at each place from 0, in folder 1000 and its place, each read giving one.
        let journal = |at: i64, buffer: &mut [u64]| -> Result<u32> {
            let bytes = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, CHUNK) };
            if at >= 100 {
                bytes[..8].copy_from_slice(&at.to_le_bytes());
                return Ok(8);
            }
            bytes[..8].copy_from_slice(&(at + 1).to_le_bytes());
            bytes[8..72].fill(0);
            bytes[8..12].copy_from_slice(&64u32.to_le_bytes());
            bytes[12..14].copy_from_slice(&2u16.to_le_bytes());
            bytes[24..32].copy_from_slice(&(1000 + at as u64).to_le_bytes());
            Ok(72)
        };
        let mut from = 0;
        let mut reads = 0;
        let mut all = Vec::new();
        loop {
            let Read::Changed { folders, next, more } = gather(from, 10 * 72, journal).unwrap() else { panic!("lost") };
            reads += 1;
            assert!(folders.len() <= 10, "a part at a time: {}", folders.len());
            all.extend(folders);
            from = next;
            if !more {
                break;
            }
        }
        assert_eq!(all, (1000..1100).collect::<Vec<u64>>());
        assert_eq!((from, reads), (100, 11));
    }
}
