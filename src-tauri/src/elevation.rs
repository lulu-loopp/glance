//! Running as administrator without asking every time, starting with
//! Windows, and the PawnIO driver the sensors need.
//!
//! The driver only answers administrators. Windows asks the user to confirm
//! every elevated start, except for scheduled tasks set to run with highest
//! privileges: so Glance, once elevated, registers such a task for itself,
//! and an ordinary start of Glance hands over to that task and exits. The
//! user confirms once, on the first start.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
    RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// The task an ordinary start hands over to, and the one that starts Glance
/// when the user signs in.
const LAUNCH_TASK: &str = "Glance";
const LOGON_TASK: &str = "Glance at sign-in";
const NO_WINDOW: u32 = 0x0800_0000;

/// Where Glance notes that it installed PawnIO itself, so that uninstalling
/// Glance removes the driver only then (another program may rely on it).
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

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(NO_WINDOW)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Starts an elevated copy of Glance, through its task if it has one, else
/// by asking the user. Returns whether one was started.
pub fn relaunch_elevated() -> bool {
    if schtasks(&["/Run", "/TN", LAUNCH_TASK]) {
        return true;
    }
    let exe = std::env::current_exe().expect("own path");
    let started = unsafe {
        ShellExecuteW(None, w!("runas"), &HSTRING::from(exe.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // ShellExecute reports success with a value above 32.
    started.0 as usize > 32
}

/// A task that runs this executable with highest privileges, for the
/// current user, on demand or, with `at_sign_in`, when the user signs in.
fn task_xml(exe: &Path, at_sign_in: bool) -> String {
    let user = std::env::var("USERDOMAIN").map(|domain| format!("{domain}\\")).unwrap_or_default()
        + &std::env::var("USERNAME").unwrap_or_default();
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
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

fn register_task(name: &str, at_sign_in: bool) -> bool {
    let exe = std::env::current_exe().expect("own path");
    let file = std::env::temp_dir().join(format!("glance-task-{}.xml", std::process::id()));
    // Task Scheduler reads the file as UTF-16, as the declaration says.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(task_xml(&exe, at_sign_in).encode_utf16().flat_map(|unit| unit.to_le_bytes()));
    if std::fs::write(&file, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/Create", "/TN", name, "/XML", &file.display().to_string(), "/F"]);
    let _ = std::fs::remove_file(&file);
    ok
}

/// Keeps the launch task pointing at this executable. Needs elevation.
pub fn register_launch_task() -> bool {
    register_task(LAUNCH_TASK, false)
}

pub fn autostart_enabled() -> bool {
    schtasks(&["/Query", "/TN", LOGON_TASK])
}

/// Turns starting at sign-in on or off; returns whether it is now on.
pub fn set_autostart(enabled: bool) -> bool {
    if enabled {
        register_task(LOGON_TASK, true);
    } else {
        schtasks(&["/Delete", "/TN", LOGON_TASK, "/F"]);
    }
    autostart_enabled()
}

fn pawnio_installed() -> bool {
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PAWNIO_UNINSTALL_KEY, w!("DisplayVersion"), RRF_RT_REG_SZ, None, None, Some(&mut size)) }
        .is_ok()
}

/// Installs the PawnIO driver from the setup shipped with Glance, if no
/// program has installed it yet, and notes that Glance did. Needs elevation.
pub fn ensure_pawnio(setup: &Path) {
    if pawnio_installed() || !setup.exists() {
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
    let _ = unsafe { RegSetValueExW(key, INSTALLED_PAWNIO, None, REG_DWORD, Some(&one)) };
}

