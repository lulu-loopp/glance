//! A drive as the user reads it: its root folder opened (never the drive itself), and each of its folders listed by
//! its file id, every name in it at once with its id and attributes. Listed by id, a folder is the one meant even
//! where a link was put in its place since.

use super::index::Item;
use windows::core::{Error, Result, HSTRING};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
    OpenFileById, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType, OPEN_EXISTING,
};

/// FILE_LIST_DIRECTORY and FILE_READ_ATTRIBUTES: listing a folder, nothing more.
const LIST: u32 = 0x1 | 0x80;

/// How much of a folder is asked for at once.
const CHUNK: usize = 64 * 1024;

/// A drive's root folder, opened to list folders through.
pub struct Drive {
    root: HANDLE,
    /// The root's path ("C:\").
    pub path: String,
    /// The volume's serial number and its file system's name, which tell it from another put at the same letter.
    pub serial: u32,
    pub system: String,
    /// The root folder's file id.
    pub root_id: u64,
}

// (The handle is only listed through, from any thread.)
unsafe impl Send for Drive {}
unsafe impl Sync for Drive {}

impl Drop for Drive {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.root);
        }
    }
}

/// A folder to list: by its file id, or by its path (`\\?\`-prefixed, as long as it is).
pub enum Folder {
    Id(u64),
    Path(std::path::PathBuf),
}

/// What listing a folder found.
pub enum Listing {
    Items(Vec<Item>),
    /// The user may not list it.
    Denied,
    /// It is not there any more.
    Gone,
    /// It could not be listed whole (something else failed).
    Failed(Error),
}

impl Drive {
    /// The drive whose root is `path` ("C:\").
    pub fn open(path: &str) -> Result<Drive> {
        let root = unsafe {
            CreateFileW(&HSTRING::from(path), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)?
        };
        // (Closed if what follows fails.)
        let mut drive = Drive { root, path: path.to_string(), serial: 0, system: String::new(), root_id: 0 };
        let mut system = [0u16; 32];
        unsafe { GetVolumeInformationByHandleW(drive.root, None, Some(&mut drive.serial), None, None, Some(&mut system))? };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(drive.root, &mut info)? };
        drive.system = String::from_utf16_lossy(&system[..system.iter().position(|&c| c == 0).unwrap_or(system.len())]);
        drive.root_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok(drive)
    }

    pub fn root(&self) -> HANDLE {
        self.root
    }

    /// Whether its folders are found by their file ids (NTFS's, which stay with a file as it is renamed or moved);
    /// another drive's are found by their paths.
    pub fn by_id(&self) -> bool {
        self.system == "NTFS"
    }

    /// The names in `folder` (by its id where the drive's are kept, else by its path), `buffer` being where they are
    /// read.
    pub fn list(&self, folder: &Folder, buffer: &mut [u64]) -> Listing {
        let opened = match folder {
            Folder::Id(id) => {
                let described = FILE_ID_DESCRIPTOR { dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32, Type: FileIdType, Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: *id as i64 } };
                unsafe { OpenFileById(self.root, &described, LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, FILE_FLAG_BACKUP_SEMANTICS) }
            }
            Folder::Path(path) => unsafe {
                CreateFileW(&HSTRING::from(path.as_os_str()), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
            },
        };
        let folder = match opened {
            Ok(folder) => Closed(folder),
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => return Listing::Denied,
            // (An id no longer of a file, ERROR_INVALID_PARAMETER, or a path not found.)
            Err(_) => return Listing::Gone,
        };
        let mut items = Vec::new();
        let mut class = FileIdBothDirectoryRestartInfo;
        loop {
            let read = unsafe { GetFileInformationByHandleEx(folder.0, class, buffer.as_mut_ptr() as *mut _, (buffer.len() * 8) as u32) };
            if let Err(error) = read {
                if error.code() == ERROR_NO_MORE_FILES.to_hresult() {
                    return Listing::Items(items);
                }
                return Listing::Failed(error);
            }
            class = FileIdBothDirectoryInfo;
            let bytes = buffer.as_ptr() as *const u8;
            let mut at = 0usize;
            loop {
                let info = unsafe { &*(bytes.add(at) as *const FILE_ID_BOTH_DIR_INFO) };
                let name = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), info.FileNameLength as usize / 2) };
                if name != [u16::from(b'.')] && name != [u16::from(b'.'), u16::from(b'.')] {
                    // (For a link, EaSize holds what kind of link it is.)
                    items.push(Item { name: name.to_vec(), id: info.FileId as u64, attributes: info.FileAttributes, tag: info.EaSize });
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                at += info.NextEntryOffset as usize;
            }
        }
    }
}

/// A buffer to list folders into (8-byte aligned, as the entries in it are).
pub fn buffer() -> Vec<u64> {
    vec![0u64; CHUNK / 8]
}

struct Closed(HANDLE);

impl Drop for Closed {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
