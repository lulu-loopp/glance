//! Running as administrator without asking every time, starting with
//! Windows, and the PawnIO driver the sensors need.
//!
//! The driver only answers administrators. Windows asks the user to confirm
//! every elevated start, except for scheduled tasks set to run with highest
//! privileges: so Glance, once elevated, registers such a task for itself,
//! and an ordinary start of Glance hands over to that task and exits. The
//! user confirms once, on the first start.
//!
//! A task like that runs whatever file is at its path, elevated, whenever
//! anyone running as the user asks. So Glance only makes one for an
//! executable no one but administrators can change: one installed under
//! Program Files, not a copy in Downloads or a build folder. From anywhere
//! else every start asks.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    CreateDirectoryW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_DATA, WRITE_DAC,
    WRITE_OWNER,
};
use windows::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, RRF_RT_REG_SZ,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ITaskFolder, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE, TASK_ENUM_HIDDEN,
    TASK_LOGON_INTERACTIVE_TOKEN,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// The task an ordinary start hands over to, and the one that starts Glance
/// when the user signs in.
const LAUNCH_TASK: &str = "Glance";
const LOGON_TASK: &str = "Glance at sign-in";
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

/// What an install folder may already hold: an earlier Glance.
const INSTALLED: [&str; 4] = ["glance.exe", "uninstall.exe", "resources", "licenses"];

/// What a folder chosen to install into is.
#[derive(Debug, PartialEq)]
pub enum InstallFolder {
    /// Glance can be installed there and start elevated without asking: no
    /// one but administrators can change any folder above it, and the folder
    /// is new (the installer creates it administrators' from the start) or
    /// an earlier Glance's that only administrators could ever change.
    Holds,
    /// A folder above it can be changed by others: whatever was installed
    /// there could be put in another's place.
    Open,
    /// It exists, and holds something besides an earlier Glance or could be
    /// changed by others: Glance is not installed into what it cannot vouch for.
    Occupied,
    /// It, or a folder above it, is a link or junction, or it is a drive's
    /// root: what it leads to is not what was chosen.
    Indirect,
}

/// The folder as chosen, if it is where it says: no part of it a link or
/// junction (what exists of it resolves to itself), and not a drive's root.
fn direct(folder: &Path) -> Option<PathBuf> {
    let folder = std::path::absolute(folder).ok()?;
    folder.parent()?;
    let existing = folder.ancestors().find(|dir| dir.exists())?;
    let resolved = std::fs::canonicalize(existing).ok()?;
    // canonicalize answers with the \\?\ prefix; compare without it.
    let plain = |path: &Path| path.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase();
    (plain(&resolved) == plain(existing)).then_some(folder)
}

/// Whether `folder` holds nothing but an earlier Glance.
fn only_ours(folder: &Path) -> bool {
    let ours = |name: &str| INSTALLED.iter().any(|known| known.eq_ignore_ascii_case(name));
    std::fs::read_dir(folder).is_ok_and(|entries| entries.flatten().all(|entry| ours(&entry.file_name().to_string_lossy())))
}

/// Whether `path` is a link, junction or other reparse point (not followed).
fn is_link(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    path.symlink_metadata().map_or(true, |meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

/// Whether anything inside `folder`, at any depth, is a link or junction
/// (none is followed while looking).
fn links_inside(folder: &Path) -> bool {
    std::fs::read_dir(folder).map_or(true, |entries| {
        entries.into_iter().any(|entry| {
            entry.map_or(true, |entry| {
                let path = entry.path();
                is_link(&path) || (path.is_dir() && links_inside(&path))
            })
        })
    })
}

/// Rights over a folder that would let someone change what is in it.
const FOLDER_CHANGES: u32 =
    FILE_ADD_FILE.0 | FILE_ADD_SUBDIRECTORY.0 | FILE_DELETE_CHILD.0 | DELETE.0 | WRITE_DAC.0 | WRITE_OWNER.0;
/// Rights over a file that would let someone change it.
const FILE_CHANGES: u32 = FILE_WRITE_DATA.0 | FILE_APPEND_DATA.0 | DELETE.0 | WRITE_DAC.0 | WRITE_OWNER.0;

/// Whether only administrators can change `folder` and everything in it,
/// none of it a link. Then no one else can have changed it, or hold it open
/// to change it now.
fn only_trusted_inside(folder: &Path) -> bool {
    !is_link(folder)
        // What it passes on to what is extracted into it counts too.
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

pub fn install_folder(folder: &Path) -> InstallFolder {
    let Some(folder) = direct(folder) else { return InstallFolder::Indirect };
    let above = folder.parent().and_then(|above| std::fs::canonicalize(above).ok());
    if !above.is_some_and(|above| folders_hold(&above)) {
        return InstallFolder::Open;
    }
    if folder.exists() && !(only_ours(&folder) && only_trusted_inside(&folder)) {
        return InstallFolder::Occupied;
    }
    InstallFolder::Holds
}

/// Readies `folder` for Glance to be put in it. A new folder is created
/// owned by the administrators, changed only by them and the system, and
/// read by everyone (as Program Files' folders are), in the one step that
/// creates it: no one else ever has it. An earlier Glance's folder, which
/// only administrators can change, is left as it is. Anything else is
/// refused, unless the user has chosen to install there `anyway`: then an
/// unprotected place or an existing folder is used as it is (a new folder
/// is still created administrators'), and Glance there asks for
/// administrator rights at every start and cannot start with Windows, as
/// it checks for itself. A link or a drive's root is refused either way:
/// the files would land elsewhere, or among others at the root, and could
/// not be removed cleanly. Nothing outside `folder` is touched.
pub fn prepare_install_folder(folder: &Path, anyway: bool) -> Result<(), InstallFolder> {
    match install_folder(folder) {
        InstallFolder::Holds => {}
        InstallFolder::Open | InstallFolder::Occupied if anyway => {}
        other => return Err(other),
    }
    let folder = direct(folder).ok_or(InstallFolder::Indirect)?;
    if anyway && folder.exists() {
        // Even so, nothing in it may lead elsewhere: the installer would
        // follow it and write there with administrator rights.
        if links_inside(&folder) {
            return Err(InstallFolder::Indirect);
        }
        // And what Glance puts there and later removes (its licenses folder
        // goes whole) must not be someone else's: either none of it is
        // there yet, or the folder is an earlier Glance's alone.
        let ours_there = INSTALLED.iter().any(|name| folder.join(name).exists());
        if ours_there && !only_ours(&folder) {
            return Err(InstallFolder::Occupied);
        }
    }
    if anyway && !folder.exists() {
        // Folders above a new one in an unprotected place, as they come.
        if let Some(above) = folder.parent() {
            std::fs::create_dir_all(above).map_err(|_| InstallFolder::Open)?;
        }
    }
    if !folder.exists() {
        let sddl = w!("O:BAD:PAI(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;BU)");
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut descriptor, None) }
            .map_err(|_| InstallFolder::Open)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let made = unsafe { CreateDirectoryW(&HSTRING::from(folder.as_os_str()), Some(&attributes)) };
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        made.map_err(|_| InstallFolder::Occupied)?;
    }
    // As Glance will check it when it starts (the user's choice aside).
    (anyway || only_ours(&folder) && only_trusted_inside(&folder)).then_some(()).ok_or(InstallFolder::Occupied)
}

/// `file` where it really is, if Glance may run it elevated without asking
/// (see `checked`), as a plain path.
pub fn trusted(file: &Path) -> Option<PathBuf> {
    checked(file).map(|real| plain(&real))
}

fn protected(file: &Path) -> bool {
    checked(file).is_some()
}

/// This executable as Task Scheduler should run it: where it really is,
/// without the `\\?\` prefix resolution adds.
fn this_program() -> Option<PathBuf> {
    Some(plain(&std::fs::canonicalize(std::env::current_exe().ok()?).ok()?))
}

/// A resolved path without the `\\?\` prefix (kept on a network path, which
/// needs it).
fn plain(real: &Path) -> PathBuf {
    let text = real.to_string_lossy();
    text.strip_prefix(r"\\?\").filter(|rest| !rest.starts_with("UNC\\")).map_or(real.to_path_buf(), PathBuf::from)
}

/// Whether this executable may be started elevated without asking: see the
/// module's notes.
pub fn may_start_unasked() -> bool {
    std::env::current_exe().is_ok_and(|exe| protected(&exe))
}

/// The current user's security identifier, which names their tasks: each
/// account that runs Glance has tasks of its own.
fn user_sid() -> Option<String> {
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
    [LOGON_TASK, LAUNCH_TASK].iter().any(|base| {
        name.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.strip_prefix(" S-1-").is_some_and(|sid| !sid.is_empty() && sid.chars().all(|c| c.is_ascii_digit() || c == '-')))
    })
}

/// Removes every account's Glance tasks: what the uninstaller asks of the
/// installed, protected copy of Glance, run elevated.
pub fn remove_all_tasks() {
    for name in glance_tasks() {
        delete_task(&name);
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

/// Whether the task `name` runs this very executable.
fn runs_this(name: &str) -> bool {
    let (Some(program), Some(this)) = (task_program(name), this_program()) else { return false };
    std::fs::canonicalize(&program).is_ok_and(|program| std::fs::canonicalize(&this).is_ok_and(|this| program == this))
}

/// Starts an elevated copy of Glance, through its task if that runs this
/// very executable, else by asking the user. Returns whether one was started.
pub fn relaunch_elevated() -> bool {
    let exe = std::env::current_exe().expect("own path");
    if runs_this(&launch_task()) && may_start_unasked() {
        let started = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(launch_task())).ok()?.Run(&VARIANT::default()).ok() });
        if started.is_some() {
            return true;
        }
    }
    let started = unsafe {
        ShellExecuteW(None, w!("runas"), &HSTRING::from(exe.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // ShellExecute reports success with a value above 32.
    started.0 as usize > 32
}

/// A task that runs `exe` with highest privileges, for the current user, on
/// demand or, with `at_sign_in`, when the user signs in.
fn task_xml(exe: &Path, at_sign_in: bool) -> String {
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
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command></Exec>
  </Actions>
</Task>"#,
        user = escape(&user),
        exe = escape(&exe.display().to_string()),
    )
}

/// Registers a task for this executable, handed to Task Scheduler directly
/// (no file in between). Needs elevation.
fn register_task(name: &str, at_sign_in: bool) -> bool {
    let Some(exe) = this_program() else { return false };
    let Some(folder) = task_folder() else { return false };
    let none = VARIANT::default();
    unsafe {
        folder
            .RegisterTask(
                &BSTR::from(name),
                &BSTR::from(task_xml(&exe, at_sign_in)),
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

/// Keeps the launch task pointing at this executable, if it may start
/// unasked; else makes sure no task starts it. Needs elevation.
pub fn register_launch_task() {
    if may_start_unasked() {
        let launches = register_task(&launch_task(), false);
        // Earlier versions' tasks, shared by every account under one name,
        // move to this account's own if they were this account's: starting
        // at sign-in carries over, and each old task goes only once its
        // replacement is registered.
        let mine = |name: &str| runs_this(name) && task_is_mine(name);
        if launches && mine(LAUNCH_TASK) {
            delete_task(LAUNCH_TASK);
        }
        if mine(LOGON_TASK) && register_task(&logon_task(), true) {
            delete_task(LOGON_TASK);
        }
    } else {
        // A copy that may not start unasked keeps no task that starts it,
        // whose account it is.
        for name in glance_tasks() {
            if runs_this(&name) {
                delete_task(&name);
            }
        }
    }
}

/// Whether this account starts this copy of Glance at sign-in.
pub fn autostart_enabled() -> bool {
    runs_this(&logon_task())
}

/// Turns starting at sign-in on or off; returns whether it is now on. Only
/// an executable that may start unasked can start at sign-in.
pub fn set_autostart(enabled: bool) -> bool {
    if enabled && may_start_unasked() {
        register_task(&logon_task(), true);
    } else if !enabled {
        delete_task(&logon_task());
    }
    autostart_enabled()
}

// ---- The driver ----

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
    let installed = Command::new(setup)
        .args(["-install", "-silent"])
        .creation_flags(NO_WINDOW)
        .status()
        .is_ok_and(|status| status.success());
    if installed && pawnio_installed() {
        mark_pawnio_ours();
    }
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
    use super::*;

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
        // Refused for where it is, and for what is in it.
        assert_eq!(install_folder(&earlier), InstallFolder::Open);
        assert_eq!(prepare_install_folder(&earlier, false), Err(InstallFolder::Open));
        assert!(!only_trusted_inside(&earlier));
        // Not even when the user chooses to install there all the same.
        assert!(links_inside(&earlier));
        assert_eq!(prepare_install_folder(&earlier, true), Err(InstallFolder::Indirect));
        std::fs::remove_dir(&inner).unwrap();
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }
}
