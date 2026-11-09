//! A drive as the user reads it: its root folder opened (never the drive itself), and each of its folders listed by
//! its file id, every name in it at once with its id and attributes. Listed by id, a folder is the one meant even
//! where a link was put in its place since.
//!
//! What is held open of a drive is let go of as Windows asks to take the drive out (see `Drive::hear_removal`), as
//! Everything does: held, a USB drive could not be taken out safely.

use super::index::Item;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{Error, Result, HSTRING};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
    CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE, CM_NOTIFY_ACTION_DEVICEREMOVEPENDING, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_1,
    CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_DEVICE_REMOVED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, ERROR_PATH_NOT_FOUND, HANDLE, WIN32_ERROR};
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
    /// Its root folder and what is under way on it, to be cancelled as it is let go of, and told as a use of its root,
    /// or its watching, ends.
    busy: Mutex<Busy>,
    ended: Condvar,
    /// Let go of (Windows asked to take it out, or took it: set with `busy` held, which what begins on it looks at
    /// under the same), the asking failed since (the drive stays), and taken out.
    left: AtomicBool,
    stays: AtomicBool,
    removed: AtomicBool,
}

/// What is under way on a drive: its root folder (none once closed) and how many calls use it now (a journal read
/// through it, a folder listed: see `Drive::with_root`), each folder being listed, and its root opened again to be
/// watched (see `Drive::watch`). A root let go of while it is used is closed by the last call using it, as it ends.
struct Busy {
    root: Option<HANDLE>,
    using: usize,
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
        if let Some(root) = self.held.busy.lock().unwrap().root.take() {
            unsafe {
                let _ = CloseHandle(root);
            }
        }
    }
}

/// How long letting go of a drive waits for what is under way on it to end.
const LET_GO_WAIT: Duration = Duration::from_secs(5);

impl Held {
    /// All held of the drive let go of (see `let_go_within`), waited for `LET_GO_WAIT` at most.
    fn let_go(&self) {
        self.let_go_within(LET_GO_WAIT);
    }

    /// All held of the drive let go of: what is under way on it cancelled (nothing begun after), and cancelled again
    /// as it is waited for (a watch asked again as it was cancelled, a folder opened as it was), its root closed as
    /// soon as no call uses it, till that and its watching have ended, or for `wait` at most. What has not ended by
    /// then (a call to a drive that answers nothing, cancelled or not) is not waited for: Windows' asking is answered,
    /// and the root is closed by the last call using it, as that ends.
    fn let_go_within(&self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut busy = self.busy.lock().unwrap();
        self.left.store(true, Ordering::SeqCst);
        loop {
            busy.cancel();
            if busy.using == 0 {
                if let Some(root) = busy.root.take() {
                    unsafe {
                        let _ = CloseHandle(root);
                    }
                }
                if busy.watched.is_none() {
                    return;
                }
            }
            let now = Instant::now();
            if now >= until {
                return;
            }
            busy = self.ended.wait_timeout(busy, (until - now).min(Duration::from_millis(100))).unwrap().0;
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
            busy: Mutex::new(Busy { root: Some(root), using: 0, listing: Vec::new(), watched: None }),
            ended: Condvar::new(),
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
        let Some(root) = self.held.busy.lock().unwrap().root else { return false };
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

    /// `asked` of its root folder, while it is held (none once let go of): a call on it as it is let go of is
    /// cancelled, and it is closed once the call ends.
    pub fn with_root<T>(&self, asked: impl FnOnce(HANDLE) -> T) -> Option<T> {
        let root = {
            let mut busy = self.held.busy.lock().unwrap();
            if self.held.left() {
                return None;
            }
            let root = busy.root?;
            busy.using += 1;
            root
        };
        let _using = Using { held: &self.held };
        Some(asked(root))
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
        self.held.ended.notify_all();
    }

    /// Whether its folders are found by their file ids (NTFS's, which stay with a file as it is renamed or moved);
    /// another drive's are found by their paths.
    pub fn by_id(&self) -> bool {
        self.system == "NTFS"
    }

    /// The names in `folder` (by its id where the drive's are kept, else by its path), `buffer` being where they are
    /// read. (Its root used through the listing whole: none is begun once it is let go of, and one under way then is
    /// cancelled.)
    pub fn list(&self, folder: &Folder, buffer: &mut [u64]) -> Listing {
        self.with_root(|root| self.list_from(root, folder, buffer)).unwrap_or_else(|| Listing::Failed(Error::from(ERROR_DEVICE_REMOVED)))
    }

    fn list_from(&self, root: HANDLE, folder: &Folder, buffer: &mut [u64]) -> Listing {
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
            Err(error) if gone(folder, &error) => return Listing::Gone,
            // (Held by another program that shares it with none, the drive not answering, and the like: listed again
            // after.)
            Err(error) => return Listing::Failed(error),
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

/// Whether `error`, as `folder` was opened, says it is not there any more: an id no longer of a file
/// (ERROR_INVALID_PARAMETER, its number's sequence moved on, or ERROR_FILE_NOT_FOUND), or a path not found. No other
/// failing says so.
fn gone(folder: &Folder, error: &Error) -> bool {
    let codes: &[WIN32_ERROR] = match folder {
        Folder::Id(_) => &[ERROR_INVALID_PARAMETER, ERROR_FILE_NOT_FOUND],
        Folder::Path(_) => &[ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND],
    };
    codes.iter().any(|code| error.code() == code.to_hresult())
}

/// The serial number of the volume `root` (a folder of it opened) is on, where it can be told.
pub fn serial(root: HANDLE) -> Option<u32> {
    let mut serial = 0u32;
    unsafe { GetVolumeInformationByHandleW(root, None, Some(&mut serial), None, None, None) }.ok()?;
    Some(serial)
}

/// A call using a drive's root (see `Drive::with_root`), counted till it ends: the last to end closes a root let go of.
struct Using<'a> {
    held: &'a Held,
}

impl Drop for Using<'_> {
    fn drop(&mut self) {
        let mut busy = self.held.busy.lock().unwrap();
        busy.using -= 1;
        if busy.using == 0 && self.held.left() {
            if let Some(root) = busy.root.take() {
                unsafe {
                    let _ = CloseHandle(root);
                }
            }
        }
        self.held.ended.notify_all();
    }
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
            assert!(busy.root.is_none() && busy.using == 0 && busy.watched.is_none() && busy.listing.is_empty());
        }
        assert!(matches!(drive.list(&Folder::Path(folder.clone()), &mut buffer()), Listing::Failed(_)));
        assert!(drive.watch().is_none() && drive.with_root(|_| ()).is_none());
        // (Nothing held: the folder can be removed.)
        drop(drive);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// Let go of while a call on its root does not end (a card reader that answers nothing, cancelled or not), a drive
    /// is let go of all the same within the time it waits: Windows' asking is answered (it waited on the call before,
    /// for as long as the call took), nothing more is begun on it, and its root is closed as that call ends.
    #[test]
    fn a_call_that_does_not_end_does_not_hold_the_letting_go() {
        let folder = std::env::temp_dir().join(format!("glance-let-go-stuck-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let drive = std::sync::Arc::new(Drive::open(&format!("{}\\", folder.display())).unwrap());
        let (begun, using) = std::sync::mpsc::channel();
        let (end, ended) = std::sync::mpsc::channel::<()>();
        let stuck = std::thread::spawn({
            let drive = drive.clone();
            move || {
                drive.with_root(|_| {
                    let _ = begun.send(());
                    let _ = ended.recv();
                })
            }
        });
        using.recv().unwrap();
        let (gone, let_go) = std::sync::mpsc::channel();
        std::thread::spawn({
            let drive = drive.clone();
            move || {
                drive.held.let_go_within(Duration::from_millis(300));
                let _ = gone.send(());
            }
        });
        let answered = let_go.recv_timeout(Duration::from_secs(3)).is_ok();
        let left = drive.left() && drive.with_root(|_| ()).is_none();
        let still_open = drive.held.busy.lock().unwrap().root.is_some();
        // (The call ended, whatever was found: no thread is left waiting.)
        end.send(()).unwrap();
        stuck.join().unwrap();
        assert!(answered, "let go of within the time it waits");
        assert!(left && still_open, "nothing more begun on it, its root still used");
        assert!(drive.held.busy.lock().unwrap().root.is_none(), "closed as the call ended");
        drop(drive);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// Only a folder that is not there any more is gone, by its id or its path: one that cannot be opened now for
    /// another reason (held by a program that shares it with none) failed, to be listed again, where it was taken for
    /// gone and what was not yet listed of it was lost.
    #[test]
    fn only_a_folder_not_there_any_more_is_gone() {
        let folder = std::env::temp_dir().join(format!("glance-gone-{}", std::process::id()));
        let inner = folder.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("a.txt"), "a").unwrap();
        let drive = Drive::open(&format!("{}\\", folder.display())).unwrap();
        let Listing::Items(found) = drive.list(&Folder::Path(folder.clone()), &mut buffer()) else { panic!("listed") };
        let id = found.iter().find(|item| item.name == "inner".encode_utf16().collect::<Vec<_>>()).unwrap().id;
        let listing = |folder: &Folder| drive.list(folder, &mut buffer());
        let (by_id, by_path) = (Folder::Id(id), Folder::Path(inner.clone()));

        let held = unsafe { CreateFileW(&HSTRING::from(inner.as_os_str()), LIST, Default::default(), None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) }.unwrap();
        let shared_with_none = [listing(&by_id), listing(&by_path)];
        unsafe {
            let _ = CloseHandle(held);
        }
        assert!(shared_with_none.iter().all(|listing| matches!(listing, Listing::Failed(_))), "held by another, it failed, to be listed again");
        assert!([listing(&by_id), listing(&by_path)].iter().all(|listing| matches!(listing, Listing::Items(items) if items.len() == 1)));

        std::fs::remove_dir_all(&inner).unwrap();
        assert!(matches!(listing(&by_id), Listing::Gone) && matches!(listing(&by_path), Listing::Gone), "gone");
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
