//! What changed on a drive with no change journal (a USB stick's FAT or exFAT, a ReFS drive), as Windows tells a
//! program watching it: the folders names were made, gone or renamed in, by their paths from its root. Only while it
//! is watched: what changed while Glance was not running is found by listing the drive whole as it starts.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_INFORMATION,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// FILE_LIST_DIRECTORY.
const LIST: u32 = 0x1;

/// What watching found.
pub enum Seen {
    /// Names changed in these folders (their paths from the root, "" the root itself).
    Folders(Vec<PathBuf>),
    /// More changed than Windows kept: the drive must be listed whole again.
    Lost,
}

/// Watches the drive whose root is `root` ("E:\") on this thread, telling `tell` what it sees, till the drive is
/// gone or `tell` is not heard any more.
pub fn watch(root: &str, tell: Sender<Seen>) {
    let Ok(folder) = (unsafe {
        CreateFileW(&HSTRING::from(root), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
    }) else {
        return;
    };
    // (u64s: the records in it are on 4-byte boundaries.)
    let mut buffer = vec![0u64; 64 * 1024 / 8];
    loop {
        let mut got = 0u32;
        let read = unsafe {
            ReadDirectoryChangesW(
                folder,
                buffer.as_mut_ptr() as *mut _,
                (buffer.len() * 8) as u32,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                Some(&mut got),
                None,
                None,
            )
        };
        if read.is_err() {
            break;
        }
        let seen = if got == 0 { Seen::Lost } else { Seen::Folders(folders(unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, got as usize) })) };
        if tell.send(seen).is_err() {
            break;
        }
    }
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(folder);
    }
}

/// The folders of the names in the records in `bytes`.
fn folders(bytes: &[u8]) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let mut at = 0usize;
    while at + size_of::<FILE_NOTIFY_INFORMATION>() <= bytes.len() {
        let record = unsafe { &*(bytes.as_ptr().add(at) as *const FILE_NOTIFY_INFORMATION) };
        let len = record.FileNameLength as usize / 2;
        let start = at + std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
        if start + len * 2 > bytes.len() {
            break;
        }
        let name = unsafe { std::slice::from_raw_parts(bytes.as_ptr().add(start) as *const u16, len) };
        let path = PathBuf::from(<std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(name));
        folders.push(path.parent().map(PathBuf::from).unwrap_or_default());
        if record.NextEntryOffset == 0 {
            break;
        }
        at += record.NextEntryOffset as usize;
    }
    folders.sort();
    folders.dedup();
    folders
}
