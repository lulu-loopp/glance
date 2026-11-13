//! Running as administrator without asking every time, starting with
//! Windows, and the PawnIO driver the sensors need.
//!
//! The driver only answers administrators. Windows asks the user to confirm
//! every elevated start, except for scheduled tasks set to run with highest
//! privileges: so Glance's sensors (`glance-sensors.exe`, beside
//! `glance.exe`; see `sensing`), once elevated, register such a task for
//! themselves, and Glance, which runs as the user, starts them through it.
//! The user confirms once, on the first start. Glance itself starts at
//! sign-in through a task that runs it as the user.
//!
//! A task like that runs whatever file is at its path, elevated, whenever
//! anyone running as the user asks. So Glance only makes one for an
//! executable no one but administrators can change: one installed under
//! Program Files, not a copy in Downloads or a build folder. From anywhere
//! else every start asks.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use windows::core::{w, Interface, BSTR, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetAce, GetTokenInformation, MapGenericMask, TokenElevation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, CONTAINER_INHERIT_ACE, GENERIC_MAPPING, INHERIT_ONLY_ACE, OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, FileDispositionInfo, GetFinalPathNameByHandleW, SetFileInformationByHandle, DELETE,
    FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DELETE_CHILD, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, SYNCHRONIZE,
    WRITE_DAC, WRITE_OWNER,
};
use windows::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE,
    REG_DWORD, REG_SZ,
    REG_OPTION_NON_VOLATILE, RRF_RT_REG_SZ,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, IRegisteredTask, ITaskFolder, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE, TASK_ENUM_HIDDEN,
    TASK_LOGON_INTERACTIVE_TOKEN,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::ShellExecuteW;

/// Glance's two programs, side by side in its folder: Glance itself, which
/// runs as the user, and its sensors, which run elevated.
pub const APP: &str = "glance.exe";
pub const SENSORS: &str = "glance-sensors.exe";
/// The folder in Glance's own an OpenRGB put there with it is in (see `openrgb`).
pub const OPENRGB: &str = "OpenRGB";

/// The task that starts the sensors, and the one that starts Glance when the
/// user signs in.
const LAUNCH_TASK: &str = "Glance";
const LOGON_TASK: &str = "Glance at sign-in";
/// The task that runs the phone remote (another program, never elevated)
/// at sign-in and while it is on.
const REMOTE_TASK: &str = "Glance Remote";
const NO_WINDOW: u32 = 0x0800_0000;

/// Where Glance notes that it installed PawnIO itself, so that uninstalling
/// Glance offers to remove the driver only then.
const GLANCE_KEY: PCWSTR = w!(r"SOFTWARE\Glance");
const INSTALLED_PAWNIO: PCWSTR = w!("InstalledPawnIO");
const PAWNIO_UNINSTALL_KEY: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO");

pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(token) };
    ok && elevation.TokenIsElevated != 0
}

// ---- Who can change a file ----

/// The accounts that may change installed programs: SYSTEM, the
/// Administrators group and TrustedInstaller.
const TRUSTED: [&str; 3] = ["S-1-5-18", "S-1-5-32-544", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];
/// CREATOR OWNER: on an entry passed on, whoever creates the new object.
const CREATOR_OWNER: &str = "S-1-3-0";

fn sid_string(sid: PSID) -> Option<String> {
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.ok()?;
    let string = unsafe { text.to_string() }.ok();
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    string
}

/// Whether only trusted accounts can do any of `forbidden` to `path`: it is
/// owned by one (an owner may always re-permission), and its access list
/// grants those rights to no one else. Fails closed on anything unexpected.
fn only_trusted_can(path: &Path, forbidden: u32) -> bool {
    only_trusted_can_now_and_after(path, forbidden, false)
}

/// As `only_trusted_can`; with `passed_on`, what the folder passes on to
/// anything new in it must grant `forbidden` to no one else either. There
/// the creator's own rights (CREATOR OWNER) count as trusted: only someone
/// trusted can create in a folder that holds.
fn only_trusted_can_now_and_after(path: &Path, forbidden: u32, passed_on: bool) -> bool {
    let wide = HSTRING::from(path.as_os_str());
    let (mut owner, mut dacl, mut descriptor) = (PSID::default(), std::ptr::null_mut::<ACL>(), PSECURITY_DESCRIPTOR::default());
    let read = unsafe {
        GetNamedSecurityInfoW(
            &wide,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if read.is_err() {
        return false;
    }
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: FILE_ALL_ACCESS.0,
    };
    let trusted = |sid: PSID| sid_string(sid).is_some_and(|sid| TRUSTED.contains(&sid.as_str()));
    // No access list at all lets everyone do everything.
    let mut clear = trusted(owner) && !dacl.is_null();
    if clear {
        for index in 0..unsafe { (*dacl).AceCount } as u32 {
            let mut ace = std::ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut ace) }.is_err() {
                clear = false;
                break;
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            let flags = header.AceFlags as u32;
            let passes_on = flags & (OBJECT_INHERIT_ACE.0 | CONTAINER_INHERIT_ACE.0) != 0;
            // Entries that only pass on to children do not apply here.
            if flags & INHERIT_ONLY_ACE.0 != 0 && !passed_on {
                continue;
            }
            match header.AceType as u32 {
                ACCESS_DENIED_ACE_TYPE => {}
                ACCESS_ALLOWED_ACE_TYPE => {
                    let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
                    let sid = PSID(&allowed.SidStart as *const u32 as *mut _);
                    let mut mask = allowed.Mask;
                    unsafe { MapGenericMask(&mut mask, &mapping) };
                    let creator = passed_on && passes_on && sid_string(sid).as_deref() == Some(CREATOR_OWNER);
                    if mask & forbidden != 0 && !trusted(sid) && !creator {
                        clear = false;
                        break;
                    }
                }
                // Conditional and object entries are not looked into.
                _ => {
                    clear = false;
                    break;
                }
            }
        }
    }
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    clear
}

/// Where `file` really is, if no one but administrators can change it or
/// put anything else in its place: alter or replace it, add files beside it
/// (which Windows might load with it), or rename or re-permission any folder
/// above it. Checked where the file really is, through any links and
/// junctions; that path, not the one given, is what may then be run.
fn checked(file: &Path) -> Option<PathBuf> {
    let file = std::fs::canonicalize(file).ok()?;
    let mut clear = only_trusted_can(&file, (FILE_WRITE_DATA | FILE_APPEND_DATA | DELETE | WRITE_DAC | WRITE_OWNER).0);
    if let Some(own) = file.parent() {
        // Its own folder takes no new files.
        clear &= only_trusted_can(own, (FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY).0) && folders_hold(own);
    }
    clear.then_some(file)
}

/// Whether no one but administrators can rename `folder` or any folder
/// above it, take one of their children away, or re-permission them.
fn folders_hold(folder: &Path) -> bool {
    folder.ancestors().all(|dir| {
        // A drive's root cannot be renamed or deleted: its own Delete right
        // (which ordinary users hold on a second drive) changes nothing.
        let delete = if dir.parent().is_some() { DELETE.0 } else { 0 };
        only_trusted_can(dir, delete | (FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER).0)
    })
}

/// What an install folder may already hold: an earlier Glance (and what an install of it leaves while it runs, or
/// if it was cut short).
const INSTALLED: [&str; 13] = [APP, SENSORS, "glance-remote.exe", "uninstall.exe", "resources", "licenses", OPENRGB, INCOMING, REPLACED, DONE_WITH, TASKS_OFF, TASKS_OFF_NEW, INSTALLING];
/// What an install leaves while it runs (or left, cut short): none of it is an earlier Glance's own files.
const UNDER_WAY: [&str; 6] = [INCOMING, REPLACED, DONE_WITH, TASKS_OFF, TASKS_OFF_NEW, INSTALLING];
/// Held, sharing nothing, by the install under way, for the whole of it: another finds it held, and does nothing.
/// (Gone as it is let go, even by an install ended part way.)
const INSTALLING: &str = ".installing";
/// The earlier files moved aside (`REPLACED`) once every new one is in place: no longer to be put back, only taken
/// away. (Taken away from `REPLACED` itself, an install cut short in the middle of it left some of them, and the
/// next put those back over their new ones.)
const DONE_WITH: &str = ".done-with";
/// Where an install puts the new files, all of them, before any goes in place.
const INCOMING: &str = ".incoming";
/// Where an install moves the files it replaces, till all the new ones are in place (put back if one could not be).
const REPLACED: &str = ".replaced";
/// The tasks an install has turned off while it runs, by name, a line each: turned on again after, even if the
/// install was cut short (see `TasksOff`).
const TASKS_OFF: &str = ".tasks-off";
/// The record of the tasks turned off as it is written, before it takes the place of the last (see `write_down`).
const TASKS_OFF_NEW: &str = ".tasks-off.new";

/// What a folder chosen to install into is, or why installing failed.
#[derive(Debug, PartialEq)]
pub enum InstallFolder {
    /// Glance can be installed there and start elevated without asking: no
    /// one but administrators can change any folder above it, and the folder
    /// is new (created administrators' from the start) or an earlier
    /// Glance's that only administrators could ever change.
    Holds,
    /// The same, except that a folder above it can be changed by others:
    /// installed there at the user's choice, Glance asks for administrator
    /// rights at every start and cannot start with Windows.
    Open,
    /// It exists and is not an earlier Glance's that only administrators
    /// could change: what is in it cannot be vouched for, and writing into it
    /// with administrator rights is not safe.
    Occupied,
    /// It, or a folder above it, is a link or junction, or it is a drive's
    /// root: what it leads to is not what was chosen.
    Indirect,
    /// The files could not be put there or taken away.
    Failed,
}

/// The folder as chosen, if it is where it says: no part of it a link or
/// junction (what exists of it resolves to itself), and not a drive's root.
fn direct(folder: &Path) -> Option<PathBuf> {
    let folder = std::path::absolute(folder).ok()?;
    folder.parent()?;
    let existing = folder.ancestors().find(|dir| dir.exists())?;
    let resolved = std::fs::canonicalize(existing).ok()?;
    (same_path(&resolved, existing)).then_some(folder)
}

/// Whether two paths name the same place, case aside, and whether written
/// plainly or as Windows answers by handle (\\?\C:\…, \\?\UNC\server\…).
fn same_path(a: &Path, b: &Path) -> bool {
    let plain = |path: &Path| {
        let text = path.to_string_lossy();
        let text = match text.strip_prefix(r"\\?\UNC\") {
            Some(share) => format!(r"\\{share}"),
            None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
        };
        text.trim_end_matches('\\').to_lowercase()
    };
    plain(a) == plain(b)
}

/// Where the user's own `folder` (their application data) really is, through any link or junction it is reached by:
/// a profile, or its application data, moved to another drive by one leads there. What Glance keeps under it is read
/// and written from there, each folder below checked to be where its path says (see `read_in_place`): checked from
/// the path as given, a folder reached through a link above it was never where its path said, and Glance neither
/// read nor kept anything (its settings were as new at every start). Following what leads to the folder itself gives
/// no one a way in that they had not: where it is, is the user's to say (Windows takes it from their own registry).
/// As given, where it cannot be found.
pub fn where_it_is(folder: &Path) -> PathBuf {
    std::fs::canonicalize(folder).map_or_else(|_| folder.to_path_buf(), |found| plain(&found))
}

/// Whether `folder` holds nothing but an earlier Glance.
fn only_ours(folder: &Path) -> bool {
    let ours = |name: &str| INSTALLED.iter().any(|known| known.eq_ignore_ascii_case(name));
    std::fs::read_dir(folder).is_ok_and(|entries| entries.flatten().all(|entry| ours(&entry.file_name().to_string_lossy())))
}

/// Whether what Glance puts in `folder` and later removes (its licenses
/// folder goes whole) can be no one else's: none of it is there yet, or the
/// folder is an earlier Glance's alone (its program and uninstaller there,
/// nothing else beside them).
fn claimable(folder: &Path) -> bool {
    let ours_there = INSTALLED.iter().any(|name| folder.join(name).exists());
    // (An install cut short may have moved them aside, or not have come to them: see `install_into`.)
    let whole = ["glance.exe", "uninstall.exe"].iter().all(|name| folder.join(name).is_file()) || UNDER_WAY.iter().any(|name| folder.join(name).exists());
    let earlier = whole && only_ours(folder);
    !ours_there || earlier
}

/// Whether `path` is a link, junction or other reparse point (not followed).
fn is_link(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    path.symlink_metadata().map_or(true, |meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

/// A file or folder held open by handle, not followed if it is a link.
/// While a folder is held, neither it nor any folder above it can be
/// renamed, moved or deleted (Windows refuses while something below is open
/// without delete sharing), so its path leads where it was checked to.
struct Held(std::fs::File);

impl Held {
    /// Opens `path` itself (a link as the link), able to delete it.
    fn open(path: &Path) -> Option<Self> {
        Self::open_with(path, DELETE.0)
    }

    /// Opens `path` itself (a link as the link), to look at and hold only.
    fn open_to_hold(path: &Path) -> Option<Self> {
        Self::open_with(path, 0)
    }

    fn open_with(path: &Path, rights: u32) -> Option<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .access_mode((FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE).0 | rights)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
            .open(path)
            .ok()
            .map(Held)
    }

    /// Where it really is, as Windows resolved it when it was opened.
    fn place(&self) -> Option<PathBuf> {
        use std::os::windows::io::AsRawHandle;
        let mut buffer = vec![0u16; 1024];
        let handle = HANDLE(self.0.as_raw_handle());
        let length = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) } as usize;
        (length > 0 && length < buffer.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
    }

    /// What the file system knows it by: its volume's serial number, and its number on that volume.
    fn id(&self) -> Option<(u32, u64)> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(self.0.as_raw_handle()), &mut information) }.ok()?;
        Some((information.dwVolumeSerialNumber, (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow)))
    }

    fn is_folder(&self) -> bool {
        use std::os::windows::fs::MetadataExt;
        self.0.metadata().is_ok_and(|meta| meta.is_dir() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0)
    }

    /// Whether it lies directly in the folder `parent` holds.
    /// Takes the name `path` (a full path), replacing what is there.
    fn rename_to(&self, path: &Path) -> std::io::Result<()> {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows::Win32::Storage::FileSystem::{FileRenameInfo, FILE_RENAME_INFO, FILE_RENAME_INFO_0};
        let name: Vec<u16> = path.as_os_str().encode_wide().collect();
        // The structure, its name running on past its end.
        let header = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = header + (name.len() + 1) * 2;
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let info = buffer.as_mut_ptr() as *mut FILE_RENAME_INFO;
        unsafe {
            (*info).Anonymous = FILE_RENAME_INFO_0 { ReplaceIfExists: true };
            (*info).RootDirectory = HANDLE::default();
            (*info).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(name.as_ptr(), std::ptr::addr_of_mut!((*info).FileName) as *mut u16, name.len());
            SetFileInformationByHandle(HANDLE(self.0.as_raw_handle()), FileRenameInfo, info as *const _, size as u32)?;
        }
        Ok(())
    }

    fn lies_in(&self, parent: &Held) -> bool {
        let (Some(me), Some(parent)) = (self.place(), parent.place()) else { return false };
        me.parent().is_some_and(|mine| same_path(mine, &parent))
    }

    /// Deletes what is held (a link as the link; a folder only if empty)
    /// once it is let go.
    fn delete(self) -> bool {
        use std::os::windows::io::AsRawHandle;
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        let handle = HANDLE(self.0.as_raw_handle());
        unsafe {
            SetFileInformationByHandle(handle, FileDispositionInfo, (&disposition as *const FILE_DISPOSITION_INFO).cast(), size_of::<FILE_DISPOSITION_INFO>() as u32)
        }
        .is_ok()
    }
}

/// Holds `folder` open, if it is a folder and not a link, able to delete it: for a folder made here (see
/// `create_held`), or one deleted by its handle.
fn pin(folder: &Path) -> Option<Held> {
    Held::open(folder).filter(Held::is_folder)
}

/// Holds `folder` open, if it is a folder and not a link, without the right to delete it: for a folder that was
/// there already. Held so, it can be neither moved nor deleted by anyone else (it shares no deletion), and holding it
/// is not refused because another program has it open as it may (as its current folder: the Glance an install
/// replaces, a console), which asking to delete it is (ERROR_SHARING_VIOLATION).
fn hold(folder: &Path) -> Option<Held> {
    Held::open_to_hold(folder).filter(Held::is_folder)
}

/// Which folder `path` is (its volume's serial number and its number there): another made in its place, another.
/// (Looked at only: opened sharing all, held not even for a moment against its being renamed.)
pub fn folder_id(path: &Path) -> Option<(u32, u64)> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
    let opened = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .ok()?;
    Held(opened).id()
}

/// Rights over a folder that would let someone change what is in it, or
/// make it a link (setting a reparse point needs only to write attributes).
const FOLDER_CHANGES: u32 = FILE_ADD_FILE.0
    | FILE_ADD_SUBDIRECTORY.0
    | FILE_DELETE_CHILD.0
    | DELETE.0
    | WRITE_DAC.0
    | WRITE_OWNER.0
    | FILE_WRITE_ATTRIBUTES.0;
/// Rights over a file that would let someone change it.
const FILE_CHANGES: u32 = FILE_WRITE_DATA.0 | FILE_APPEND_DATA.0 | DELETE.0 | WRITE_DAC.0 | WRITE_OWNER.0 | FILE_WRITE_ATTRIBUTES.0;

/// Whether only administrators can change `folder` and everything in it,
/// none of it a link. Then no one else can have changed it, or hold it open
/// to change it now.
fn only_trusted_inside(folder: &Path) -> bool {
    !is_link(folder)
        // What it passes on to what is put into it counts too.
        && only_trusted_can_now_and_after(folder, FOLDER_CHANGES | FILE_CHANGES, true)
        && std::fs::read_dir(folder).is_ok_and(|entries| {
            entries.into_iter().all(|entry| {
                entry.is_ok_and(|entry| {
                    let path = entry.path();
                    if is_link(&path) {
                        false
                    } else if path.is_dir() {
                        only_trusted_inside(&path)
                    } else {
                        only_trusted_can(&path, FILE_CHANGES)
                    }
                })
            })
        })
}

/// Where `folder` really is, if no one but administrators can change it or anything in it (none of it a link), nor
/// move it or any folder above it aside (see `only_trusted_inside`, `folders_hold`): what is in it may then be run with
/// an administrator's rights, from that path.
pub fn vouched_folder(folder: &Path) -> Option<PathBuf> {
    let real = plain(&std::fs::canonicalize(folder).ok()?);
    (only_trusted_inside(&real) && folders_hold(&real)).then_some(real)
}

/// Whether every folder above `folder` holds (see `checked`).
fn above_holds(folder: &Path) -> bool {
    let above = folder.parent().and_then(|above| std::fs::canonicalize(above).ok());
    above.is_some_and(|above| folders_hold(&above))
}

pub fn install_folder(folder: &Path) -> InstallFolder {
    let Some(folder) = direct(folder) else { return InstallFolder::Indirect };
    if folder.exists() && !(claimable(&folder) && only_trusted_inside(&folder)) {
        return InstallFolder::Occupied;
    }
    if above_holds(&folder) {
        InstallFolder::Holds
    } else {
        InstallFolder::Open
    }
}

/// Creates `folder` (whose parent `parent` holds) with `attributes`, and
/// holds it. What was created is checked by handle to lie in `parent`; if
/// it went elsewhere (a link put in the way), it is removed again by that
/// handle, and nothing is left behind.
fn create_held(folder: &Path, parent: &Held, attributes: Option<&SECURITY_ATTRIBUTES>) -> Result<Held, InstallFolder> {
    unsafe { CreateDirectoryW(&HSTRING::from(folder.as_os_str()), attributes.map(|a| a as *const _)) }
        .map_err(|_| InstallFolder::Occupied)?;
    let made = pin(folder).ok_or(InstallFolder::Indirect)?;
    if !made.lies_in(parent) {
        made.delete();
        return Err(InstallFolder::Indirect);
    }
    // Held from here as a folder that was there already is (`hold`), without the right to delete it: held able to
    // delete it, every name put in it by a rename was refused on Windows 10 (the rename opens the folder to add the
    // name, sharing no deletion: ERROR_SHARING_VIOLATION, see `held_where_it_says`), and a first install, which
    // makes the folder, failed there at its first rename. The same folder, by its id, still in `parent`.
    let id = made.id();
    drop(made);
    hold(folder).filter(|held| id.is_some() && held.id() == id && held.lies_in(parent)).ok_or(InstallFolder::Indirect)
}

/// `create_held` with the security descriptor `sddl`, set in the one step
/// that creates the folder.
fn create_held_as(folder: &Path, parent: &Held, sddl: PCWSTR) -> Result<Held, InstallFolder> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut descriptor, None) }.map_err(|_| InstallFolder::Failed)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let made = create_held(folder, parent, Some(&attributes));
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    made
}

/// A new folder that only administrators and the system can open, held: no
/// one else can put anything in it, change what is put there, or move it
/// (or a folder above it) aside while it is held.
pub struct Private {
    _held: Held,
    path: PathBuf,
}

impl Private {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Makes a `Private` folder `name` in `parent`, where `parent` really is
/// (as Windows resolves it, held): new (it must not exist yet), and checked
/// by handle to lie in that held folder. Only in a parent that no one but
/// administrators can take children from or re-permission, nor any folder
/// above it (`folders_hold`): then no one else can move the new folder
/// aside, from the moment it is made until long after it is let go.
pub fn private_folder(parent: &Path, name: &str) -> Option<Private> {
    let parent_held = hold(parent)?;
    let real = plain(&parent_held.place()?);
    if !folders_hold(&real) {
        return None;
    }
    let path = real.join(name);
    let held = create_held_as(&path, &parent_held, w!("O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)")).ok()?;
    Some(Private { _held: held, path })
}

/// Removes the folders in `parent` whose names begin with `prefix`, each with all that is in it: those of
/// administrators' alone, as `private_folder` made them. Another's that took such a name stays.
pub fn remove_private_folders(parent: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with(prefix) && !is_link(&path) && path.is_dir() && only_trusted_can(&path, FOLDER_CHANGES) {
            let _ = remove_all_kept(&path);
        }
    }
}

/// The key noting the update installers downloaded and not yet removed,
/// under Glance's, which only administrators can write: a value each, named
/// after its folder (new each time), holding the installer's path. Each is
/// added and removed on its own, so Glances in two sessions at once never
/// write over each other's.
const UPDATE_DOWNLOADS: PCWSTR = w!(r"SOFTWARE\Glance\UpdateDownloads");

/// Notes `file` (in a `Private` folder) as an update installer downloaded,
/// for `forget_download` to remove later. Noted before it is written, so
/// that nothing Glance puts there goes unrecorded.
pub fn remember_download(file: &Path) -> bool {
    let Some(name) = file.parent().and_then(Path::file_name) else { return false };
    let mut key = HKEY::default();
    let created = unsafe { RegCreateKeyExW(HKEY_LOCAL_MACHINE, UPDATE_DOWNLOADS, None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None) };
    if created.is_err() {
        return false;
    }
    let text: Vec<u16> = file.as_os_str().encode_wide().chain([0]).collect();
    let bytes: Vec<u8> = text.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let written = unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&bytes)) }.is_ok();
    unsafe {
        let _ = RegCloseKey(key);
    }
    written
}

/// Removes each update installer `remember_download` noted, and its folder,
/// one file and one empty folder each, nothing else. A file still in use
/// (its installer running) stays noted for next time; a folder no longer
/// administrators' alone is no longer the one Glance made (that one was
/// removed, and someone else's made in its place) and is left alone, its
/// note dropped.
pub fn forget_download() {
    for (name, file) in noted() {
        let gone = match file.parent() {
            Some(folder) if folder.exists() && !is_link(folder) && only_trusted_can(folder, FOLDER_CHANGES) => {
                (!file.exists() || std::fs::remove_file(&file).is_ok()) && std::fs::remove_dir(folder).is_ok()
            }
            _ => true,
        };
        if gone {
            let mut key = HKEY::default();
            if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, UPDATE_DOWNLOADS, None, KEY_WRITE, &mut key) }.is_ok() {
                unsafe {
                    let _ = RegDeleteValueW(key, &HSTRING::from(name.as_str()));
                    let _ = RegCloseKey(key);
                }
            }
        }
    }
}

/// The update installers noted, by the name of their note, in order of it.
fn noted() -> Vec<(String, PathBuf)> {
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, UPDATE_DOWNLOADS, None, KEY_READ, &mut key) }.is_err() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        let mut data = [0u16; 1024];
        let mut size = std::mem::size_of_val(&data) as u32;
        let mut kind = 0u32;
        let read = unsafe {
            RegEnumValueW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut length, None, Some(&mut kind), Some(data.as_mut_ptr().cast()), Some(&mut size))
        };
        if read.is_err() {
            break;
        }
        if kind != REG_SZ.0 {
            continue;
        }
        let text = String::from_utf16_lossy(&data[..(size as usize / 2)]);
        found.push((String::from_utf16_lossy(&name[..length as usize]), PathBuf::from(text.trim_end_matches('\0'))));
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    found.sort();
    found
}

#[cfg(test)]
fn noted_downloads() -> Vec<PathBuf> {
    noted().into_iter().map(|(_, file)| file).collect()
}

/// Puts the files the installer laid out in `payload` into `folder`. A new
/// folder is created owned by the administrators, changed only by them and
/// the system, and read by everyone (as Program Files' folders are), in the
/// one step that creates it; an earlier Glance's folder that only
/// administrators could change is used as it is. Only where every folder
/// above holds too: nowhere else, whatever the user would choose (installed
/// where others could swap it, the copy run as administrator was theirs to
/// replace).
///
/// Every folder on the way is held before the next is made or used: then
/// nothing above can be swapped for a link, and every folder made is
/// checked by handle to be where it was meant to be. The install folder is
/// administrators' alone, so nothing in it can be changed by anyone else:
/// the files land where they were meant to and nowhere else.
pub fn install_into(folder: &Path, payload: &Path) -> Result<(), InstallFolder> {
    match install_folder(folder) {
        InstallFolder::Holds => {}
        other => return Err(other),
    }
    let folder = direct(folder).ok_or(InstallFolder::Indirect)?;
    let existing = folder.ancestors().find(|dir| dir.exists()).ok_or(InstallFolder::Failed)?.to_path_buf();
    let mut held = vec![hold(&existing).ok_or(InstallFolder::Indirect)?];
    if !same_path(&held[0].place().ok_or(InstallFolder::Failed)?, &existing) {
        return Err(InstallFolder::Indirect);
    }
    // The folders between it and the install folder, as they come, then the install folder, administrators'.
    let missing: Vec<&Path> = folder.ancestors().take_while(|dir| *dir != existing.as_path()).collect();
    for dir in missing.iter().rev() {
        let parent = held.last().unwrap();
        let next = if *dir == folder.as_path() {
            create_held_as(dir, parent, w!("O:BAD:PAI(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;BU)"))?
        } else {
            create_held(dir, parent, None)?
        };
        held.push(next);
    }
    // Checked again with all of it held.
    if direct(&folder).is_none() {
        return Err(InstallFolder::Indirect);
    }
    // One install at a time: another under way, this one does nothing (two at once undid each other's files).
    let _installing = installing(&folder).map_err(|_| InstallFolder::Failed)?;
    if !(claimable(&folder) && only_trusted_inside(&folder)) {
        return Err(InstallFolder::Occupied);
    }
    if !above_holds(&folder) {
        return Err(InstallFolder::Open);
    }
    // Nothing of Glance's runs from it while its files are replaced, and nothing starts one meanwhile; its tasks
    // as they were once this is done (or has failed).
    let _tasks = TasksOff::now(&folder).ok_or(InstallFolder::Failed)?;
    if !end_programs_in(&folder) {
        return Err(InstallFolder::Failed);
    }
    put_in_place(payload, &folder).map_err(|_| InstallFolder::Failed)
}

/// Holds `INSTALLING` in `folder`, sharing nothing, owned by the Administrators group (see `own_as_administrators`):
/// till it is dropped, no other install there can. An error where another holds it.
fn installing(folder: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Security::Authorization::{ConvertStringSidToSidW, SetSecurityInfo};
    let held = held_alone(&folder.join(INSTALLING))?;
    let mut administrators = PSID::default();
    unsafe { ConvertStringSidToSidW(w!("S-1-5-32-544"), &mut administrators) }.map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
    let set = unsafe { SetSecurityInfo(HANDLE(held.as_raw_handle()), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, Some(administrators), None, None, None) };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(administrators.0)));
    }
    if set.is_err() {
        return Err(std::io::Error::from_raw_os_error(set.0 as i32));
    }
    Ok(held)
}

/// `path` made (or opened) and held sharing nothing, able to change its owner; gone once let go, however the holder
/// ends. An error where another holds it.
fn held_alone(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_FLAG_DELETE_ON_CLOSE;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .access_mode((FILE_GENERIC_WRITE | DELETE | WRITE_OWNER).0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE.0)
        .open(path)
}

/// Puts the files in `payload` in `folder`, all of them or none: each copied in beside them first (`INCOMING`), then
/// each in place of its earlier one, which is moved aside (`REPLACED`) till all are, and put back if one could not
/// be. (Copied over one by one, an install that failed halfway, out of room or at a file held, left programs of two
/// versions, or one copied in part.) What an install cut short left is put back as it was first.
fn put_in_place(payload: &Path, folder: &Path) -> std::io::Result<()> {
    let (incoming, replaced) = (folder.join(INCOMING), folder.join(REPLACED));
    put_back(folder)?;
    remove(&folder.join(DONE_WITH))?;
    remove(&incoming)?;
    std::fs::create_dir(&incoming)?;
    own_as_administrators(&incoming)?;
    if let Err(error) = copy_into(payload, &incoming) {
        let _ = remove(&incoming);
        return Err(error);
    }
    std::fs::create_dir(&replaced)?;
    own_as_administrators(&replaced)?;
    swap_in(folder)
}

/// Puts each file in `INCOMING` in its place in `folder`, the earlier one moved aside into `REPLACED` (both there);
/// all of them, or, one not put in place, none (see `put_in_place`).
fn swap_in(folder: &Path) -> std::io::Result<()> {
    let (incoming, replaced) = (folder.join(INCOMING), folder.join(REPLACED));
    let names: Vec<std::ffi::OsString> = std::fs::read_dir(&incoming)?.map(|entry| entry.map(|entry| entry.file_name())).collect::<std::io::Result<_>>()?;
    // Those new here (no earlier one to put back), as they are put in place.
    let mut new = Vec::new();
    for name in &names {
        let target = folder.join(name);
        let earlier = target.symlink_metadata().is_ok();
        let placed = (|| {
            if earlier {
                patiently(|| std::fs::rename(&target, replaced.join(name)))?;
            }
            patiently(|| std::fs::rename(incoming.join(name), &target))
        })();
        if !earlier && placed.is_ok() {
            new.push(target);
        }
        if let Err(error) = placed {
            // As it was: what was new taken out, what was moved aside back.
            for target in &new {
                let _ = remove(target);
            }
            let _ = put_back(folder);
            let _ = remove(&incoming);
            return Err(error);
        }
    }
    // All in place: the earlier ones no longer to be put back (moved at once, as one), then taken away. (What is
    // left of them, were they not taken now, would be by the next install.)
    let done_with = folder.join(DONE_WITH);
    remove(&done_with)?;
    patiently(|| std::fs::rename(&replaced, &done_with))?;
    let _ = remove(&done_with);
    let _ = remove(&incoming);
    Ok(())
}

/// Puts each file moved aside by an install (in `REPLACED`) back in its place, in place of what took it, if any;
/// then nothing is left aside.
fn put_back(folder: &Path) -> std::io::Result<()> {
    let replaced = folder.join(REPLACED);
    if !replaced.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&replaced)? {
        let entry = entry?;
        let target = folder.join(entry.file_name());
        remove(&target)?;
        patiently(|| std::fs::rename(entry.path(), &target))?;
    }
    remove(&replaced)
}

/// Takes `path` away, a folder with all in it (never following a link out of it); none there is taken away already.
fn remove(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => patiently(|| std::fs::remove_dir_all(path)),
        Ok(_) => patiently(|| std::fs::remove_file(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The programs Glance installs that run.
const PROGRAMS: [&str; 3] = [APP, SENSORS, "glance-remote.exe"];
/// How long a program of Glance's that was ended is waited for to be gone.
const ENDED_WITHIN_MS: u32 = 15_000;

/// Ends every program of Glance's running from `folder` (every account's), and waits till each is gone; whether
/// none is left. Those looked at are only those of Glance's names (no other process is opened, a game's least of
/// all), and ended only if their file is in `folder`: another copy of Glance elsewhere is left alone. One started
/// meanwhile is found by the next look.
fn end_programs_in(folder: &Path) -> bool {
    use windows::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    let Ok(folder) = std::fs::canonicalize(folder) else { return true };
    loop {
        let running = running(|file| file.parent().and_then(|parent| std::fs::canonicalize(parent).ok()).is_some_and(|parent| parent == folder));
        if running.is_empty() {
            return true;
        }
        // All ended first, then waited for: none is left a moment to notice another gone (and start it again).
        // (One already ending refuses to be ended, and is waited for all the same.)
        for &process in &running {
            let _ = unsafe { TerminateProcess(process, 1) };
        }
        let mut gone = true;
        for process in running {
            unsafe {
                gone &= WaitForSingleObject(process, ENDED_WITHIN_MS) == WAIT_OBJECT_0;
                let _ = CloseHandle(process);
            }
        }
        if !gone {
            return false;
        }
    }
}

/// The processes of Glance's programs whose file is as `ours` wants (its path as Windows names it), opened to be ended
/// and waited for.
fn running(ours: impl Fn(&Path) -> bool) -> Vec<HANDLE> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if PROGRAMS.iter().any(|program| program.eq_ignore_ascii_case(&name)) {
                if let Ok(process) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID) {
                    let mut path = vec![0u16; 1024];
                    let mut size = path.len() as u32;
                    let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut size).is_ok();
                    let file = PathBuf::from(String::from_utf16_lossy(&path[..size as usize]));
                    if named && ours(&file) {
                        found.push(process);
                    } else {
                        let _ = CloseHandle(process);
                    }
                }
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

/// Glance's tasks (every account's) that were on, turned off while Glance's files are replaced in `.1` (none starts
/// a Glance meanwhile); on again once this is dropped. Each is written down in the install folder (`TASKS_OFF`)
/// before it is turned off, so that an install cut short (Windows restarted, the installer ended) leaves none off for
/// good: the next install, or the sensors as they start, turn them on again (see `tasks_left_off`).
struct TasksOff(Vec<String>, PathBuf);

impl TasksOff {
    /// None where what is turned off could not be written down first.
    fn now(install: &Path) -> Option<Self> {
        let folder = task_folder()?;
        let on = |name: &str| unsafe { folder.GetTask(&BSTR::from(name)).is_ok_and(|task| task.Enabled().is_ok_and(|on| on.as_bool())) };
        // Those an install cut short left off, with those on now.
        let mut off = left_off(install);
        let now_on: Vec<String> = glance_tasks().into_iter().filter(|name| on(name) && !off.contains(name)).collect();
        off.extend(now_on);
        write_down(install, &off).ok()?;
        for name in &off {
            if let Ok(task) = unsafe { folder.GetTask(&BSTR::from(name.as_str())) } {
                let _ = unsafe { task.SetEnabled(windows::Win32::Foundation::VARIANT_FALSE) };
            }
        }
        Some(TasksOff(off, install.to_path_buf()))
    }
}

impl Drop for TasksOff {
    fn drop(&mut self) {
        turn_on(&self.0);
        let _ = remove(&self.1.join(TASKS_OFF));
    }
}

/// The tasks an install of Glance in `install` turned off and has not turned on again.
fn left_off(install: &Path) -> Vec<String> {
    read_in_place(install, TASKS_OFF, 64 * 1024).map(|names| names.lines().filter(|name| is_glance_task(name)).map(str::to_string).collect()).unwrap_or_default()
}

/// Writes down `names` as the tasks an install in `install` turns off, on the disk before anything is: written
/// beside the record first, then put in its place whole (written over in place, an install ended meanwhile left it
/// empty, and the tasks it named off for good), owned by the Administrators group as the rest is.
fn write_down(install: &Path, names: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let beside = install.join(TASKS_OFF_NEW);
    let mut file = std::fs::File::create(&beside)?;
    file.write_all(names.join("\n").as_bytes())?;
    file.sync_all()?;
    drop(file);
    own_as_administrators(&beside)?;
    std::fs::rename(&beside, install.join(TASKS_OFF))
}

fn turn_on(names: &[String]) {
    let Some(folder) = task_folder() else { return };
    for name in names {
        if let Ok(task) = unsafe { folder.GetTask(&BSTR::from(name.as_str())) } {
            let _ = unsafe { task.SetEnabled(windows::Win32::Foundation::VARIANT_TRUE) };
        }
    }
}

/// Turns on again the tasks an install of this copy turned off and, cut short, left off. Needs elevation.
fn tasks_left_off() {
    let Some(install) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else { return };
    let off = left_off(&install);
    if !off.is_empty() {
        turn_on(&off);
        crate::journal::note("sensors: the tasks an install cut short had left off are on again");
    }
    if install.join(TASKS_OFF).exists() {
        let _ = remove(&install.join(TASKS_OFF));
    }
}

/// Copies everything in `from` into `to`, folders and all, replacing files
/// of the same name.
fn copy_into(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if !target.is_dir() {
                std::fs::create_dir(&target)?;
            }
            own_as_administrators(&target)?;
            copy_into(&entry.path(), &target)?;
        } else {
            replace(&entry.path(), &target)?;
            own_as_administrators(&target)?;
        }
    }
    Ok(())
}

/// `path`, put in place by the installer, owned by the Administrators group, whatever account it runs as: an
/// administrator's session that did not come through the elevation prompt (a remote shell's, a deployment tool's)
/// makes what it creates its user's, who could then re-permission it, and Glance would rightly no longer start
/// from there unasked (nor be installed over it again).
fn own_as_administrators(path: &Path) -> std::io::Result<()> {
    use windows::Win32::Security::Authorization::{ConvertStringSidToSidW, SetNamedSecurityInfoW};
    let mut administrators = PSID::default();
    unsafe { ConvertStringSidToSidW(w!("S-1-5-32-544"), &mut administrators) }.map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
    let set = unsafe { SetNamedSecurityInfoW(&HSTRING::from(path.as_os_str()), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, Some(administrators), None, None, None) };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(administrators.0)));
    }
    if set.is_err() {
        return Err(std::io::Error::from_raw_os_error(set.0 as i32));
    }
    Ok(())
}

/// How long a file still held by a program that was just ended is waited
/// for, and how often it is tried meanwhile.
const LET_GO_WITHIN: Duration = Duration::from_secs(15);
const TRIED_EVERY: Duration = Duration::from_millis(100);

/// Copies `from` over `to` (see `patiently`).
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    patiently(|| std::fs::copy(from, to).map(drop))
}

/// Does `act` to a file, waited for while it is held. A program ended a moment ago (the Glance being replaced, its
/// phone remote) holds its own file till Windows has done with it, which may be a while after it was ended: such a
/// file is waited for, that long.
fn patiently(mut act: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let began = Instant::now();
    loop {
        match act() {
            Ok(()) => return Ok(()),
            // (In use by another, mapped as a running program, or refused as a running program's is.)
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 1224)) && began.elapsed() < LET_GO_WITHIN => std::thread::sleep(TRIED_EVERY),
            Err(error) => return Err(error),
        }
    }
}

/// Takes Glance's own files out of `folder`, for the uninstaller (see `take_out`), and what it kept beside them.
pub fn uninstall_from(folder: &Path) -> Result<(), InstallFolder> {
    take_out(folder, true)
}

/// Takes out the Glance installed in `earlier` once it has been installed in `now`, elsewhere: the user chose another
/// folder for it (an earlier Glance in a folder not protected can be installed again only elsewhere: see
/// `install_into`). Its programs running from there are ended, and its files go, as uninstalling takes them (see
/// `take_out`); what Glance keeps beside its folder stays, for the one installed now. Nothing to do where `earlier` is
/// `now`, or holds no Glance any more.
pub fn moved_from(earlier: &Path, now: &Path) -> Result<(), InstallFolder> {
    let (Ok(earlier_found), Ok(now_found)) = (std::fs::canonicalize(earlier), std::fs::canonicalize(now)) else {
        return if earlier.exists() { Err(InstallFolder::Failed) } else { Ok(()) };
    };
    if same_path(&earlier_found, &now_found) || !earlier.join(APP).exists() {
        return Ok(());
    }
    take_out(earlier, false)
}

/// Takes Glance's own files out of `folder`: held, and only if only administrators can change it (then no one else
/// can have put anything in its place or make it a link); its programs running from there ended first; only what
/// Glance puts there, then the folder itself, by its handle, if nothing else is left. With what Glance kept beside
/// it (`forgetting`: the lights' program fetched), for an uninstall.
fn take_out(folder: &Path, forgetting: bool) -> Result<(), InstallFolder> {
    let folder = direct(folder).ok_or(InstallFolder::Indirect)?;
    let held = hold(&folder).ok_or(InstallFolder::Indirect)?;
    if direct(&folder).is_none() || !held.place().is_some_and(|place| same_path(&place, &folder)) {
        return Err(InstallFolder::Indirect);
    }
    if !only_trusted_can(&folder, FOLDER_CHANGES) {
        return Err(InstallFolder::Occupied);
    }
    // (Its tasks are gone by now: see `remove_all_tasks`.)
    if !end_programs_in(&folder) {
        return Err(InstallFolder::Failed);
    }
    if forgetting {
        crate::openrgb::forget();
    }
    // The uninstaller last, and only once all the rest is gone: left, it is there to try again with (as Glance is
    // left in the installed programs: see the installer).
    let mut removed = true;
    for name in INSTALLED.iter().filter(|name| **name != "uninstall.exe") {
        removed &= remove(&folder.join(name)).is_ok();
    }
    if removed {
        removed = remove(&folder.join("uninstall.exe")).is_ok();
    }
    // Gone, if nothing else is in it: by a handle able to delete it, if it is still the one held (which is let go
    // first: it shares no deletion).
    drop(held);
    if let Some(deletable) = pin(&folder).filter(|again| again.place().is_some_and(|place| same_path(&place, &folder))) {
        deletable.delete();
    }
    removed.then_some(()).ok_or(InstallFolder::Failed)
}

/// `folder` held, if it is really where its path says: Windows, asked by
/// handle, puts it there (no link at it or anywhere above it).
fn held_where_it_says(folder: &Path) -> Option<Held> {
    // Held without the right to delete it: a file renamed into it has the
    // folder opened to add the name, and on Windows 10 that open, sharing
    // no deletion, is refused by a holder that may delete
    // (ERROR_SHARING_VIOLATION). Holding it still keeps it, and every
    // folder above it, from being moved or deleted by anyone else.
    let held = Held::open_to_hold(folder).filter(Held::is_folder)?;
    let place = held.place()?;
    same_path(&place, &std::path::absolute(folder).ok()?).then_some(held)
}

/// Reads `name` in `folder`, both held and checked by handle (the folder
/// where its path says, the file in it and not a link), so that Glance,
/// running elevated, reads nothing a link in a user's folder leads to; and
/// nothing larger than `most` bytes (a file anyone could have filled).
pub fn read_in_place(folder: &Path, name: &str, most: u64) -> Option<String> {
    String::from_utf8(read_bytes_in_place(folder, name, most)?).ok()
}

/// Reads `name` in `folder` as `read_in_place` does, as it is (not text).
pub fn read_bytes_in_place(folder: &Path, name: &str, most: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let held = held_where_it_says(folder)?;
    let mut file = Held::open(&folder.join(name))?;
    if file.is_folder() || is_link(&folder.join(name)) || !file.lies_in(&held) {
        return None;
    }
    // Larger than `most`: not one Glance wrote, and not read whole.
    let mut bytes = Vec::new();
    file.0.by_ref().take(most + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= most).then_some(bytes)
}

/// Renames `name` in `folder` as `aside` (in place of one there), for Glance running elevated: the folder held and
/// checked to be where its path says (see `read_in_place`); a link there renamed itself, never followed.
pub fn put_aside_in_place(folder: &Path, name: &str, aside: &str) -> std::io::Result<()> {
    let _held = held_where_it_says(folder).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not where its path says"))?;
    std::fs::rename(folder.join(name), folder.join(aside))
}

/// Removes `name` in `folder`, for Glance running elevated, as `remove_kept` removes a file: the folder held and
/// checked by handle to be where its path says, the file opened as itself (a link as the link) and checked by handle
/// to lie in it, and removed by its handle. Whether it was.
pub fn remove_in_place(folder: &Path, name: &str) -> bool {
    let Some(held) = held_where_it_says(folder) else { return false };
    Held::open(&folder.join(name)).is_some_and(|file| file.lies_in(&held) && file.delete())
}

/// Writes `contents` as `name` in `folder`, for Glance running elevated: the
/// folder held and checked by handle to be where its path says, the bytes
/// written to a new file there (a new file follows no link) checked by
/// handle to lie in it, which then takes the name's place (a link there is
/// replaced, never written through).
pub fn write_in_place(folder: &Path, name: &str, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    write_in_place_by(folder, name, |file| file.write_all(contents))
}

/// Writes `name` in `folder` as `write_in_place` does, what it holds written by `write` into the new file.
pub fn write_in_place_by(folder: &Path, name: &str, write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let refused = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not where its path says");
    let held = held_where_it_says(folder).ok_or_else(refused)?;
    // A name of its own for each attempt.
    static ATTEMPT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let attempt = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = folder.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode((FILE_GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES).0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&temporary)?;
    let mut file = Held(file);
    if !file.lies_in(&held) {
        file.delete();
        return Err(refused());
    }
    // On the disk before it takes the name's place: a power cut or a stop of the system just after would otherwise
    // leave the name on an empty file (the settings then read as none).
    if let Err(error) = write(&mut file.0).and_then(|()| file.0.sync_all()) {
        file.delete();
        return Err(error);
    }
    // Renamed by its own handle while still open (not opened again by its
    // path), so that if the name's place cannot be taken, what was written
    // goes again by its handle.
    match file.rename_to(&folder.join(name)) {
        Ok(()) => Ok(()),
        Err(error) => {
            file.delete();
            Err(error)
        }
    }
}

/// Makes sure `folder` exists, creating it, if need be, only in a parent
/// that is where its path says, checked by handle to lie in it.
pub fn ensure_folder(folder: &Path) -> std::io::Result<()> {
    if folder.is_dir() {
        return Ok(());
    }
    let refused = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not where its path says");
    let parent = folder.parent().and_then(held_where_it_says).ok_or_else(refused)?;
    create_held(folder, &parent, None).map(drop).map_err(|_| refused())
}

/// Removes this user's settings for the uninstaller: the one file Glance
/// keeps, then its folder if empty. The folder is held and checked by
/// handle to be where its path says, the file opened as itself (a link as
/// the link) and checked by handle to lie in it, and each removed by its
/// handle: nothing elsewhere can be reached through a link.
pub fn remove_settings() -> bool {
    // Glance's own, and the phone remote's (the phones paired and how the lights are lit, as kept and as it was being kept anew; and its log).
    let remote = std::env::var_os("APPDATA").map(|roaming| where_it_is(Path::new(&roaming)).join("dev.weiyi.glance.remote"));
    // (The reports sent, and the pictures kept of them, first: their folder is in its own.)
    let reports = remove_all_kept(&crate::settings::config_dir().join(crate::reports::PICTURES));
    // (What Glance's threads stood at as one hung, see `hang`: in its own folder too.)
    let hangs = remove_all_kept(&crate::settings::config_dir().join(crate::hang::FOLDER));
    let own = remove_kept(&crate::settings::config_dir(), &["settings.json", crate::settings::UNREAD, crate::journal::FILE, crate::reports::FILE]);
    let remotes = remote.is_none_or(|folder| remove_kept(&folder, &["config.json", "config.json.new", "remote.log"]));
    // And what it fetched and kept (the lyrics found), its index of the music on this PC, the station (what was copied
    // and put on it, and what it keeps of that) and what it found of files lately, then its local folder. (The notes
    // are the user's own files, in a folder of theirs: they stay.)
    let lyrics = crate::lyrics::kept::names();
    let local = crate::settings::local_dir();
    let station = remove_all_kept(&local.join(crate::station::folder::OWN));
    let fetched = remove_kept(&crate::lyrics::kept::folder(), &lyrics.iter().map(String::as_str).collect::<Vec<_>>())
        && remove_kept(&local, &[crate::library::INDEX_FILE, crate::serif::FILE, crate::station::recent::FILE]);
    reports && hangs && own && remotes && station && fetched
}

/// Takes all that is in `folder` out of it, the folders in it each so too, then `folder`: as `remove_kept` takes each
/// (a link taken as the link, nothing it leads to reached).
fn remove_all_kept(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else { return !folder.exists() };
    let (mut files, mut inner) = (Vec::new(), true);
    for entry in entries.flatten() {
        // (A folder as itself: a link to one, as a link, is a file here.)
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => inner &= remove_all_kept(&entry.path()),
            _ => files.push(entry.file_name().to_string_lossy().into_owned()),
        }
    }
    remove_kept(folder, &files.iter().map(String::as_str).collect::<Vec<_>>()) && inner
}

/// Takes the files named out of `folder`, and the folder if nothing else
/// is left in it: as `remove_settings` says.
fn remove_kept(folder: &Path, names: &[&str]) -> bool {
    if !folder.exists() {
        return true;
    }
    let Some(held) = held_where_it_says(folder) else { return false };
    // Each file by its handle.
    let gone: Vec<bool> = names
        .iter()
        .map(|name| match Held::open(&folder.join(name)) {
            Some(file) if file.lies_in(&held) => file.delete(),
            Some(_) => false,
            // Not opened: gone already, or held by another program.
            None => folder.join(name).symlink_metadata().is_err(),
        })
        .collect();
    // Then the folder, by a handle able to delete it, if it is still the
    // one held (which is let go first: it shares no deletion).
    drop(held);
    if let Some(folder_held) = pin(folder).filter(|held| held.place().is_some_and(|place| std::path::absolute(folder).is_ok_and(|path| same_path(&place, &path)))) {
        folder_held.delete();
    }
    gone.iter().all(|gone| *gone)
}

/// `file` where it really is, if Glance may run it elevated without asking
/// (see `checked`), as a plain path.
pub fn trusted(file: &Path) -> Option<PathBuf> {
    checked(file).map(|real| plain(&real))
}

fn protected(file: &Path) -> bool {
    checked(file).is_some()
}

/// Glance's program `name` (`APP` or `SENSORS`), beside this one: as given, unresolved.
pub fn beside(name: &str) -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.with_file_name(name))
}

/// Glance's program `name` beside this one as Task Scheduler should run it:
/// where it really is, without the `\\?\` prefix resolution adds.
fn program(name: &str) -> Option<PathBuf> {
    Some(plain(&std::fs::canonicalize(beside(name)?).ok()?))
}

/// A resolved path without the `\\?\` prefix (kept on a network path, which
/// needs it).
fn plain(real: &Path) -> PathBuf {
    let text = real.to_string_lossy();
    text.strip_prefix(r"\\?\").filter(|rest| !rest.starts_with("UNC\\")).map_or(real.to_path_buf(), PathBuf::from)
}

/// Whether Glance's programs here may be started without asking, the sensors elevated: see the module's notes. Each
/// is checked as itself (a protected folder may still hold a file someone else may change).
pub fn may_start_unasked() -> bool {
    [APP, SENSORS].iter().all(|name| beside(name).is_some_and(|program| protected(&program)))
}

/// The current user's security identifier, which names their tasks: each
/// account that runs Glance has tasks of its own.
pub fn user_sid() -> Option<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut buffer = vec![0u64; 64];
    let mut size = 0u32;
    let read = unsafe { GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr().cast()), (buffer.len() * 8) as u32, &mut size) };
    let _ = unsafe { CloseHandle(token) };
    read.ok()?;
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    sid_string(user.User.Sid)
}

fn launch_task() -> String {
    format!("{LAUNCH_TASK} {}", user_sid().unwrap_or_default())
}

fn logon_task() -> String {
    format!("{LOGON_TASK} {}", user_sid().unwrap_or_default())
}

// ---- Tasks ----

fn task_folder() -> Option<ITaskFolder> {
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).ok()?;
        let none = VARIANT::default();
        service.Connect(&none, &none, &none, &none).ok()?;
        service.GetFolder(&BSTR::from("\\")).ok()
    }
}

/// Whether a task is one of Glance's: under the names it gives tasks now
/// (with an account's identifier) or gave them before.
fn is_glance_task(name: &str) -> bool {
    [LOGON_TASK, LAUNCH_TASK, REMOTE_TASK].iter().any(|base| {
        name.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.strip_prefix(" S-1-").is_some_and(|sid| !sid.is_empty() && sid.chars().all(|c| c.is_ascii_digit() || c == '-')))
    })
}

/// Removes every account's Glance tasks, then ends every program of Glance's running from `folder`: what the
/// uninstaller asks of its own copy of Glance, run elevated, before Glance's settings and files are taken away.
pub fn remove_all_tasks(folder: &Path) -> bool {
    for name in glance_tasks() {
        // (What a task has running is stopped with it: the phone remote runs for as long as its task does.)
        if let Some(task) = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(name.as_str())) }.ok()) {
            let _ = unsafe { task.Stop(0) };
        }
        delete_task(&name);
    }
    end_programs_in(folder)
}

/// Runs this account's task for the phone remote, if it has one for the
/// remote at `helper`: Glance starting finds the remote running, or
/// starts it (an update stops it to replace it).
pub fn run_remote(helper: &Path) {
    if remote_enabled(helper) {
        if let Some(task) = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(remote_task())) }.ok()) {
            let _ = unsafe { task.Run(&VARIANT::default()) };
        }
    }
}

/// Every account's Glance tasks, by name.
fn glance_tasks() -> Vec<String> {
    let Some(folder) = task_folder() else { return Vec::new() };
    unsafe {
        let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) else { return Vec::new() };
        let count = tasks.Count().unwrap_or(0);
        (1..=count)
            .filter_map(|i| tasks.get_Item(&VARIANT::from(i)).ok()?.Name().ok().map(|n| n.to_string()))
            .filter(|name| is_glance_task(name))
            .collect()
    }
}

/// The program a task runs, if the task exists.
fn task_program(name: &str) -> Option<String> {
    unsafe {
        let task = task_folder()?.GetTask(&BSTR::from(name)).ok()?;
        let action: IExecAction = task.Definition().ok()?.Actions().ok()?.get_Item(1).ok()?.cast().ok()?;
        let mut path = BSTR::new();
        action.Path(&mut path).ok()?;
        Some(path.to_string())
    }
}

/// Whether the task `name` runs as the current account: its principal is
/// this account, named as the task XML names it or by its identifier.
fn task_is_mine(name: &str) -> bool {
    let principal = unsafe {
        (|| {
            let task = task_folder()?.GetTask(&BSTR::from(name)).ok()?;
            let mut user = BSTR::new();
            task.Definition().ok()?.Principal().ok()?.UserId(&mut user).ok()?;
            Some(user.to_string())
        })()
    };
    let Some(principal) = principal else { return false };
    let account = std::env::var("USERDOMAIN").map(|domain| format!("{domain}\\")).unwrap_or_default()
        + &std::env::var("USERNAME").unwrap_or_default();
    principal.eq_ignore_ascii_case(&account) || user_sid().is_some_and(|sid| principal.eq_ignore_ascii_case(&sid))
}

/// Starts Glance's sensors, elevated: through the launch task, if it starts this copy's sensors and this copy may
/// start unasked; else by asking the user (once: the sensors then make the task). Returns whether they were started
/// (or asked for).
pub fn start_sensors() -> bool {
    if start_sensors_unasked() {
        return true;
    }
    let Some(sensors) = beside(SENSORS) else { return false };
    crate::journal::note("sensors: the user asked to let them run as administrator");
    let started = unsafe { ShellExecuteW(None, w!("runas"), &HSTRING::from(sensors.as_os_str()), PCWSTR::null(), PCWSTR::null(), windows::Win32::UI::WindowsAndMessaging::SW_HIDE) };
    // ShellExecute reports success with a value above 32.
    started.0 as usize > 32
}

/// Starts Glance's sensors through the launch task, if it starts this copy's sensors and this copy may start
/// unasked (the user is never asked); whether they were started.
pub fn start_sensors_unasked() -> bool {
    // (An earlier version's launch task starts `glance.exe`: elevated, with Glance running, it starts the sensors'
    // own program, which makes the task anew.)
    let launch = launch_task();
    if !((runs(&launch, SENSORS) || runs(&launch, APP)) && may_start_unasked()) {
        return false;
    }
    let started = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(launch)).ok()?.Run(&VARIANT::default()).ok() });
    if started.is_some() {
        crate::journal::note("sensors: started through the launch task");
    }
    started.is_some()
}

/// Starts the sensors' own program beside this one, elevated as this process is: for `glance.exe` started elevated
/// by an earlier version's task, or by hand. Only a program as protected as this one is started: from a protected
/// install, the sensors' file must be too; one run elevated from where anyone may change it was the user's choice,
/// and its sensors beside it are no less so. Whether they were started.
pub fn start_sensors_from_here() -> bool {
    let Some(sensors) = beside(SENSORS) else { return false };
    let here_protected = std::env::current_exe().is_ok_and(|exe| protected(&exe));
    let sensors = if here_protected {
        match trusted(&sensors) {
            Some(sensors) => sensors,
            None => return false,
        }
    } else {
        sensors
    };
    Command::new(sensors).creation_flags(NO_WINDOW).spawn().is_ok()
}

/// Whether the task `task` runs Glance's program `name` here (`APP` or `SENSORS`).
fn runs(task: &str, name: &str) -> bool {
    let (Some(program), Some(ours)) = (task_program(task), program(name)) else { return false };
    std::fs::canonicalize(&program).is_ok_and(|program| std::fs::canonicalize(&ours).is_ok_and(|ours| program == ours))
}

/// The priority Glance's tasks run at: an ordinary program's (Task Scheduler's own default, 7, is below it, for the CPU,
/// the disk and memory alike, which suits work in the background: these hear the mouse, draw, and answer the phone).
const TASK_PRIORITY: u32 = 5;

/// A task for the current user: the launch task, which starts the sensors' program `exe` with highest privileges on
/// demand; or, `at_sign_in`, the one that starts Glance (`exe`) as the user when the user signs in.
fn task_xml(exe: &Path, at_sign_in: bool, enabled: bool) -> String {
    let user = std::env::var("USERDOMAIN").map(|domain| format!("{domain}\\")).unwrap_or_default()
        + &std::env::var("USERNAME").unwrap_or_default();
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let trigger = if at_sign_in {
        format!("<Triggers><LogonTrigger><UserId>{}</UserId></LogonTrigger></Triggers>", escape(&user))
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  {trigger}
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>{level}</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>{TASK_PRIORITY}</Priority>
    <Enabled>{enabled}</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command></Exec>
  </Actions>
</Task>"#,
        user = escape(&user),
        exe = escape(&exe.display().to_string()),
        level = if at_sign_in { "LeastPrivilege" } else { "HighestAvailable" },
    )
}

/// Whether the task `name` is on: none there is (one made is, unless made off).
fn task_enabled(name: &str) -> bool {
    let task = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(name)).ok() });
    task.is_none_or(|task| unsafe { task.Enabled() }.is_ok_and(|on| on.as_bool()))
}

/// Registers a task for this copy's programs (Glance at sign-in, else the sensors), on or off, handed to Task
/// Scheduler directly (no file in between). Needs elevation.
fn register_task(name: &str, at_sign_in: bool, enabled: bool) -> bool {
    let Some(exe) = program(if at_sign_in { APP } else { SENSORS }) else { return false };
    let Some(folder) = task_folder() else { return false };
    let none = VARIANT::default();
    unsafe {
        folder
            .RegisterTask(
                &BSTR::from(name),
                &BSTR::from(task_xml(&exe, at_sign_in, enabled)),
                TASK_CREATE_OR_UPDATE.0,
                &none,
                &none,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &none,
            )
            .is_ok()
    }
}

fn delete_task(name: &str) {
    if let Some(folder) = task_folder() {
        let _ = unsafe { folder.DeleteTask(&BSTR::from(name), 0) };
    }
}

/// Keeps the launch task pointing at this copy's sensors, if they may start
/// unasked; else makes sure no task starts this copy. Needs elevation.
pub fn register_launch_task() {
    tasks_left_off();
    // Each task made anew as the user left it, on or off (made on, one the user had turned off in Task Scheduler
    // started Glance at every sign-in again).
    if may_start_unasked() {
        // (An earlier version's, which ran `glance.exe` itself, is made anew.)
        let launches = register_task(&launch_task(), false, task_enabled(&launch_task()));
        // Earlier versions' tasks, shared by every account under one name,
        // move to this account's own if they were this account's: starting
        // at sign-in carries over, and each old task goes only once its
        // replacement is registered.
        let mine = |name: &str| runs(name, APP) && task_is_mine(name);
        if launches && mine(LAUNCH_TASK) {
            delete_task(LAUNCH_TASK);
        }
        if mine(LOGON_TASK) && register_task(&logon_task(), true, task_enabled(LOGON_TASK)) {
            delete_task(LOGON_TASK);
        }
        // This account's sign-in task, made by an earlier version to start Glance whole and elevated, starts it as
        // the user now; and one left starting a Glance taken away (installed in another folder since: see
        // `moved_from`) starts this one, on or off as it was.
        let gone = task_program(&logon_task()).map(PathBuf::from).is_some_and(|program| program.file_name().is_some_and(|name| name.eq_ignore_ascii_case(APP)) && !program.exists());
        if runs(&logon_task(), APP) || gone {
            register_task(&logon_task(), true, task_enabled(&logon_task()));
        }
    } else {
        // A copy that may not start unasked keeps no task that starts it,
        // whose account it is.
        for name in glance_tasks() {
            if runs(&name, APP) || runs(&name, SENSORS) {
                delete_task(&name);
            }
        }
    }
}

/// Whether this account starts this copy of Glance at sign-in (its task there and on).
pub fn autostart_enabled() -> bool {
    runs(&logon_task(), APP) && task_enabled(&logon_task())
}

/// Turns starting at sign-in on or off; returns whether it is now on. Only
/// an executable that may start unasked can start at sign-in.
pub fn set_autostart(enabled: bool) -> bool {
    if enabled && may_start_unasked() {
        register_task(&logon_task(), true, true);
    } else if !enabled {
        delete_task(&logon_task());
    }
    autostart_enabled()
}

// ---- The phone remote ----

fn remote_task() -> String {
    format!("{REMOTE_TASK} {}", user_sid().unwrap_or_default())
}

/// A task that runs `helper` as the user, with no more than a user's
/// rights, at sign-in and for as long as it runs.
fn remote_task_xml(helper: &Path) -> String {
    // The account by its identifier, which Windows itself gives (its name
    // as the environment has it may be missing or another's).
    let user = user_sid().unwrap_or_default();
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>{TASK_PRIORITY}</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{helper}</Command></Exec>
  </Actions>
</Task>"#,
        user = escape(&user),
        helper = escape(&helper.display().to_string()),
    )
}

/// Whether this account's task runs the remote at `helper`.
pub fn remote_enabled(helper: &Path) -> bool {
    task_program(&remote_task()).is_some_and(|program| same_path(Path::new(&program), helper))
}

/// Turns the helper of the phone remote and nearby transfer on (its task registered, and run anew now: one running is
/// stopped first, and takes what it was told as it starts again) or off (stopped, its task gone); returns whether it
/// is on then.
pub fn set_remote(helper: &Path, on: bool) -> bool {
    let name = remote_task();
    let Some(folder) = task_folder() else { return false };
    let none = VARIANT::default();
    unsafe {
        if let Ok(task) = folder.GetTask(&BSTR::from(name.as_str())) {
            stopped(&task, helper);
        }
        if on {
            let registered = folder.RegisterTask(&BSTR::from(name.as_str()), &BSTR::from(remote_task_xml(helper)), TASK_CREATE_OR_UPDATE.0, &none, &none, TASK_LOGON_INTERACTIVE_TOKEN, &none);
            if let Ok(task) = registered {
                let _ = task.Run(&none);
            }
        } else {
            delete_task(&name);
        }
    }
    remote_enabled(helper)
}

/// How long a task stopped is waited for to be over (its program ended, the ports it held let go).
const STOPPED_WITHIN_MS: u32 = 5_000;

/// `task`, which runs `program`, stopped, and waited for till that program has ended (a while at most: see
/// `STOPPED_WITHIN_MS`). The task's state is not what is waited for: it leaves "running" as the Task Scheduler asks the
/// program to end, and a program started again at once found the old one still holding its port (2026-11-12).
fn stopped(task: &IRegisteredTask, program: &Path) {
    use windows::Win32::System::Threading::WaitForSingleObject;
    // (Opened before it is stopped: what is waited for is that very process, whatever starts after.)
    let running = running(|file| same_path(file, program));
    unsafe {
        let _ = task.Stop(0);
        for process in running {
            WaitForSingleObject(process, STOPPED_WITHIN_MS);
            let _ = CloseHandle(process);
        }
    }
}

// ---- The driver ----

/// The PawnIO driver's version, if it is installed.
pub fn pawnio_version() -> Option<String> {
    let version = crate::metrics::reg_string(PAWNIO_UNINSTALL_KEY, w!("DisplayVersion"));
    (!version.is_empty()).then_some(version)
}

fn pawnio_installed() -> bool {
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PAWNIO_UNINSTALL_KEY, w!("DisplayVersion"), RRF_RT_REG_SZ, None, None, Some(&mut size)) }
        .is_ok()
}

/// Installs the PawnIO driver from the setup shipped with Glance, if no
/// program has installed it yet, and notes that Glance did. Needs elevation;
/// the setup runs only from where the user cannot have changed it.
pub fn ensure_pawnio(setup: &Path) {
    if pawnio_installed() {
        return;
    }
    // Run from exactly the place that was checked.
    let Some(setup) = checked(setup) else { return };
    let child = match Command::new(setup).args(["-install", "-silent"]).creation_flags(NO_WINDOW).spawn() {
        Ok(child) => child,
        Err(_) => {
            crate::journal::note("the PawnIO driver's installer could not be started; temperatures, power and fans will be missing");
            return;
        }
    };
    // Its end waited for on a thread of its own, which marks the driver as Glance's once it is in, and tells what
    // reads through it to open again (whenever that is: an installer is not ended part way, which could leave the
    // driver half in). Nothing else waits for it: the readings start without it (waited for, the sensors greeted the
    // rest of Glance only once it was in, and a slow install had Glance give up on them and read without rights).
    std::thread::spawn(move || {
        let mut child = child;
        let installed = child.wait().is_ok_and(|status| status.success()) && pawnio_installed();
        if installed {
            mark_pawnio_ours();
            crate::journal::note("the PawnIO driver is installed; the readings that need it are opened again");
            crate::pawnio::installed();
        } else {
            crate::journal::note("the PawnIO driver could not be installed; temperatures, power and fans will be missing");
        }
    });
}

fn mark_pawnio_ours() {
    let mut key = HKEY::default();
    let created = unsafe {
        RegCreateKeyExW(HKEY_LOCAL_MACHINE, GLANCE_KEY, None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    };
    if created.is_err() {
        return;
    }
    let one = 1u32.to_le_bytes();
    unsafe {
        let _ = RegSetValueExW(key, INSTALLED_PAWNIO, None, REG_DWORD, Some(&one));
        let _ = RegCloseKey(key);
    }
}

#[cfg(test)]
mod tests {
    /// Glance's tasks run as an ordinary program does: Task Scheduler's own default is below that for the CPU, the disk
    /// and memory alike, and what hears the mouse and draws should not wait behind all else on a busy machine.
    #[test]
    fn its_tasks_run_as_ordinary_programs() {
        let exe = std::path::Path::new(r"C:\Program Files\Glance\glance-sensors.exe");
        for xml in [super::task_xml(exe, false, true), super::task_xml(exe, true, true), super::remote_task_xml(exe)] {
            assert!(xml.contains("<Priority>5</Priority>"), "{xml}");
        }
    }

    /// Settings kept under application data reached through a junction (a profile moved to another drive by one) are
    /// kept and read again from where the folder really is, while a link in the folder Glance keeps them in is still
    /// never followed.
    #[test]
    fn keeps_what_is_under_a_folder_reached_through_a_junction() {
        let root = crate::settings::local_dir().join("moved-profile");
        let _ = std::fs::remove_dir_all(&root);
        let (moved, profile) = (root.join("moved"), root.join("profile"));
        std::fs::create_dir_all(&moved).unwrap();
        let junction = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&profile).arg(&moved).output().unwrap();
        assert!(junction.status.success(), "{junction:?}");
        let own = super::where_it_is(&profile).join("dev.weiyi.glance");
        let kept = super::ensure_folder(&own).and_then(|()| super::write_in_place(&own, "settings.json", b"{}"));
        let read = super::read_in_place(&own, "settings.json", 16);
        // A link in Glance's own folder: refused, as before.
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let inner = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(own.join("linked")).arg(&elsewhere).output().unwrap();
        let through = super::write_in_place(&own.join("linked"), "settings.json", b"{}");
        let landed = elsewhere.join("settings.json").exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(kept.is_ok(), "{kept:?}");
        assert_eq!(read.as_deref(), Some("{}"));
        assert!(inner.status.success() && through.is_err() && !landed, "written through a link in its own folder");
    }

    /// An install folder holding an earlier Glance and the OpenRGB put there with it is that Glance's, to install over
    /// and to take away: anything else in it, and it is not.
    #[test]
    fn an_earlier_glance_with_its_openrgb_is_its_own() {
        let folder = crate::settings::local_dir().join("earlier-glance");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join(super::OPENRGB)).unwrap();
        for name in [super::APP, "uninstall.exe"] {
            std::fs::write(folder.join(name), "x").unwrap();
        }
        std::fs::write(folder.join(super::OPENRGB).join("OpenRGB.exe"), "x").unwrap();
        let with_openrgb = super::claimable(&folder);
        std::fs::write(folder.join("theirs.txt"), "x").unwrap();
        let with_another = super::claimable(&folder);
        let _ = std::fs::remove_dir_all(&folder);
        assert!(with_openrgb && !with_another, "{with_openrgb} {with_another}");
    }

    /// A folder the user can change is vouched for by no one: what is in it is not run with an administrator's rights.
    #[test]
    fn vouches_for_no_folder_the_user_can_change() {
        let folder = crate::settings::local_dir().join("not-vouched");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("OpenRGB.exe"), "x").unwrap();
        let vouched = super::vouched_folder(&folder);
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(vouched, None);
    }

    /// A folder taken out whole as Glance is uninstalled (the station's): its files, the folders in it and theirs, and
    /// a link in it to a folder elsewhere taken as the link, what it leads to left as it is.
    #[test]
    fn takes_a_folder_out_whole_but_not_what_a_link_in_it_leads_to() {
        let root = crate::settings::local_dir().join("remove-all-kept");
        let _ = std::fs::remove_dir_all(&root);
        let (inside, outside) = (root.join("Station"), root.join("elsewhere"));
        std::fs::create_dir_all(inside.join("named").join("a1")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(inside.join("station.json"), "{}").unwrap();
        std::fs::write(inside.join("k1"), "kept").unwrap();
        std::fs::write(inside.join("named").join("a1").join("报告 1.pdf"), "named").unwrap();
        std::fs::write(outside.join("theirs.txt"), "theirs").unwrap();
        let junction = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(inside.join("link")).arg(&outside).output().unwrap();
        assert!(junction.status.success(), "{junction:?}");
        let removed = super::remove_all_kept(&inside);
        let (left, theirs) = (inside.exists(), std::fs::read_to_string(outside.join("theirs.txt")));
        let _ = std::fs::remove_dir_all(&root);
        assert!(removed && !left, "the station's folder left");
        assert_eq!(theirs.ok().as_deref(), Some("theirs"));
    }

    #[test]
    #[ignore = "registers this account's task for the remote on this machine, and runs it"]
    fn turns_the_remote_on() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let helper = std::path::PathBuf::from(std::env::var("GLANCE_REMOTE_HELPER").expect("the helper's path"));
        assert!(super::set_remote(&helper, true));
        assert!(super::remote_enabled(&helper));
    }

    use super::*;

    /// A folder in the temp folder, of this test alone, holding `files` (name, what is in it; a name with a `/` in a
    /// folder of its own).
    fn folder_of(test: &str, files: &[(&str, &str)]) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("glance-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for (name, text) in files {
            let path = folder.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// The files in `folder`, by name (a `/` before each file in a folder), each with what is in it.
    fn files_in(folder: &Path) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.path().is_dir() {
                found.extend(files_in(&entry.path()).into_iter().map(|(inner, text)| (format!("{name}/{inner}"), text)));
            } else {
                found.push((name, std::fs::read_to_string(entry.path()).unwrap()));
            }
        }
        found.sort();
        found
    }

    fn named(files: &[(&str, &str)]) -> Vec<(String, String)> {
        files.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect()
    }

    /// One install at a time: held by one, another cannot have it; let go, it is gone.
    #[test]
    fn one_install_at_a_time() {
        let folder = folder_of("installing", &[]);
        let lock = folder.join(INSTALLING);
        let first = held_alone(&lock).unwrap();
        assert!(held_alone(&lock).is_err());
        drop(first);
        assert!(!lock.exists());
        assert!(held_alone(&lock).is_ok());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The new files go in place of the earlier ones, and those new here beside them; nothing is left aside.
    #[test]
    fn new_files_take_the_earlier_ones_place() {
        let folder = folder_of(
            "swap",
            &[("glance.exe", "old"), ("licenses/a.txt", "old"), ("kept.txt", "kept"), (".incoming/glance.exe", "new"), (".incoming/licenses/a.txt", "new"), (".incoming/new.exe", "new")],
        );
        std::fs::create_dir(folder.join(REPLACED)).unwrap();
        swap_in(&folder).unwrap();
        assert_eq!(files_in(&folder), named(&[("glance.exe", "new"), ("kept.txt", "kept"), ("licenses/a.txt", "new"), ("new.exe", "new")]));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// An install cut short as it put the new files in place: what it had moved aside goes back, in place of what
    /// took it, and the version it replaced is whole again.
    #[test]
    fn what_an_install_cut_short_moved_aside_goes_back() {
        let folder = folder_of(
            "put-back",
            &[("glance.exe", "new"), ("licenses/a.txt", "old"), (".replaced/glance.exe", "old"), (".replaced/glance-sensors.exe", "old"), (".incoming/glance-sensors.exe", "new")],
        );
        put_back(&folder).unwrap();
        assert_eq!(
            files_in(&folder),
            named(&[(".incoming/glance-sensors.exe", "new"), ("glance-sensors.exe", "old"), ("glance.exe", "old"), ("licenses/a.txt", "old")])
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    #[ignore = "writes into this user's folders"]
    fn writes_in_place_here() {
        let scratch = std::env::temp_dir().join("glance-write-test");
        let _ = std::fs::create_dir_all(&scratch);
        println!("temp: {:?}", write_in_place(&scratch, "settings.json", b"{}"));
        println!("temp holds: {:?}", std::fs::read_dir(&scratch).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>()));
        let config = crate::settings::config_dir();
        println!("config {config:?} ensure: {:?}", ensure_folder(&config));
        println!("config held: {}", held_where_it_says(&config).is_some());
        println!("config: {:?}", write_in_place(&config, "probe.json", b"{}"));
        println!("config holds: {:?}", std::fs::read_dir(&config).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>()));
        let _ = std::fs::remove_file(config.join("probe.json"));
    }

    #[test]
    fn knows_its_own_tasks() {
        assert!(is_glance_task("Glance"));
        assert!(is_glance_task("Glance at sign-in S-1-5-21-1-2-3-1001"));
        assert!(!is_glance_task("Glance Updater"));
        assert!(!is_glance_task("GlanceX"));
    }

    #[test]
    fn tells_protected_places_from_writable_ones() {
        // Windows' own programs cannot be changed by the user; this build can.
        assert!(protected(Path::new(r"C:\Windows\System32\notepad.exe")));
        assert!(!protected(&std::env::current_exe().unwrap()));
    }

    /// Needs administrator rights (a folder is made owned by them, and
    /// Glance's key written): a private folder is made only where no one
    /// else can move it aside (ProgramData, not the user's temporary
    /// folder), new, administrators' alone inside; the download noted in it
    /// is removed with its folder, and the note with them.
    #[test]
    #[ignore = "needs administrator rights; writes in ProgramData and the machine's registry"]
    fn makes_private_folders_elevated() {
        let name = format!("glance-private-test-{}", std::process::id());
        assert!(private_folder(&std::env::temp_dir(), &name).is_none());
        let program_data = PathBuf::from(std::env::var("ProgramData").unwrap());
        let folder = private_folder(&program_data, &name).expect("made");
        let file = folder.path().join("setup.exe");
        assert!(remember_download(&file));
        std::fs::write(&file, b"x").unwrap();
        assert!(only_trusted_inside(folder.path()));
        // Vouched for, where it really is.
        assert_eq!(vouched_folder(folder.path()).as_deref(), Some(folder.path()));
        // Not made over something already there.
        assert!(private_folder(&program_data, &name).is_none());
        // Held: it cannot be renamed, even by an administrator.
        assert!(std::fs::rename(folder.path(), program_data.join(format!("{name}-moved"))).is_err());
        let path = folder.path().to_path_buf();
        drop(folder);
        // A second download while the first is still in use (its installer
        // running): the first stays noted until it can go.
        let second = private_folder(&program_data, &format!("{name}-2")).expect("made");
        let second_file = second.path().join("setup.exe");
        // Held as a running installer holds itself: shared for reading only.
        use std::os::windows::fs::OpenOptionsExt;
        let in_use = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ.0).open(&file).unwrap();
        forget_download();
        assert_eq!(noted_downloads(), vec![file.clone()]);
        assert!(remember_download(&second_file));
        std::fs::write(&second_file, b"y").unwrap();
        drop(second);
        assert_eq!(noted_downloads(), vec![file.clone(), second_file.clone()]);
        drop(in_use);
        forget_download();
        assert!(!file.exists() && !path.exists() && !second_file.exists());
        assert_eq!(noted_downloads(), Vec::<PathBuf>::new());
    }

    #[test]
    fn tells_where_an_install_would_hold() {
        use InstallFolder::*;
        // A new folder in Program Files would; one beside this build would not.
        assert_eq!(install_folder(Path::new(r"C:\Program Files\Glance-not-there")), Holds);
        let beside = std::env::current_exe().unwrap().with_file_name("Glance-not-there");
        assert_eq!(install_folder(&beside), Open);
        // A drive's root is no folder to install into; one with other things in it is taken.
        assert_eq!(install_folder(Path::new(r"C:\")), Indirect);
        assert_eq!(install_folder(Path::new(r"C:\Windows")), Occupied);
        assert_eq!(install_folder(Path::new(r"C:\no-such-folder\Glance")), Open);
        println!(r"D:\Glance: {:?}", install_folder(Path::new(r"D:\Glance")));
    }

    #[test]
    fn pinning_holds_the_path() {
        // While a folder is pinned, nothing above it can be renamed or
        // removed, so no link can be put in its place.
        let base = std::env::temp_dir().join(format!("glance-pin-test-{}", std::process::id()));
        let above = base.join("above");
        let folder = above.join("Glance");
        std::fs::create_dir_all(&folder).unwrap();
        let pinned = pin(&folder).expect("a folder can be pinned");
        assert!(std::fs::rename(&above, base.join("moved")).is_err());
        assert!(std::fs::rename(&folder, above.join("moved")).is_err());
        assert!(std::fs::remove_dir(&folder).is_err());
        drop(pinned);
        // Held only to keep it (as the settings' folder is while they are
        // written), the same: and a file is renamed into it all the same.
        let held = held_where_it_says(&folder).expect("a folder can be held");
        assert!(std::fs::rename(&above, base.join("moved")).is_err());
        assert!(std::fs::rename(&folder, above.join("moved")).is_err());
        assert!(std::fs::remove_dir(&folder).is_err());
        write_in_place(&folder, "settings.json", b"{}").unwrap();
        assert_eq!(std::fs::read(folder.join("settings.json")).unwrap(), b"{}");
        drop(held);
        std::fs::remove_file(folder.join("settings.json")).unwrap();
        std::fs::rename(&above, base.join("moved")).unwrap();
        // A junction is not pinned.
        let link = base.join("link");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(base.join("moved")).output().unwrap();
        assert!(made.status.success());
        assert!(pin(&link).is_none());
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn compares_paths_as_windows_answers_them() {
        assert!(same_path(Path::new(r"\\?\C:\Users\A\AppData"), Path::new(r"c:\users\a\appdata\")));
        assert!(same_path(Path::new(r"\\?\UNC\server\share\Roaming"), Path::new(r"\\server\share\Roaming")));
        assert!(!same_path(Path::new(r"\\?\UNC\server\share\Roaming"), Path::new(r"C:\server\share\Roaming")));
    }

    #[test]
    fn acts_on_what_it_holds() {
        let base = std::env::temp_dir().join(format!("glance-held-test-{}", std::process::id()));
        let folder = base.join("folder");
        let target = base.join("target");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(folder.join("settings.json"), b"{}").unwrap();
        std::fs::write(target.join("settings.json"), b"{}").unwrap();
        let held = pin(&folder).unwrap();
        // A file in the held folder lies in it, and goes by its handle.
        let file = Held::open(&folder.join("settings.json")).unwrap();
        assert!(file.lies_in(&held));
        assert!(file.delete());
        assert!(!folder.join("settings.json").exists());
        // A junction is held as itself: it lies where it is, and deleting it
        // removes the junction, never what it leads to.
        let base_held = pin(&base).unwrap();
        let link = base.join("link");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap();
        assert!(made.status.success());
        assert!(pin(&link).is_none());
        let junction = Held::open(&link).unwrap();
        assert!(junction.lies_in(&base_held));
        assert!(!Held::open(&link.join("settings.json")).unwrap().lies_in(&base_held));
        assert!(junction.delete());
        assert!(!link.exists() && target.join("settings.json").exists());
        drop((held, base_held));
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// This account's settings folder is where its path says, so settings load.
    #[test]
    #[ignore = "reads this account's settings; run with --ignored"]
    fn finds_this_accounts_settings() {
        let folder = crate::settings::config_dir();
        println!("{} held where it says: {}", folder.display(), held_where_it_says(&folder).is_some());
        println!("settings: {:?}", read_in_place(&folder, "settings.json", 1 << 20).map(|text| text.len()));
    }

    #[test]
    fn reads_and_writes_only_where_the_path_says() {
        let base = std::env::temp_dir().join(format!("glance-place-test-{}", std::process::id()));
        let folder = base.join("settings");
        let target = base.join("target");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        // Where the path says: written, replaced, read back.
        write_in_place(&folder, "settings.json", b"one").unwrap();
        write_in_place(&folder, "settings.json", b"two").unwrap();
        assert_eq!(read_in_place(&folder, "settings.json", 1 << 20).as_deref(), Some("two"));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        // Through a junction, and through a junction above: refused, the
        // place it leads to untouched.
        std::fs::write(target.join("settings.json"), b"theirs").unwrap();
        let link = base.join("link");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap();
        assert!(made.status.success());
        assert!(write_in_place(&link, "settings.json", b"mine").is_err());
        assert!(read_in_place(&link, "settings.json", 1 << 20).is_none());
        std::fs::create_dir(target.join("inner")).unwrap();
        assert!(write_in_place(&link.join("inner"), "settings.json", b"mine").is_err());
        assert_eq!(std::fs::read(target.join("settings.json")).unwrap(), b"theirs");
        assert!(!target.join("inner").join("settings.json").exists());
        // Nor is a missing folder created through one.
        assert!(ensure_folder(&link.join("new")).is_err());
        assert!(!target.join("new").exists());
        ensure_folder(&base.join("made")).unwrap();
        assert!(base.join("made").is_dir());
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Installed in another folder, the earlier Glance is taken out of its own: not where it is the folder installed in
    /// now, holds no Glance or is gone, nor where others could have changed what is in it (then nothing in it is taken).
    #[test]
    fn takes_out_only_an_earlier_glance_elsewhere() {
        let base = std::env::temp_dir().join(format!("glance-moved-test-{}", std::process::id()));
        let (earlier, now) = (base.join("Earlier"), base.join("Now"));
        std::fs::create_dir_all(&earlier).unwrap();
        std::fs::create_dir_all(&now).unwrap();
        std::fs::write(now.join(APP), b"MZ").unwrap();
        // The same folder, written otherwise.
        assert_eq!(moved_from(&now, &base.join("now\\")), Ok(()));
        assert!(now.join(APP).exists());
        // No Glance in it, or none there at all.
        std::fs::write(earlier.join("notes.txt"), b"theirs").unwrap();
        assert_eq!(moved_from(&earlier, &now), Ok(()));
        assert_eq!(moved_from(&base.join("Gone"), &now), Ok(()));
        assert!(earlier.join("notes.txt").exists());
        // A Glance in a folder anyone may change: refused, all of it left.
        std::fs::write(earlier.join(APP), b"MZ").unwrap();
        assert_eq!(moved_from(&earlier, &now), Err(InstallFolder::Occupied));
        assert!(earlier.join(APP).exists() && earlier.join("notes.txt").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn sees_through_links() {
        // A junction to a folder: what was chosen is not where it leads.
        let base = std::env::temp_dir().join(format!("glance-link-test-{}", std::process::id()));
        let target = base.join("target");
        std::fs::create_dir_all(&target).unwrap();
        let link = base.join("link");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap();
        assert!(made.status.success());
        assert_eq!(install_folder(&link), InstallFolder::Indirect);
        assert_eq!(install_folder(&link.join("Glance")), InstallFolder::Indirect);
        // An earlier Glance's folder whose resources lead elsewhere.
        let earlier = base.join("Glance");
        std::fs::create_dir(&earlier).unwrap();
        let inner = earlier.join("resources");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&inner).arg(&target).output().unwrap();
        assert!(made.status.success());
        // Refused for what is in it, even when the user would install there
        // all the same; nothing is written through the junction.
        assert!(!only_trusted_inside(&earlier));
        assert_eq!(install_folder(&earlier), InstallFolder::Occupied);
        let payload = base.join("payload");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("glance.exe"), b"MZ").unwrap();
        assert_eq!(install_into(&earlier, &payload), Err(InstallFolder::Occupied));
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        std::fs::remove_dir(&inner).unwrap();
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }
}
