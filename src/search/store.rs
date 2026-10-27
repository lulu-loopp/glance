//! A drive's index kept on disk, in Glance's own folder (`%LOCALAPPDATA%\Glance\Search`), with where its journal had
//! come to when it was kept, and read where it lies: its file mapped into memory, read in by Windows as a search goes
//! through it and let go of again as other programs want the memory (it is not Glance's own: what Task Manager says
//! Glance holds does not count it).
//!
//! Kept anew as one more file, numbered on (a file mapped in can be neither written over nor put another in its
//! place): written whole under another name, put on the disk, then named as kept, and read where it lies from then
//! on. The one before is kept too, those before it let go of. Checked whole as it is read: one that is not as
//! written is not used.

use super::index::Index;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, GetFileSizeEx, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING};
use windows::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READONLY};

const MAGIC: &[u8; 8] = b"GLNCSRCH";
const VERSION: u32 = 2;
/// How long what is kept before the index is (bytes).
const HEADER: usize = 48;

/// What the index was kept with: the drive it is of, and its journal and where it had come to (the changes before
/// that are in it), where it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept {
    pub serial: u32,
    pub journal: Option<Cursor>,
}

/// A drive's change journal (by its id), and where in it changes are read from next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub journal: u64,
    pub next: i64,
}

/// What an index is read from: a file mapped into memory, or (made here) bytes of its own.
pub enum Bytes {
    Mapped(Mapped),
    Owned(Vec<u64>, usize),
}

impl Bytes {
    pub fn slice(&self) -> &[u8] {
        match self {
            Bytes::Mapped(mapped) => unsafe { std::slice::from_raw_parts(mapped.view.Value as *const u8, mapped.len) },
            Bytes::Owned(words, len) => &bytemuck_bytes(words)[..*len],
        }
    }

    /// A copy of `bytes`, from where an index's parts may be read (a multiple of 8 bytes in memory).
    #[cfg(test)]
    pub fn owned(bytes: &[u8]) -> Bytes {
        let mut words = vec![0u64; bytes.len().div_ceil(8)];
        for (word, part) in words.iter_mut().zip(bytes.chunks(8)) {
            let mut eight = [0u8; 8];
            eight[..part.len()].copy_from_slice(part);
            *word = u64::from_le_bytes(eight);
        }
        Bytes::Owned(words, bytes.len())
    }
}

fn bytemuck_bytes(words: &[u64]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 8) }
}

/// A file mapped into memory to be read (none may write it, or put another in its place, while it is).
pub struct Mapped {
    file: HANDLE,
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

// (Only read, from any thread.)
unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

impl Mapped {
    fn open(path: &std::path::Path) -> io::Result<Mapped> {
        unsafe {
            let file = CreateFileW(&HSTRING::from(path.as_os_str()), GENERIC_READ.0, FILE_SHARE_READ, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None)?;
            let mut size = 0i64;
            let mapped = (|| {
                GetFileSizeEx(file, &mut size)?;
                // (An empty file cannot be mapped; none kept is.)
                if size <= 0 || size as u64 > usize::MAX as u64 {
                    return Err(windows::core::Error::from(windows::Win32::Foundation::ERROR_FILE_INVALID));
                }
                let mapping = CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, None)?;
                let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
                if view.Value.is_null() {
                    let error = windows::core::Error::from_thread();
                    let _ = CloseHandle(mapping);
                    return Err(error);
                }
                Ok((mapping, view))
            })();
            match mapped {
                Ok((mapping, view)) => Ok(Mapped { file, mapping, view, len: size as usize }),
                Err(error) => {
                    let _ = CloseHandle(file);
                    Err(error.into())
                }
            }
        }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
            let _ = CloseHandle(self.file);
        }
    }
}

/// Where the indexes are kept.
fn folder() -> PathBuf {
    crate::settings::local_dir().join("Search")
}

/// The file in `folder` the index of the drive of `serial` is kept in as its `number`th, and the one it is written to
/// first.
fn file(folder: &Path, serial: u32, number: u64) -> PathBuf {
    folder.join(format!("{serial:08X}.{number:016X}.index"))
}

fn writing(folder: &Path, serial: u32, number: u64) -> PathBuf {
    folder.join(format!("{serial:08X}.{number:016X}.new"))
}

/// The numbers of the files the drive of `serial` has kept (each as `file` names it), the last first; and those
/// begun but never finished (each as `writing` names it).
fn numbers(folder: &Path, serial: u32) -> (Vec<u64>, Vec<u64>) {
    let (mut kept, mut begun) = (Vec::new(), Vec::new());
    let prefix = format!("{serial:08X}.");
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        let Some((number, kind)) = rest.split_once('.') else { continue };
        let Ok(number) = u64::from_str_radix(number, 16) else { continue };
        match kind {
            "index" => kept.push(number),
            "new" => begun.push(number),
            _ => {}
        }
    }
    kept.sort_unstable_by(|a, b| b.cmp(a));
    (kept, begun)
}

/// FNV-1a, 64 bits: whether what is read is what was written.
#[derive(Clone, Copy)]
struct Sum(u64);

impl Sum {
    fn new() -> Sum {
        Sum(0xCBF2_9CE4_8422_2325)
    }

    fn add(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }
}

/// What is written, summed as it is.
struct Summed<W: Write> {
    out: W,
    sum: Sum,
}

impl<W: Write> Write for Summed<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let wrote = self.out.write(bytes)?;
        self.sum.add(&bytes[..wrote]);
        Ok(wrote)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// `index` kept with `kept`, of a drive known `by_id`, as the next file: the index as read from there, to be used in
/// its place. (Where it could not be, the one before stands.)
pub fn keep(index: &Index, kept: Kept) -> io::Result<Index> {
    keep_in(&folder(), index, kept)
}

fn keep_in(folder: &Path, index: &Index, kept: Kept) -> io::Result<Index> {
    std::fs::create_dir_all(folder)?;
    let (kept_numbers, _) = numbers(folder, kept.serial);
    let number = kept_numbers.first().map_or(1, |last| last + 1);
    let new = writing(folder, kept.serial, number);
    let written = (|| {
        let mut out = Summed { out: BufWriter::with_capacity(1 << 20, std::fs::File::create(&new)?), sum: Sum::new() };
        out.write_all(MAGIC)?;
        out.write_all(&VERSION.to_le_bytes())?;
        let flags = u32::from(index.by_id()) | u32::from(kept.journal.is_some()) << 1;
        out.write_all(&flags.to_le_bytes())?;
        out.write_all(&kept.serial.to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        let cursor = kept.journal.unwrap_or(Cursor { journal: 0, next: 0 });
        out.write_all(&cursor.journal.to_le_bytes())?;
        out.write_all(&cursor.next.to_le_bytes())?;
        out.write_all(&index.made().to_le_bytes())?;
        index.write(&mut out)?;
        let sum = out.sum.0;
        let mut file = out.out.into_inner().map_err(|error| error.into_error())?;
        file.write_all(&sum.to_le_bytes())?;
        // (On the disk before it is named as kept: a PC that goes off meanwhile has the one before.)
        file.sync_all()
    })();
    let read = written.and_then(|()| {
        let path = file(folder, kept.serial, number);
        std::fs::rename(&new, &path)?;
        read(&path, kept.serial).map(|(index, _)| index).ok_or_else(|| io::Error::other("kept, but not read as it was written"))
    });
    if read.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    read
}

/// The files of the drive of `serial` but the last two let go of (they are read no more), and those begun but never
/// finished. (One that cannot be let go of yet, read still or by a virus scanner, is let go of another time.)
pub fn let_go(serial: u32) {
    let_go_in(&folder(), serial);
}

fn let_go_in(folder: &Path, serial: u32) {
    let (kept, begun) = numbers(folder, serial);
    for &number in kept.iter().skip(2) {
        let _ = std::fs::remove_file(file(folder, serial, number));
    }
    let last = kept.first().copied().unwrap_or(0);
    for number in begun.into_iter().filter(|&number| number < last) {
        let _ = std::fs::remove_file(writing(folder, serial, number));
    }
    // (Kept as Glance first kept them, all of one file: read no more.)
    let _ = std::fs::remove_file(folder.join(format!("{serial:08X}.index")));
}

/// The index the drive of `serial` last kept, read where it lies, and what it was kept with; the one before where
/// that one is not as written; none where none is.
pub fn kept(serial: u32) -> Option<(Index, Kept)> {
    kept_in(&folder(), serial)
}

fn kept_in(folder: &Path, serial: u32) -> Option<(Index, Kept)> {
    numbers(folder, serial).0.into_iter().find_map(|number| read(&file(folder, serial, number), serial))
}

fn read(path: &std::path::Path, serial: u32) -> Option<(Index, Kept)> {
    let bytes = Bytes::Mapped(Mapped::open(path).ok()?);
    let (by_id, kept, made, end) = {
        let body = bytes.slice();
        let (body, sum_bytes) = body.split_at_checked(body.len().checked_sub(8)?)?;
        let mut sum = Sum::new();
        sum.add(body);
        if sum.0 != u64::from_le_bytes(sum_bytes.try_into().ok()?) || body.get(..8)? != MAGIC {
            return None;
        }
        let u32_at = |at: usize| Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?));
        let u64_at = |at: usize| Some(u64::from_le_bytes(body.get(at..at + 8)?.try_into().ok()?));
        let flags = u32_at(12)?;
        if u32_at(8)? != VERSION || u32_at(16)? != serial || flags & !3 != 0 {
            return None;
        }
        let journal = (flags & 2 != 0).then(|| Some(Cursor { journal: u64_at(24)?, next: u64_at(32)? as i64 })).flatten();
        (flags & 1 != 0, Kept { serial, journal }, u64_at(40)?, body.len())
    };
    Some((Index::read(bytes, HEADER, end, by_id, made)?, kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::index::{Changes, Item, ROOT};

    fn index(names: &[&str]) -> Index {
        let mut index = Index::new(5, true);
        let items: Vec<Item> = names.iter().enumerate().map(|(n, name)| Item { name: name.encode_utf16().collect(), id: 10 + n as u64, attributes: 0x20, tag: 0 }).collect();
        let mut changes = Changes::default();
        index.list(ROOT, &items, &[], &mut changes);
        index.finish(changes);
        index
    }

    fn names(index: &Index) -> Vec<String> {
        (1..index.len()).map(|place| index.name(place).to_string_lossy().into_owned()).collect()
    }

    /// Kept, it is read where it lies, with what it was kept with; kept again, it is the next file, the one before it
    /// kept, those older let go of (once they are read no more); the last not as written, the one before is read.
    #[test]
    fn indexes_are_kept_one_after_another() {
        let folder = std::env::temp_dir().join(format!("glance-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let kept = Kept { serial: 0xABCD, journal: Some(Cursor { journal: 7, next: 99 }) };
        let first = keep_in(&folder, &index(&["a.txt"]), kept).unwrap();
        assert_eq!(names(&first), ["a.txt"]);
        let (read, read_kept) = kept_in(&folder, 0xABCD).unwrap();
        assert_eq!((names(&read), read_kept), (vec!["a.txt".to_string()], kept));
        // (Mapped in, it cannot be put away.)
        assert!(std::fs::remove_file(file(&folder, 0xABCD, 1)).is_err());
        drop((first, read));

        let second = keep_in(&folder, &index(&["b.txt"]), Kept { journal: None, ..kept }).unwrap();
        let third = keep_in(&folder, &index(&["c.txt"]), kept).unwrap();
        drop(second);
        let_go_in(&folder, 0xABCD);
        assert_eq!(numbers(&folder, 0xABCD).0, [3, 2]);
        assert_eq!(names(&kept_in(&folder, 0xABCD).unwrap().0), ["c.txt"]);
        drop(third);

        // The last not as written: the one before is read.
        let last = file(&folder, 0xABCD, 3);
        let mut bytes = std::fs::read(&last).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x40;
        std::fs::write(&last, bytes).unwrap();
        let (read, read_kept) = kept_in(&folder, 0xABCD).unwrap();
        assert_eq!((names(&read), read_kept.journal), (vec!["b.txt".to_string()], None));
        // Another drive's are not read.
        assert!(kept_in(&folder, 0x1234).is_none());
        drop(read);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
