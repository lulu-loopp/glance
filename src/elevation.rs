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
//! executable the user cannot change without elevation: one installed under
//! Program Files, not a copy in Downloads or a build folder. From anywhere
//! else every start asks.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::core::{w, Interface, BOOL, BSTR, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Security::{
    AccessCheck, DuplicateToken, GetFileSecurityW, GetTokenInformation, SecurityIdentification, TokenElevation,
    TokenLinkedToken, DACL_SECURITY_INFORMATION, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PRIVILEGE_SET, PSECURITY_DESCRIPTOR, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_LINKED_TOKEN, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_DATA, WRITE_DAC, WRITE_OWNER,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, RRF_RT_REG_SZ,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ITaskFolder, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN,
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

/// The user's own rights without elevation, as a token to check access
/// against: the elevated token's linked one, or the process's own.
fn limited_token() -> Option<HANDLE> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &mut token) }.ok()?;
    let mut linked = TOKEN_LINKED_TOKEN::default();
    let mut size = 0u32;
    let elevated = is_elevated()
        && unsafe {
            GetTokenInformation(token, TokenLinkedToken, Some(&mut linked as *mut _ as *mut _), size_of::<TOKEN_LINKED_TOKEN>() as u32, &mut size)
        }
        .is_ok();
    let source = if elevated { linked.LinkedToken } else { token };
    let mut checkable = HANDLE::default();
    let duplicated = unsafe { DuplicateToken(source, SecurityIdentification, &mut checkable) };
    unsafe {
        if elevated {
            let _ = CloseHandle(linked.LinkedToken);
        }
        let _ = CloseHandle(token);
    }
    duplicated.ok().map(|_| checkable)
}

/// The rights `token` has to `path`, as Windows grants them.
fn rights(token: HANDLE, path: &Path) -> Option<u32> {
    let wide = HSTRING::from(path.as_os_str());
    let wanted = (OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION).0;
    let mut needed = 0u32;
    unsafe {
        let _ = GetFileSecurityW(&wide, wanted, None, 0, &mut needed);
    }
    let mut descriptor = vec![0u8; needed as usize];
    unsafe {
        GetFileSecurityW(&wide, wanted, Some(PSECURITY_DESCRIPTOR(descriptor.as_mut_ptr().cast())), needed, &mut needed)
            .as_bool()
            .then_some(())?
    };
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: GENERIC_ALL.0,
    };
    let mut privileges = [0u8; 256];
    let mut privileges_size = privileges.len() as u32;
    let (mut granted, mut status) = (0u32, BOOL(0));
    unsafe {
        AccessCheck(
            PSECURITY_DESCRIPTOR(descriptor.as_mut_ptr().cast()),
            token,
            windows::Win32::System::SystemServices::MAXIMUM_ALLOWED,
            &mapping,
            Some(privileges.as_mut_ptr() as *mut PRIVILEGE_SET),
            &mut privileges_size,
            &mut granted,
            &mut status,
        )
        .ok()?
    };
    Some(if status.as_bool() { granted } else { 0 })
}

/// Whether the user, without elevation, can neither change `file` nor put
/// anything else in its place: not alter or replace it, not add files beside
/// it (which Windows might load with it), and not rename or re-permission any
/// folder above it.
fn protected(file: &Path) -> bool {
    let Some(token) = limited_token() else { return false };
    let clear = |path: &Path, forbidden: u32| rights(token, path).is_some_and(|granted| granted & forbidden == 0);
    let file_ok = clear(file, (FILE_WRITE_DATA | FILE_APPEND_DATA | DELETE | WRITE_DAC | WRITE_OWNER).0);
    let mut folders_ok = true;
    let mut folder = file.parent();
    let mut first = true;
    while let Some(dir) = folder {
        // Its own folder takes no new files; no folder above it can be
        // emptied of a child, renamed or re-permissioned.
        let adding = if first { (FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY).0 } else { 0 };
        folders_ok &= clear(dir, adding | (FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER).0);
        first = false;
        folder = dir.parent();
    }
    let _ = unsafe { CloseHandle(token) };
    file_ok && folders_ok
}

/// Whether this executable may be started elevated without asking: see the
/// module's notes.
pub fn may_start_unasked() -> bool {
    std::env::current_exe().is_ok_and(|exe| protected(&exe))
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

fn runs_this(name: &str, exe: &Path) -> bool {
    task_program(name).is_some_and(|program| Path::new(&program) == exe)
}

/// Starts an elevated copy of Glance, through its task if that runs this
/// very executable, else by asking the user. Returns whether one was started.
pub fn relaunch_elevated() -> bool {
    let exe = std::env::current_exe().expect("own path");
    if runs_this(LAUNCH_TASK, &exe) && may_start_unasked() {
        let started = task_folder().and_then(|folder| unsafe { folder.GetTask(&BSTR::from(LAUNCH_TASK)).ok()?.Run(&VARIANT::default()).ok() });
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
    let exe = std::env::current_exe().expect("own path");
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
    let exe = std::env::current_exe().expect("own path");
    if may_start_unasked() {
        register_task(LAUNCH_TASK, false);
    } else {
        // A task for this copy, made by an earlier version, goes.
        for name in [LAUNCH_TASK, LOGON_TASK] {
            if runs_this(name, &exe) {
                delete_task(name);
            }
        }
    }
}

pub fn autostart_enabled() -> bool {
    task_program(LOGON_TASK).is_some()
}

/// Turns starting at sign-in on or off; returns whether it is now on. Only
/// an executable that may start unasked can start at sign-in.
pub fn set_autostart(enabled: bool) -> bool {
    if enabled && may_start_unasked() {
        register_task(LOGON_TASK, true);
    } else if !enabled {
        delete_task(LOGON_TASK);
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
    if pawnio_installed() || !setup.exists() || !protected(setup) {
        return;
    }
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
    fn tells_protected_places_from_writable_ones() {
        // Windows' own programs cannot be changed by the user; this build can.
        assert!(protected(Path::new(r"C:\Windows\System32\notepad.exe")));
        assert!(!protected(&std::env::current_exe().unwrap()));
    }
}
