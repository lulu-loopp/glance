//! The tray icon, on the main thread. A left click shows the readings, a
//! right click the settings: one interface each, rather than a menu.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
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
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, DrawTextW, GetDC, ReleaseDC, SelectObject, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE,
};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};

/// The icon's messages to its window, and another start of Glance asking
/// for the settings.
const NOTIFY: u32 = WM_APP + 1;
const SHOW_SETTINGS: u32 = WM_APP + 2;
/// The settings changed: the shortcut is registered again, or let go.
const FOLLOW_SETTINGS: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
const HOTKEY_ID: i32 = 1;

/// The tooltip: the readings in brief, kept for when the icon is added again.
static TIP: Mutex<String> = Mutex::new(String::new());
/// The shortcut is wanted but another program has it.
static HOTKEY_TAKEN: AtomicBool = AtomicBool::new(false);

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
                WM_CONTEXTMENU => crate::show_settings(),
                // A notification of ours (a new version) clicked.
                NIN_BALLOONUSERCLICK => crate::show_settings(),
                _ => {}
            }
            LRESULT(0)
        }
        SHOW_SETTINGS => {
            crate::show_settings();
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

/// Sets the icon's tooltip, from any thread.
pub fn set_tip(text: &str) {
    *TIP.lock().unwrap() = text.to_string();
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    let mut data = icon_data(HWND(hwnd as *mut _));
    data.uFlags = NIF_TIP | NIF_SHOWTIP;
    put(&mut data.szTip, text);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// Rows of cells set as columns in the tooltip, in the tooltip's own font
/// (the system's status and tooltip font): the first cell of each row at the
/// left, every later one ending where the longest of its column ends, at
/// least a space after what comes before it. The room is filled with the
/// widest spaces that fit, then narrower ones (the tooltip holds only 127
/// characters), and every step measures the whole line as the tooltip draws
/// it: a space after Chinese is drawn in the font that lends the Chinese,
/// and figure spaces are not always as wide as digits.
///
/// Lined up, the lines take more characters than with a space between
/// cells: if they would not fit `budget` (newlines between them counted),
/// or the text cannot be measured, the cells are joined by a space.
pub fn columns(rows: &[Vec<String>], budget: usize) -> Vec<String> {
    // Em, en, thin and hair spaces, widest first.
    const SPACES: [&str; 4] = ["\u{2003}", "\u{2002}", "\u{2009}", "\u{200A}"];
    let plain: Vec<String> = rows.iter().map(|cells| cells.join(" ")).collect();
    let fits = |lines: &[String]| lines.iter().map(|line| line.encode_utf16().count()).sum::<usize>() + lines.len().saturating_sub(1) <= budget;
    let lined = in_tooltip_font(|width| {
        if width(" ") <= 0 {
            return None;
        }
        let mut lines: Vec<String> = rows.iter().map(|cells| cells.first().cloned().unwrap_or_default()).collect();
        let count = rows.iter().map(Vec::len).max().unwrap_or(0);
        for column in 1..count {
            let end = rows
                .iter()
                .zip(&lines)
                .filter_map(|(cells, line)| cells.get(column).map(|cell| width(&format!("{line} {cell}"))))
                .max()
                .unwrap_or(0);
            for (cells, line) in rows.iter().zip(lines.iter_mut()) {
                let Some(cell) = cells.get(column) else { continue };
                let reach = |pad: &str| width(&format!("{line}{pad}{cell}"));
                let mut pad = String::new();
                for fill in SPACES {
                    // A space that measures as nothing would never reach the end.
                    loop {
                        let (now, next) = (reach(&pad), reach(&(pad.clone() + fill)));
                        if next > end || next <= now {
                            break;
                        }
                        pad.push_str(fill);
                    }
                }
                // One more hair space if that comes nearer the end.
                if reach(&(pad.clone() + SPACES[3])) - end < end - reach(&pad) {
                    pad.push_str(SPACES[3]);
                }
                line.push_str(&pad);
                line.push_str(cell);
            }
        }
        Some(lines)
    });
    lined.filter(|lines| fits(lines)).unwrap_or(plain)
}

/// Runs `act` with a measure of text in the tooltip font, in pixels, as
/// DrawText (which the tooltip draws with) lays it out: GetTextExtentPoint32
/// measures some spaces apart from Latin text otherwise than it draws them.
fn in_tooltip_font<R>(act: impl FnOnce(&dyn Fn(&str) -> i32) -> R) -> R {
    unsafe {
        let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()), Default::default());
        let screen = GetDC(None);
        let font = CreateFontIndirectW(&metrics.lfStatusFont);
        let previous = SelectObject(screen, font.into());
        let width = |text: &str| {
            let mut units: Vec<u16> = text.encode_utf16().collect();
            let mut bounds = RECT::default();
            DrawTextW(screen, &mut units, &mut bounds, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            bounds.right - bounds.left
        };
        let result = act(&width);
        SelectObject(screen, previous);
        let _ = DeleteObject(font.into());
        ReleaseDC(None, screen);
        result
    }
}

/// Registers Ctrl+Alt+G if the settings want it, or lets it go.
fn register_hotkey(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
    }
    let wanted = crate::app().settings.lock().unwrap().hotkey;
    let taken = wanted && unsafe { RegisterHotKey(Some(hwnd), HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(b'G')) }.is_err();
    if taken && !HOTKEY_TAKEN.load(Ordering::Relaxed) {
        crate::journal::note("Ctrl+Alt+G is taken by another program");
    }
    HOTKEY_TAKEN.store(taken, Ordering::Relaxed);
}

/// Whether the shortcut is wanted but another program already has it.
pub fn hotkey_taken() -> bool {
    HOTKEY_TAKEN.load(Ordering::Relaxed)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|cell| cell.to_string()).collect()
    }

    fn sample() -> Vec<Vec<String>> {
        vec![
            row(&["CPU", "3%", "· 60 °C"]),
            row(&["GPU 1", "0%", "· 41 °C"]),
            row(&["GPU 2", "12%", "· 49 °C"]),
            row(&["内存", "100%"]),
            row(&["Memory", "40%"]),
        ]
    }

    #[test]
    fn lines_up_columns_in_the_tooltip_font() {
        let rows = sample();
        let lines = columns(&rows, 120);
        // Where each cell after the first ends, measured as the tooltip draws.
        let (ends, hair) = in_tooltip_font(|width| {
            let ends: Vec<Vec<i32>> = rows
                .iter()
                .zip(&lines)
                .map(|(cells, line)| {
                    let mut upto = 0;
                    cells
                        .iter()
                        .skip(1)
                        .map(|cell| {
                            upto += line[upto..].find(cell.as_str()).unwrap() + cell.len();
                            width(&line[..upto])
                        })
                        .collect()
                })
                .collect();
            (ends, width("\u{200A}").max(1))
        });
        for column in 0..2 {
            let at: Vec<i32> = ends.iter().filter_map(|row| row.get(column).copied()).collect();
            let (low, high) = (*at.iter().min().unwrap(), *at.iter().max().unwrap());
            assert!(high - low <= hair, "column {column}: {at:?}");
        }
        // "Glance" and the lines fit the tooltip's 127 characters.
        assert!(7 + lines.iter().map(|line| line.encode_utf16().count() + 1).sum::<usize>() <= 128);
    }

    #[test]
    fn falls_back_to_spaces_beyond_the_budget() {
        let size = |lines: &[String]| lines.iter().map(|line| line.encode_utf16().count()).sum::<usize>() + lines.len() - 1;
        let gpus: Vec<Vec<String>> = (1..=5).map(|i| row(&[&format!("GPU {i}"), &format!("{}%", i * 19), "· 40 °C"])).collect();
        for rows in [sample(), gpus] {
            let joined: Vec<String> = rows.iter().map(|cells| cells.join(" ")).collect();
            // Whatever the budget, what fits joined fits as given.
            for budget in size(&joined)..size(&joined) + 60 {
                assert!(size(&columns(&rows, budget)) <= budget, "{budget}");
            }
            // Too little for either: joined.
            assert_eq!(columns(&rows, 0), joined);
        }
    }

    /// Writes a tooltip's text, as Glance makes it, for a look at it drawn.
    #[test]
    #[ignore]
    fn writes_a_sample_tooltip() {
        let text = std::iter::once("Glance".to_string()).chain(columns(&sample(), 120)).collect::<Vec<_>>().join("\r\n");
        std::fs::write(std::env::var("TIP_OUT").unwrap(), text).unwrap();
    }
}
