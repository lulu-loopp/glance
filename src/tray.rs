//! The tray icon, on the main thread. A left click shows the readings; a
//! right click, a short menu: the readings, the settings, a check for
//! updates, and quitting.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ChangeWindowMessageFilterEx, CreateWindowExW, ASFW_ANY, DefWindowProcW, DestroyWindow, FindWindowW, MSGFLT_ALLOW, DispatchMessageW, GetMessageW, GetSystemMetrics, LoadImageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, HICON, IMAGE_ICON, LR_SHARED, MSG, SM_CXSMICON, SM_CYSMICON,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WNDCLASSW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, MapVirtualKeyW, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

use crate::settings::Shortcut;

/// The icon's messages to its window, and another start of Glance asking
/// for the settings.
const NOTIFY: u32 = WM_APP + 1;
const SHOW_SETTINGS: u32 = WM_APP + 2;
/// The settings changed: the shortcut is registered again, or let go.
const FOLLOW_SETTINGS: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
const HOTKEY_ID: i32 = 1;
/// The shortcut that opens global search.
const SEARCH_HOTKEY_ID: i32 = 2;

/// The tooltip: the readings in brief, kept for when the icon is added again.
static TIP: Mutex<String> = Mutex::new(String::new());
/// The icon's menu is up (see `open_menu`).
static MENU_UP: AtomicBool = AtomicBool::new(false);
/// The shortcut is wanted but another program has it.
static HOTKEY_TAKEN: AtomicBool = AtomicBool::new(false);
/// The shortcut is let go while the settings take a new one.
static HOTKEY_HELD: AtomicBool = AtomicBool::new(false);
/// Global search's shortcut: wanted but another program has it, or let go
/// while the settings take a new one.
static SEARCH_HOTKEY_TAKEN: AtomicBool = AtomicBool::new(false);
static SEARCH_HOTKEY_HELD: AtomicBool = AtomicBool::new(false);

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
    let tip = TIP.lock().unwrap();
    put(&mut data.szTip, if tip.is_empty() { "Glance" } else { &tip });
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
                // Version 4: where to put the menu, in the WPARAM.
                WM_CONTEXTMENU => menu(hwnd, (wparam.0 & 0xFFFF) as u16 as i16 as i32, ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32),
                // A notification of ours clicked: the settings, at the reports' page for news of a report, poured out
                // of the icon.
                NIN_BALLOONUSERCLICK if OF_REPORTS.load(Ordering::Relaxed) => crate::show_reports(icon_on_screen(hwnd)),
                NIN_BALLOONUSERCLICK => crate::show_settings(icon_on_screen(hwnd)),
                _ => {}
            }
            LRESULT(0)
        }
        SHOW_SETTINGS => {
            crate::show_settings(None);
            LRESULT(0)
        }
        FOLLOW_SETTINGS => {
            register_hotkey(hwnd);
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 == HOTKEY_ID as usize => {
            crate::app().controller.toggle();
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 == SEARCH_HOTKEY_ID as usize => {
            crate::ui::search::toggle();
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

/// The tray's window class, which another start of the same Glance finds
/// (a debug build's is its own).
#[cfg(not(debug_assertions))]
const TRAY_CLASS: PCWSTR = w!("GlanceTray");
#[cfg(debug_assertions)]
const TRAY_CLASS: PCWSTR = w!("GlanceTray.Debug");

/// Shows the icon and runs the main thread's messages until Glance quits.
pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: instance.into(), lpszClassName: TRAY_CLASS, ..Default::default() };
        RegisterClassW(&class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        // A top-level window, though never shown: message-only windows do not
        // hear that the taskbar was created again.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            TRAY_CLASS,
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
        register_hotkey(hwnd);
        // A first start since an update says so, from the icon now there.
        crate::update::tell_if_updated();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

/// Asks the Glance already running to open its settings.
pub fn ask_for_settings() {
    if let Ok(hwnd) = unsafe { FindWindowW(TRAY_CLASS, None) } {
        // This start was the user's doing and may bring a window forward;
        // the running Glance may too.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        let _ = unsafe { PostMessageW(Some(hwnd), SHOW_SETTINGS, WPARAM(0), LPARAM(0)) };
    }
}

/// Shows a notification from the icon, from any thread; clicking it opens
/// the settings.
pub fn notify(title: &str, text: &str) {
    show_notice(title, text, false);
}

/// The last notification shown was news of a report the user sent: clicked, it opens the reports' page.
static OF_REPORTS: AtomicBool = AtomicBool::new(false);

/// Shows news of a report the user sent, as `notify` does: clicked, it opens the settings at the reports' page.
pub fn notify_reports(title: &str, text: &str) {
    show_notice(title, text, true);
}

fn show_notice(title: &str, text: &str, of_reports: bool) {
    OF_REPORTS.store(of_reports, Ordering::Relaxed);
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    let mut data = icon_data(HWND(hwnd as *mut _));
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    put(&mut data.szInfoTitle, title);
    put(&mut data.szInfo, text);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// `text` in a fixed buffer, cut to fit, a terminating nul left in place.
fn put(into: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(into.len() - 1).collect();
    into[..units.len()].copy_from_slice(&units);
}

/// The icon's menu, at `(x, y)` on the screen: shown by the panel's thread,
/// which draws Glance's menus (see `open_menu`).
fn menu(_hwnd: HWND, x: i32, y: i32) {
    crate::app().controller.tray_menu(x, y);
}

/// What the choices of the icon's menu are (see `open_menu`).
const OPEN: usize = 1;
const SETTINGS: usize = 2;
const CHECK: usize = 3;
const QUIT: usize = 4;
const RECALL: usize = 5;

/// The icon's menu, opening up from `(x, y)` on the screen as the taskbar's do, up (see `menu::Menu`), with what each
/// of its choices is (see `chosen`). On the panel's thread.
/// (`over_the_panel`: opened with the panel up: see `Menu::over_the_panel`.)
pub fn open_menu(x: i32, y: i32, over_the_panel: bool) -> Option<(crate::ui::menu::Menu, Vec<usize>)> {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{Item, Menu};
    let zh = crate::update::language() == crate::ui::text::Lang::Zh;
    let recall = crate::app().controller.widget_put_away();
    // Each choice, its icon, and whether a rule sets it off; no widget to
    // bring back, no choice for it.
    let mut choices = vec![(OPEN, if zh { "打开面板" } else { "Open the panel" }, Icon::Panel, false)];
    if recall {
        choices.push((RECALL, if zh { "召回小组件" } else { "Bring the widget back" }, Icon::Restore, false));
    }
    choices.push((SETTINGS, if zh { "设置" } else { "Settings" }, Icon::Settings, false));
    choices.push((CHECK, if zh { "检查更新" } else { "Check for updates" }, Icon::Update, false));
    choices.push((QUIT, if zh { "退出 Glance" } else { "Quit Glance" }, Icon::Quit, true));
    let items: Vec<Item<'static>> = choices.iter().map(|(_, label, icon, rule_before)| Item { label, mark: Some(crate::ui::menu::Mark::Icon(*icon)), checked: false, rule_before: *rule_before }).collect();
    MENU_UP.store(true, Ordering::Relaxed);
    show_tip(false);
    let Some(menu) = Menu::open_at(items, crate::ui::menu::Look::system(), POINT { x, y }, true, over_the_panel) else {
        chosen(None, &[]);
        return None;
    };
    Some((menu, choices.into_iter().map(|(choice, ..)| choice).collect()))
}

/// Where the icon is on the screen (physical px), if it shows (in the tray's overflow, its chevron's).
pub(crate) fn icon_place() -> Option<RECT> {
    let hwnd = WINDOW.load(Ordering::Acquire);
    (hwnd != 0).then(|| icon_on_screen(HWND(hwnd as *mut _)))?
}

fn icon_on_screen(hwnd: HWND) -> Option<RECT> {
    use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
    let icon = NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() };
    unsafe { Shell_NotifyIconGetRect(&icon) }.ok()
}

/// The icon's menu gone, `choice` chosen in it (of `choices`, as `open_menu` gave them), if one was: done. (What it
/// opens comes out of the icon, as the menu did, not the menu's row, gone as it opens.)
pub fn chosen(choice: Option<usize>, choices: &[usize]) {
    MENU_UP.store(false, Ordering::Relaxed);
    show_tip(true);
    match choice.map_or(0, |i| choices[i]) {
        OPEN => crate::app().controller.open_from_tray(),
        RECALL => crate::app().controller.recall_widget(),
        SETTINGS => crate::show_settings(icon_place()),
        CHECK => crate::update::check_now(crate::update::Asker::Tray),
        QUIT => crate::quit(),
        _ => {}
    }
}

/// Sets the icon's tooltip, from any thread.
pub fn set_tip(text: &str) {
    *TIP.lock().unwrap() = text.to_string();
    show_tip(!MENU_UP.load(Ordering::Relaxed));
}

/// The icon's tooltip as it now reads, shown as the pointer rests on the
/// icon (`show`), or not (while the icon's menu is up, over which it would
/// show).
fn show_tip(show: bool) {
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    let text = TIP.lock().unwrap().clone();
    let mut data = icon_data(HWND(hwnd as *mut _));
    data.uFlags = if show { NIF_TIP | NIF_SHOWTIP } else { NIF_TIP };
    put(&mut data.szTip, if text.is_empty() { "Glance" } else { &text });
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// Registers the shortcuts if there are any (and they are not held back
/// while new ones are being chosen), or lets them go.
fn register_hotkey(hwnd: HWND) {
    let (shortcut, search) = {
        let settings = crate::app().settings.lock().unwrap();
        (settings.shortcut, settings.search_shortcut)
    };
    register_one(hwnd, HOTKEY_ID, shortcut.filter(|_| !HOTKEY_HELD.load(Ordering::Relaxed)), &HOTKEY_TAKEN, "");
    register_one(hwnd, SEARCH_HOTKEY_ID, search.filter(|_| !SEARCH_HOTKEY_HELD.load(Ordering::Relaxed)), &SEARCH_HOTKEY_TAKEN, "search ");
}

/// Registers `shortcut` for `id`, or lets it go; whether another program
/// has it is kept in `taken` and noted once.
fn register_one(hwnd: HWND, id: i32, shortcut: Option<Shortcut>, taken: &AtomicBool, what: &str) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), id);
    }
    let Some(shortcut) = shortcut else {
        taken.store(false, Ordering::Relaxed);
        return;
    };
    let mut modifiers = MOD_NOREPEAT;
    for (held, flag) in [(shortcut.ctrl, MOD_CONTROL), (shortcut.alt, MOD_ALT), (shortcut.shift, MOD_SHIFT), (shortcut.win, MOD_WIN)] {
        if held {
            modifiers |= flag;
        }
    }
    let is_taken = unsafe { RegisterHotKey(Some(hwnd), id, HOT_KEY_MODIFIERS(modifiers.0), u32::from(shortcut.key)) }.is_err();
    if is_taken && !taken.load(Ordering::Relaxed) {
        crate::journal::note(format!("{what}{} is taken by another program", shortcut_name(shortcut)));
    }
    taken.store(is_taken, Ordering::Relaxed);
}

/// Lets the shortcut go while a new one is being chosen (its keys would
/// otherwise never reach the settings), or takes it up again.
pub fn hold_hotkey(held: bool) {
    HOTKEY_HELD.store(held, Ordering::Relaxed);
    follow_settings();
}

/// The same for global search's shortcut.
pub fn hold_search_hotkey(held: bool) {
    SEARCH_HOTKEY_HELD.store(held, Ordering::Relaxed);
    follow_settings();
}

/// A shortcut as it is written: "Ctrl+Alt+G", the key by the name the
/// keyboard's layout gives it.
pub fn shortcut_name(shortcut: Shortcut) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (held, name) in [(shortcut.ctrl, "Ctrl"), (shortcut.alt, "Alt"), (shortcut.shift, "Shift"), (shortcut.win, "Win")] {
        if held {
            parts.push(name.to_string());
        }
    }
    parts.push(key_name(shortcut.key));
    parts.join("+")
}

fn key_name(key: u16) -> String {
    match key {
        // Letters and digits as they are; the function keys by number.
        0x30..=0x39 | 0x41..=0x5A => char::from(key as u8).to_string(),
        0x70..=0x87 => format!("F{}", key - 0x6F),
        _ => {
            let scan = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_VSC) };
            // The keys of the navigation block and the arrows are "extended":
            // without the flag their names are the number pad's.
            let extended = matches!(key, 0x21..=0x2E | 0x5B..=0x5D | 0x6F | 0x90);
            let lparam = ((scan as i32) << 16) | if extended { 1 << 24 } else { 0 };
            let mut name = [0u16; 64];
            let length = unsafe { GetKeyNameTextW(lparam, &mut name) };
            if length > 0 {
                String::from_utf16_lossy(&name[..length as usize])
            } else {
                format!("0x{key:02X}")
            }
        }
    }
}

/// Whether the shortcut is wanted but another program already has it.
pub fn hotkey_taken() -> bool {
    HOTKEY_TAKEN.load(Ordering::Relaxed)
}

/// Whether global search's shortcut is wanted but another program already has it.
pub fn search_hotkey_taken() -> bool {
    SEARCH_HOTKEY_TAKEN.load(Ordering::Relaxed)
}

/// Has the shortcut follow the settings, from any thread.
pub fn follow_settings() {
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd != 0 {
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), FOLLOW_SETTINGS, WPARAM(0), LPARAM(0)) };
    }
}

/// Removes the icon and ends Glance.
pub fn quit() {
    let hwnd = WINDOW.load(Ordering::Acquire);
    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}
