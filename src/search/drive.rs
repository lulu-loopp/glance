//! A drive as the user reads it: its root folder opened (never the drive itself), and each of its folders listed by
//! its file id, every name in it at once with its id and attributes. Listed by id, a folder is the one meant even
//! where a link was put in its place since.
//!
//! What is held open of a drive is let go of as Windows asks to take the drive out (see `Drive::hear_removal`), as
//! Everything does: held, a USB drive could not be taken out safely.

use super::index::Item;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};
use windows::core::{Error, Result, HSTRING};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
    CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE, CM_NOTIFY_ACTION_DEVICEREMOVEPENDING, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_1,
    CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_DEVICE_REMOVED, ERROR_NO_MORE_FILES, HANDLE};
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
    /// What is held open of it (where Windows is told it, at a place that stays while it lives).
    held: Box<Held>,
    /// Windows' telling of its removal, where it is heard (see `hear_removal`).
    removal: Option<HCMNOTIFICATION>,
    /// The root's path ("C:\").
    pub path: String,
    /// The volume's serial number and its file system's name, which tell it from another put at the same letter.
    pub serial: u32,
    pub system: String,
    /// The root folder's file id.
    pub root_id: u64,
}

/// What is held open of a drive, let go of as Windows asks to take the drive out.
struct Held {
    /// Its root folder: none once let go of. Held (read) through each use of it, a folder's listing whole, that it is
    /// let go of between them.
    root: RwLock<Option<HANDLE>>,
    /// What is under way on it, to be cancelled as it is let go of; told as its watching ends.
    busy: Mutex<Busy>,
    unwatched: Condvar,
    /// Let go of (Windows asked to take it out, or took it: set with `busy` held, which what begins on it looks at
    /// under the same), the asking failed since (the drive stays), and taken out.
    left: AtomicBool,
    stays: AtomicBool,
    removed: AtomicBool,
}

/// What is under way on a drive: its root (as a journal is read through it), each folder being listed, and its root
/// opened again to be watched (see `Drive::watch`).
struct Busy {
    root: Option<HANDLE>,
    listing: Vec<HANDLE>,
    watched: Option<HANDLE>,
}

impl Busy {
    /// All under way cancelled: a journal read, a listing, a watch waiting (each a request, which its call ends on).
    fn cancel(&self) {
        for handle in self.root.iter().chain(&self.listing).chain(&self.watched) {
            unsafe {
                let _ = windows::Win32::System::IO::CancelIoEx(*handle, None);
            }
        }
    }
}

// (The handles are only listed and read through, from any thread, and its removal's telling is only unregistered.)
unsafe impl Send for Held {}
unsafe impl Sync for Held {}
unsafe impl Send for Drive {}
unsafe impl Sync for Drive {}

impl Drop for Drive {
    fn drop(&mut self) {
        // (Unregistered before what it tells of goes: it waits for a telling under way.)
        if let Some(removal) = self.removal.take() {
            unsafe {
                let _ = CM_Unregister_Notification(removal);
            }
        }
        if let Some(root) = self.held.root.write().unwrap().take() {
            unsafe {
                let _ = CloseHandle(root);
            }
        }
    }
}

/// How long letting go of a drive waits for its watching to end.
const UNWATCH_WAIT: Duration = Duration::from_secs(5);

impl Held {
    /// All held of the drive let go of: what is under way on it cancelled (nothing begun after), its root closed once
    /// what uses it now has ended, and its watching waited for (cancelled again as it waits, a watch asked again as
    /// it was cancelled ended too), for `UNWATCH_WAIT` at most.
    fn let_go(&self) {
        {
            let busy = self.busy.lock().unwrap();
            self.left.store(true, Ordering::SeqCst);
            busy.cancel();
        }
        if let Some(root) = self.root.write().unwrap().take() {
            self.busy.lock().unwrap().root = None;
            unsafe {
                let _ = CloseHandle(root);
            }
        }
        let until = Instant::now() + UNWATCH_WAIT;
        let mut busy = self.busy.lock().unwrap();
        while busy.watched.is_some() {
            busy.cancel();
            let now = Instant::now();
            if now >= until {
                break;
            }
            busy = self.unwatched.wait_timeout(busy, (until - now).min(Duration::from_millis(100))).unwrap().0;
        }
    }

    fn left(&self) -> bool {
        self.left.load(Ordering::SeqCst)
    }
}

/// Windows telling of a drive's removal: asked to take it out, or taken (pulled out unasked), all held of it is let
/// go of at once (Windows waits for this, then takes it out), and the drive is not opened again while it is there;
/// the asking failed (another program held it), it stays, to be opened again; taken out, it is forgotten (put in
/// again, even at once at the same letter, it is followed anew).
unsafe extern "system" fn removal(_: HCMNOTIFICATION, context: *const core::ffi::c_void, action: CM_NOTIFY_ACTION, _: *const CM_NOTIFY_EVENT_DATA, _: u32) -> u32 {
    let held = unsafe { &*(context as *const Held) };
    match action {
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => {
            // (A failing before is of an asking before: this one is not known to fail.)
            held.stays.store(false, Ordering::SeqCst);
            held.let_go();
        }
        CM_NOTIFY_ACTION_DEVICEREMOVEPENDING => held.let_go(),
        CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => {
            held.let_go();
            held.removed.store(true, Ordering::SeqCst);
        }
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => held.stays.store(true, Ordering::SeqCst),
        _ => {}
    }
    0
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
        let root = open_folder(path)?;
        // (Closed if what follows fails.)
        let held = Held {
            root: RwLock::new(Some(root)),
            busy: Mutex::new(Busy { root: Some(root), listing: Vec::new(), watched: None }),
            unwatched: Condvar::new(),
            left: AtomicBool::new(false),
            stays: AtomicBool::new(false),
            removed: AtomicBool::new(false),
        };
        let mut drive = Drive { held: Box::new(held), removal: None, path: path.to_string(), serial: 0, system: String::new(), root_id: 0 };
        let mut system = [0u16; 32];
        unsafe { GetVolumeInformationByHandleW(root, None, Some(&mut drive.serial), None, None, Some(&mut system))? };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(root, &mut info)? };
        drive.system = String::from_utf16_lossy(&system[..system.iter().position(|&c| c == 0).unwrap_or(system.len())]);
        drive.root_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok(drive)
    }

    /// Hears from now on as Windows asks to take the drive out (see `removal`), for a drive followed: what is held of
    /// it let go of then. Whether it is heard.
    pub fn hear_removal(&mut self) -> bool {
        let Some(root) = *self.held.root.read().unwrap() else { return false };
        let filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
            u: CM_NOTIFY_FILTER_0 { DeviceHandle: CM_NOTIFY_FILTER_0_1 { hTarget: root } },
            ..Default::default()
        };
        let mut heard = HCMNOTIFICATION::default();
        let context = &*self.held as *const Held as *const core::ffi::c_void;
        if unsafe { CM_Register_Notification(&filter, Some(context), Some(removal), &mut heard) } != CR_SUCCESS {
            return false;
        }
        self.removal = Some(heard);
        true
    }

    /// Whether what was held of it was let go of (Windows asked to take it out), whether it stays since (the asking
    /// failed), and whether it was taken out.
    pub fn left(&self) -> bool {
        self.held.left()
    }

    pub fn stays(&self) -> bool {
        self.held.stays.load(Ordering::SeqCst)
    }

    pub fn removed(&self) -> bool {
        self.held.removed.load(Ordering::SeqCst)
    }

    /// `asked` of its root folder, while it is held (none once let go of): it is not let go of meanwhile, and a call
    /// on it then is cancelled.
    pub fn with_root<T>(&self, asked: impl FnOnce(HANDLE) -> T) -> Option<T> {
        self.held.root.read().unwrap().map(asked)
    }

    /// Its root opened again to be watched, kept to be let go of with the rest (none where it is let go of, or cannot
    /// be opened). Closed with `unwatch`.
    pub fn watch(&self) -> Option<HANDLE> {
        if self.left() {
            return None;
        }
        // (Opened outside what letting go waits on: a slow drive is not waited for there.)
        let folder = open_folder(&self.path).ok()?;
        let mut busy = self.held.busy.lock().unwrap();
        if self.left() {
            drop(busy);
            unsafe {
                let _ = CloseHandle(folder);
            }
            return None;
        }
        busy.watched = Some(folder);
        Some(folder)
    }

    /// Its root opened to be watched (see `watch`) closed, as its watching ends.
    pub fn unwatch(&self) {
        let mut busy = self.held.busy.lock().unwrap();
        if let Some(folder) = busy.watched.take() {
            unsafe {
                let _ = CloseHandle(folder);
            }
        }
        self.held.unwatched.notify_all();
    }

    /// Whether its folders are found by their file ids (NTFS's, which stay with a file as it is renamed or moved);
    /// another drive's are found by their paths.
    pub fn by_id(&self) -> bool {
        self.system == "NTFS"
    }

    /// The names in `folder` (by its id where the drive's are kept, else by its path), `buffer` being where they are
    /// read.
    /// (The drive held through the listing whole: none is begun once it is let go of, and it is let go of after one
    /// under way.)
    pub fn list(&self, folder: &Folder, buffer: &mut [u64]) -> Listing {
        let held = self.held.root.read().unwrap();
        let Some(root) = *held else { return Listing::Failed(Error::from(ERROR_DEVICE_REMOVED)) };
        let opened = match folder {
            Folder::Id(id) => {
                let described = FILE_ID_DESCRIPTOR { dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32, Type: FileIdType, Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: *id as i64 } };
                unsafe { OpenFileById(root, &described, LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, FILE_FLAG_BACKUP_SEMANTICS) }
            }
            Folder::Path(path) => unsafe {
                CreateFileW(&HSTRING::from(path.as_os_str()), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
            },
        };
        let folder = match opened {
            Ok(folder) => Listed::new(&self.held, folder),
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => return Listing::Denied,
            // (An id no longer of a file, ERROR_INVALID_PARAMETER, or a path not found.)
            Err(_) => return Listing::Gone,
        };
        // (Let go of as it was opened: not read, and closed at once.)
        let Some(folder) = folder else { return Listing::Failed(Error::from(ERROR_DEVICE_REMOVED)) };
        let mut items = Vec::new();
        let mut class = FileIdBothDirectoryRestartInfo;
        loop {
            let read = unsafe { GetFileInformationByHandleEx(folder.handle, class, buffer.as_mut_ptr() as *mut _, (buffer.len() * 8) as u32) };
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

/// The serial number of the volume `root` (a folder of it opened) is on, where it can be told.
pub fn serial(root: HANDLE) -> Option<u32> {
    let mut serial = 0u32;
    unsafe { GetVolumeInformationByHandleW(root, None, Some(&mut serial), None, None, None) }.ok()?;
    Some(serial)
}

/// A folder of a drive being listed, its handle kept among what is under way on the drive (see `Busy`) till it is
/// closed, as it is dropped.
struct Listed<'a> {
    held: &'a Held,
    handle: HANDLE,
}

impl<'a> Listed<'a> {
    /// `handle` kept as being listed, unless the drive was let go of as it was opened: then closed, and none.
    fn new(held: &'a Held, handle: HANDLE) -> Option<Listed<'a>> {
        let mut busy = held.busy.lock().unwrap();
        if held.left() {
            drop(busy);
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        busy.listing.push(handle);
        Some(Listed { held, handle })
    }
}

impl Drop for Listed<'_> {
    fn drop(&mut self) {
        // (Taken from what is under way before it is closed: never cancelled once closed.)
        self.held.busy.lock().unwrap().listing.retain(|&handle| handle != self.handle);
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// The folder at `path` opened to be listed (and watched) through.
fn open_folder(path: &str) -> Result<HANDLE> {
    unsafe { CreateFileW(&HSTRING::from(path), LIST, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) }
}

/// A buffer to list folders into (8-byte aligned, as the entries in it are).
pub fn buffer() -> Vec<u64> {
    vec![0u64; CHUNK / 8]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Let go of (as Windows asks to take the drive out), all held of a drive is closed: its root, once the listing
    /// under way is done, and its watching, cancelled and ended. Nothing more is opened of it after.
    #[test]
    fn all_held_of_a_drive_is_let_go_of() {
        let folder = std::env::temp_dir().join(format!("glance-let-go-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        let path = format!("{}\\", folder.display());
        let mut drive = Drive::open(&path).unwrap();
        assert!(drive.hear_removal(), "its removal heard");
        let drive = std::sync::Arc::new(drive);
        let (tell, told) = std::sync::mpsc::channel();
        let (ended, watch_ended) = std::sync::mpsc::channel();
        std::thread::spawn({
            let drive = drive.clone();
            move || {
                super::super::watch::watch(&drive, tell);
                let _ = ended.send(());
            }
        });
        // (Watching: a change seen, written again till it is, the watch's first wait asked at its own time.)
        let mut seen = false;
        for at in 0..50 {
            std::fs::write(folder.join("inner").join(format!("{at}.txt")), "a").unwrap();
            if told.recv_timeout(Duration::from_millis(100)).is_ok() {
                seen = true;
                break;
            }
        }
        assert!(seen, "watched");
        assert!(matches!(drive.list(&Folder::Path(folder.clone()), &mut buffer()), Listing::Items(items) if !items.is_empty()));

        drive.held.let_go();
        assert!(watch_ended.recv_timeout(Duration::from_secs(1)).is_ok(), "its watching ended at once");
        assert!(drive.left() && !drive.stays());
        {
            let busy = drive.held.busy.lock().unwrap();
            assert!(drive.held.root.read().unwrap().is_none() && busy.root.is_none() && busy.watched.is_none() && busy.listing.is_empty());
        }
        assert!(matches!(drive.list(&Folder::Path(folder.clone()), &mut buffer()), Listing::Failed(_)));
        assert!(drive.watch().is_none() && drive.with_root(|_| ()).is_none());
        // (Nothing held: the folder can be removed.)
        drop(drive);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// A USB drive whose root is held can be taken out safely, as Windows asks for it, once its removal is heard, where
    /// held as before it could not (the rest held as a drive is followed, listing and watching, is let go of with the
    /// root: see `all_held_of_a_drive_is_let_go_of`):
    /// `$env:GLANCE_EJECT="G"; cargo test --lib -- --ignored a_usb_drive_followed_can_be_taken_out --nocapture`
    /// (it is taken out: put it in again after).
    #[test]
    #[ignore = "takes a drive put in out"]
    fn a_usb_drive_followed_can_be_taken_out() {
        let letter = std::env::var("GLANCE_EJECT").expect("GLANCE_EJECT: the drive's letter");
        let root = format!("{letter}:\\");
        let device = removable_device(&letter).expect("a removable device under the drive");
        // Held as before (its removal not heard): Windows may not take it out.
        let held = Drive::open(&root).unwrap();
        let (veto, why) = eject(device);
        println!("held as before: veto {veto} ({why})");
        // (Refused by its volume, which cannot be let go of with a folder of it open: PNP_VetoDevice, naming it.)
        assert!(veto != 0 && why.contains("STORAGE\\Volume"), "held, it could not be taken out, for what was open");
        drop(held);
        // Followed now: let go of as Windows asks, and taken out.
        let mut drive = Drive::open(&root).unwrap();
        assert!(drive.hear_removal());
        let (veto, why) = eject(device);
        println!("followed: veto {veto} ({why}), let go of {}", drive.left());
        assert!(veto == 0 && drive.left());
    }

    /// A USB drive being listed and watched, as search follows one, can be taken out safely as Windows asks: its
    /// listing and its watching end, and nothing more is opened of it:
    /// `$env:GLANCE_EJECT="G"; cargo test --lib -- --ignored a_usb_drive_listed_and_watched_can_be_taken_out --nocapture`
    /// (it is taken out: put it in again after).
    #[test]
    #[ignore = "takes a drive put in out"]
    fn a_usb_drive_listed_and_watched_can_be_taken_out() {
        let letter = std::env::var("GLANCE_EJECT").expect("GLANCE_EJECT: the drive's letter");
        let root = format!("{letter}:\\");
        let device = removable_device(&letter).expect("a removable device under the drive");
        let mut drive = Drive::open(&root).unwrap();
        assert!(drive.hear_removal());
        let drive = std::sync::Arc::new(drive);
        let (tell, _told) = std::sync::mpsc::channel();
        let (watch_ended, watching) = std::sync::mpsc::channel();
        std::thread::spawn({
            let drive = drive.clone();
            move || {
                super::super::watch::watch(&drive, tell);
                let _ = watch_ended.send(());
            }
        });
        // Its root listed over and over, as a round of listing does, till it can be no more.
        let (list_ended, listing) = std::sync::mpsc::channel();
        std::thread::spawn({
            let drive = drive.clone();
            let root = std::path::PathBuf::from(&root);
            move || {
                let mut buffer = buffer();
                let mut times = 0u32;
                while !matches!(drive.list(&Folder::Path(root.clone()), &mut buffer), Listing::Failed(_)) {
                    times += 1;
                }
                let _ = list_ended.send(times);
            }
        });
        std::thread::sleep(Duration::from_millis(500));
        let (veto, why) = eject(device);
        println!("listed and watched: veto {veto} ({why}), let go of {}", drive.left());
        assert!(veto == 0 && drive.left());
        let times = listing.recv_timeout(Duration::from_secs(5)).expect("its listing ended");
        println!("listed {times} times before");
        assert!(times > 0 && watching.recv_timeout(Duration::from_secs(5)).is_ok(), "its watching ended");
    }

    /// The device that is taken out with the drive at `letter`: the nearest above its disk that Windows may remove.
    pub(crate) fn removable_device(letter: &str) -> Option<u32> {
        use windows::Win32::Devices::DeviceAndDriverInstallation::*;
        use windows::Win32::System::Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
        let number = |path: &HSTRING| unsafe {
            let handle = CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None).ok()?;
            let mut number = STORAGE_DEVICE_NUMBER::default();
            let asked = windows::Win32::System::IO::DeviceIoControl(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, 0, Some(&mut number as *mut _ as *mut _), size_of::<STORAGE_DEVICE_NUMBER>() as u32, None, None);
            let _ = CloseHandle(handle);
            asked.ok().map(|_| number.DeviceNumber)
        };
        let wanted = number(&HSTRING::from(format!(r"\\.\{letter}:")))?;
        unsafe {
            let set = SetupDiGetClassDevsW(Some(&GUID_DEVINTERFACE_DISK), windows::core::PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE).ok()?;
            let mut disk = None;
            for member in 0.. {
                let mut interface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
                if SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVINTERFACE_DISK, member, &mut interface).is_err() {
                    break;
                }
                let mut size = 0u32;
                let _ = SetupDiGetDeviceInterfaceDetailW(set, &interface, None, 0, Some(&mut size), None);
                let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
                let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
                if SetupDiGetDeviceInterfaceDetailW(set, &interface, Some(detail), size, None, Some(&mut info)).is_err() {
                    continue;
                }
                let path = windows::core::PCWSTR((*detail).DevicePath.as_ptr());
                if number(&HSTRING::from_wide(path.as_wide())) == Some(wanted) {
                    disk = Some(info.DevInst);
                    break;
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            let mut device = disk?;
            loop {
                let mut capabilities = 0u32;
                let mut length = 4u32;
                if CM_Get_DevNode_Registry_PropertyW(device, CM_DRP_CAPABILITIES, None, Some(&mut capabilities as *mut _ as *mut _), &mut length, 0) == CR_SUCCESS && capabilities & CM_DEVCAP_REMOVABLE.0 != 0 {
                    return Some(device);
                }
                let mut parent = 0u32;
                if CM_Get_Parent(&mut parent, device, 0) != CR_SUCCESS {
                    return None;
                }
                device = parent;
            }
        }
    }

    /// `device` asked to be taken out: why it was refused (PNP_VETO_TYPE, 0: it was not), and by what.
    pub(crate) fn eject(device: u32) -> (i32, String) {
        use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_Request_Device_EjectW, PNP_VETO_TYPE};
        let mut veto = PNP_VETO_TYPE::default();
        let mut name = [0u16; 260];
        let asked = unsafe { CM_Request_Device_EjectW(device, Some(&mut veto), Some(&mut name), 0) };
        let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
        (veto.0, format!("{asked:?} {name}"))
    }
}
