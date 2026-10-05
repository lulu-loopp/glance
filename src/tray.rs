//! The tray icon, on the main thread. A left click shows the readings, a
//! right click the settings: one interface each, rather than a menu.

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ChangeWindowMessageFilterEx, CreateWindowExW, ASFW_ANY, DefWindowProcW, DestroyWindow, FindWindowW, MSGFLT_ALLOW, DispatchMessageW, GetMessageW, GetSystemMetrics, LoadImageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, HICON, IMAGE_ICON, LR_SHARED, MSG, SM_CXSMICON, SM_CYSMICON,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WNDCLASSW,
};

/// The icon's messages to its window, and another start of Glance asking
/// for the settings.
const NOTIFY: u32 = WM_APP + 1;
const SHOW_SETTINGS: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Sent to every top-level window when the taskbar is created again (after
/// Explorer restarts): the icon has to be added again.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    data
}

fn add_icon(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = NOTIFY;
    // The program's icon, at the size the tray draws.
    data.hIcon = unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        HICON(
            LoadImageW(
                Some(instance.into()),
                PCWSTR(1 as _),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                // Shared: the system keeps one copy however often it is loaded.
                LR_SHARED,
            )
            .expect("program icon")
            .0,
        )
    };
    let tip: Vec<u16> = "Glance".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        NOTIFY => {
            // Version 4: the event in the low word.
            match (lparam.0 & 0xFFFF) as u32 {
                WM_LBUTTONUP => crate::app().controller.open_from_tray(),
                WM_CONTEXTMENU => crate::show_settings(),
                _ => {}
            }
            LRESULT(0)
        }
        SHOW_SETTINGS => {
            crate::show_settings();
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if message == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            add_icon(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Shows the icon and runs the main thread's messages until Glance quits.
pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: instance.into(), lpszClassName: w!("GlanceTray"), ..Default::default() };
        RegisterClassW(&class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        // A top-level window, though never shown: message-only windows do not
        // hear that the taskbar was created again.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GlanceTray"),
            w!("Glance"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("tray window");
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        // A start without administrator rights may ask too.
        let _ = ChangeWindowMessageFilterEx(hwnd, SHOW_SETTINGS, MSGFLT_ALLOW, None);
        add_icon(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
    // The other threads end with the process.
    std::process::exit(0);
}

/// Asks the Glance already running to open its settings.
pub fn ask_for_settings() {
    if let Ok(hwnd) = unsafe { FindWindowW(w!("GlanceTray"), None) } {
        // This start was the user's doing and may bring a window forward;
        // the running Glance may too.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        let _ = unsafe { PostMessageW(Some(hwnd), SHOW_SETTINGS, WPARAM(0), LPARAM(0)) };
    }
}

/// Removes the icon and ends Glance.
pub fn quit() {
    let hwnd = WINDOW.load(Ordering::Acquire);
    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}
