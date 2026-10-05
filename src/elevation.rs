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
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    GetAce, GetTokenInformation, MapGenericMask, TokenElevation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, GENERIC_MAPPING, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_DELETE_CHILD,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_DATA, WRITE_DAC, WRITE_OWNER,
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
            // Entries that only pass on to children do not apply here.
            if header.AceFlags as u32 & INHERIT_ONLY_ACE.0 != 0 {
                continue;
            }
            match header.AceType as u32 {
                ACCESS_DENIED_ACE_TYPE => {}
                ACCESS_ALLOWED_ACE_TYPE => {
                    let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
                    let mut mask = allowed.Mask;
                    unsafe { MapGenericMask(&mut mask, &mapping) };
                    if mask & forbidden != 0 && !trusted(PSID(&allowed.SidStart as *const u32 as *mut _)) {
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
    let mut folder = file.parent();
    let mut own = true;
    while let Some(dir) = folder {
        // Its own folder takes no new files; no folder above it can lose a
        // child, be renamed or be re-permissioned.
        let adding = if own { (FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY).0 } else { 0 };
        clear &= only_trusted_can(dir, adding | (FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER).0);
        own = false;
        folder = dir.parent();
    }
    clear.then_some(file)
}

fn protected(file: &Path) -> bool {
    checked(file).is_some()
}

/// This executable as Task Scheduler should run it: where it really is,
/// without the `\\?\` prefix resolution adds.
fn this_program() -> Option<PathBuf> {
    let real = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let text = real.to_string_lossy();
    Some(text.strip_prefix(r"\\?\").filter(|rest| !rest.starts_with("UNC\\")).map_or(real.clone(), PathBuf::from))
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
    let Some(folder) = task_folder() else { return };
    let names: Vec<String> = unsafe {
        let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) else { return };
        let count = tasks.Count().unwrap_or(0);
        (1..=count).filter_map(|i| tasks.get_Item(&VARIANT::from(i)).ok()?.Name().ok().map(|n| n.to_string())).collect()
    };
    for name in names.iter().filter(|name| is_glance_task(name)) {
        delete_task(name);
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
    // Tasks under the names earlier versions used go if they run this copy;
    // starting at sign-in carries over to this account's own task.
    let started_at_sign_in = runs_this(LOGON_TASK);
    for name in [LAUNCH_TASK, LOGON_TASK] {
        if runs_this(name) {
            delete_task(name);
        }
    }
    if may_start_unasked() {
        register_task(&launch_task(), false);
        if started_at_sign_in {
            register_task(&logon_task(), true);
        }
    } else {
        // A copy that may not start unasked keeps no task that starts it.
        for name in [launch_task(), logon_task()] {
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
}
