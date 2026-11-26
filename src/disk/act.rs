//! What the disk organizer does on this PC: the drives it shows and their sizes, and what is asked of it there (files
//! moved to the Recycle Bin, OneDrive's files left in the cloud alone, shown in Explorer, the Recycle Bin emptied,
//! Windows' own pages for what it keeps opened). Each is run on a thread of its own, the window not waiting on it.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{HSTRING, PCWSTR};

/// A drive the organizer offers: its root ("C:\\"), its name as Explorer shows it ("Windows (C:)"'s "Windows"), and
/// how large it is and how much is free.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub root: String,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    pub fn letter(&self) -> char {
        self.root.chars().next().unwrap_or('?')
    }

    /// Whether what is free is under 15 % of it.
    pub fn low(&self) -> bool {
        self.total > 0 && (self.free as f64) < self.total as f64 * 0.15
    }
}

/// The drives searched (whose every file is known: see `search`), by their letters.
pub fn drives() -> Vec<Drive> {
    let mut drives: Vec<Drive> = crate::search::standing()
        .into_iter()
        .map(|standing| {
            let (total, free) = space(&standing.root);
            Drive { label: label(&standing.root), root: standing.root, total, free }
        })
        .collect();
    drives.sort_by(|a, b| a.root.cmp(&b.root));
    drives
}

/// How large the drive at `root` is, and how much of it is free (to anyone: as Explorer tells it).
pub fn space(root: &str) -> (u64, u64) {
    let (mut total, mut free) = (0u64, 0u64);
    let _ = unsafe { windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(&HSTRING::from(root), None, Some(&mut total), Some(&mut free)) };
    (total, free)
}

/// The drive at `root`'s name: its label, or, with none, what Explorer calls one with none.
fn label(root: &str) -> String {
    let mut name = [0u16; 261];
    let read = unsafe { windows::Win32::Storage::FileSystem::GetVolumeInformationW(&HSTRING::from(root), Some(&mut name), None, None, None, None) };
    let named = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)]);
    if read.is_ok() && !named.trim().is_empty() {
        return named;
    }
    crate::update::language().pick("本地磁盘", "Local Disk").to_string()
}

/// `act` run on a thread of its own, the shell's (COM's, single-threaded) set up for it.
pub fn by_the_shell(act: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("glance: disk organizer acts".into()).spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        act();
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    });
}

/// The file or folder at `path` moved to the Recycle Bin, as this user may: whether it was. (Where it would be deleted
/// for good, its drive having no Recycle Bin, Windows asks first: never deleted for good unasked.)
pub fn recycle(path: &Path) -> bool {
    use windows::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_NO_CONNECTED_ELEMENTS, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
    // (Its path ended by two nulls, as the call reads a list of them. It alone: a page's folder of files, "x_files",
    // stays where the page goes, as only the page was chosen.)
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut operation = SHFILEOPSTRUCTW { wFunc: FO_DELETE, pFrom: PCWSTR(from.as_ptr()), fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_NOERRORUI | FOF_SILENT | FOF_NO_CONNECTED_ELEMENTS).0 as u16, ..Default::default() };
    unsafe { SHFileOperationW(&mut operation) == 0 && !operation.fAnyOperationsAborted.as_bool() && !path.exists() }
}

/// The Recycle Bin of the drive at `root` emptied, unasked (asked already in the window): whether it was.
pub fn empty_bin(root: &str) -> bool {
    use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND};
    unsafe { SHEmptyRecycleBinW(None, &HSTRING::from(root), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) }.is_ok()
}

/// The OneDrive file or folder at `path` (and all in it) kept in the cloud alone, as Explorer's "Free up space" does:
/// marked not to be kept on this PC, and no longer to be kept always, which OneDrive then does. How many it marked.
pub fn release(path: &Path) -> usize {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, FILE_FLAGS_AND_ATTRIBUTES, INVALID_FILE_ATTRIBUTES};
    // FILE_ATTRIBUTE_PINNED and FILE_ATTRIBUTE_UNPINNED.
    const PINNED: u32 = 0x0008_0000;
    const UNPINNED: u32 = 0x0010_0000;
    let mark = |path: &Path| {
        let wide = HSTRING::from(path.as_os_str());
        let attributes = unsafe { GetFileAttributesW(&wide) };
        if attributes == INVALID_FILE_ATTRIBUTES {
            return false;
        }
        unsafe { SetFileAttributesW(&wide, FILE_FLAGS_AND_ATTRIBUTES((attributes & !PINNED) | UNPINNED)) }.is_ok()
    };
    let mut marked = usize::from(mark(path));
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            marked += usize::from(mark(&entry.path()));
            if kind.is_dir() {
                folders.push(entry.path());
            }
        }
    }
    marked
}

/// `paths` shown in Explorer, selected: each folder they are in opened with those of them in it selected.
pub fn reveal(paths: &[PathBuf]) {
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFindLastID, ILFree, SHOpenFolderAndSelectItems};
    let mut folders: Vec<(PathBuf, Vec<&PathBuf>)> = Vec::new();
    for path in paths {
        let Some(parent) = path.parent() else { continue };
        match folders.iter_mut().find(|(folder, _)| folder == parent) {
            Some((_, inside)) => inside.push(path),
            None => folders.push((parent.to_path_buf(), vec![path])),
        }
    }
    for (folder, inside) in folders {
        unsafe {
            let held = ILCreateFromPathW(&HSTRING::from(folder.as_os_str()));
            if held.is_null() {
                continue;
            }
            let items: Vec<_> = inside.iter().map(|path| ILCreateFromPathW(&HSTRING::from(path.as_os_str()))).filter(|item| !item.is_null()).collect();
            let last: Vec<_> = items.iter().map(|item| ILFindLastID(*item) as *const _).collect();
            let _ = SHOpenFolderAndSelectItems(held, Some(&last), 0);
            items.into_iter().for_each(|item| ILFree(Some(item)));
            ILFree(Some(held));
        }
    }
}

/// The folder at `path` opened in Explorer.
pub fn open_folder(path: &Path) {
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), &HSTRING::from(path.as_os_str()), PCWSTR::null(), PCWSTR::null(), windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
    }
}

/// One of Windows' own pages for what the system keeps on a drive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    /// System Properties › Advanced › Performance › Virtual memory's way in.
    Performance,
    /// System Properties › System Protection.
    Protection,
    /// Control Panel › Power Options › what the power buttons do (hibernation and fast startup).
    PowerButtons,
    /// Settings › System › Storage.
    Storage,
}

/// `page` opened.
pub fn open_page(page: Page) {
    let (file, parameters) = match page {
        Page::Performance => ("SystemPropertiesPerformance.exe", ""),
        Page::Protection => ("SystemPropertiesProtection.exe", ""),
        Page::PowerButtons => ("control.exe", "/name Microsoft.PowerOptions /page pageGlobalSettings"),
        Page::Storage => ("ms-settings:storagesense", ""),
    };
    let parameters = (!parameters.is_empty()).then(|| HSTRING::from(parameters));
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &HSTRING::from(file),
            parameters.as_ref().map_or(PCWSTR::null(), |parameters| PCWSTR(parameters.as_ptr())),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}
