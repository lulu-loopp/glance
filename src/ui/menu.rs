//! A menu as Windows 11 draws its own (File Explorer's): a rounded sheet
//! with a soft shadow, a row for each choice with its icon, a light
//! highlight under the pointer, a tick for a choice that is on, thin rules
//! between groups. Shown at the pointer, it runs until a choice is made or
//! it is left (a click elsewhere, Esc, Alt, another window to the front),
//! as a system menu does.

use windows::Win32::Foundation::{LPARAM, POINT, RECT, WPARAM};
use windows::Win32::System::Threading::{GetCurrentThreadId, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, GetForegroundWindow, IsIconic, SC_RESTORE, WM_SYSCOMMAND, MsgWaitForMultipleObjects, PeekMessageW, PostMessageW, PostQuitMessage, PostThreadMessageW, SetForegroundWindow,
    TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SYSKEYDOWN,
};

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Point};
use super::gfx::{self, Surface};
use super::icons::Icon;
use super::window::{Window, CAPTURE_LOST};

/// One choice: what it says, its icon, whether it is on (ticked), and
/// whether a rule sets it off from the choices before it.
pub struct Item<'a> {
    pub label: &'a str,
    pub icon: Option<Icon>,
    pub checked: bool,
    pub rule_before: bool,
}

/// Its measures (DIPs), as Windows 11's menus have them.
const ROW: f32 = 32.0;
const RULE: f32 = 9.0;
const INSET: f32 = 4.0;
const RADIUS: f32 = 8.0;
const HIGHLIGHT_RADIUS: f32 = 4.0;
const ICON: f32 = 16.0;
const TEXT_LEFT: f32 = 44.0;
const MIN_WIDTH: f32 = 200.0;
/// Room round the sheet for its shadow.
const SHADOW: f32 = 16.0;
const FONT: Font = Font::new(Family::Segoe, 14.0, 400.0);

/// Shows `items` at the pointer, light or `dark`; the index of the one
/// chosen, if one is.
pub fn show(items: &[Item], dark: bool) -> Option<usize> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.ok()?;
    show_at(items, dark, cursor, false)
}

/// Shows `items` with a corner at `at` (physical px), its bottom there for
/// `above` (as a menu from the taskbar opens), light or `dark`.
pub fn show_at(items: &[Item], dark: bool, at: POINT, above: bool) -> Option<usize> {
    let cursor = at;
    let contact = crate::panel::monitor_at(cursor)?;
    let gfx = gfx::current().ok()?;
    let window = Window::activating().ok()?;
    let Ok(mut surface) = Surface::new(&gfx, window.hwnd) else {
        window.destroy();
        gfx::lost();
        return None;
    };
    let scale = contact.scale;
    let sheet = Sheet::new(&gfx, items);
    let (width, height, tops) = (sheet.width, sheet.height, &sheet.tops);
    // At the pointer, turned back where it would leave its screen.
    let (w, h) = (((width + 2.0 * SHADOW) * scale) as i32, ((height + 2.0 * SHADOW) * scale) as i32);
    let margin = (SHADOW * scale) as i32;
    let work = contact.work;
    let mut left = cursor.x - margin;
    if left + w - margin > work.right {
        left = cursor.x - w + margin;
    }
    let mut top = cursor.y - margin;
    if above || top + h - margin > work.bottom {
        top = cursor.y - h + margin;
    }
    let left = left.clamp(work.left - margin, (work.right - w + margin).max(work.left - margin));
    let top = top.clamp(work.top - margin, (work.bottom - h + margin).max(work.top - margin));
    let rect = RECT { left, top, right: left + w, bottom: top + h };
    let draw = |surface: &mut Surface, lit: Option<usize>| {
        let drawn = surface.draw(&gfx, (w as u32, h as u32), scale, |frame| {
            frame.origin(SHADOW, SHADOW);
            sheet.paint(frame, items, dark, lit);
        });
        if drawn.is_err() {
            gfx::lost();
        }
        drawn.is_ok()
    };
    // Which choice a point in the window (physical px) is on.
    let row_at = |x: i32, y: i32| {
        let (x, y) = (x as f32 / scale - SHADOW, y as f32 / scale - SHADOW);
        if x < 0.0 || x > width {
            return None;
        }
        tops.iter().position(|top| y >= *top && y < top + ROW)
    };
    let inside = |x: i32, y: i32| {
        let (x, y) = (x as f32 / scale - SHADOW, y as f32 / scale - SHADOW);
        x >= 0.0 && x <= width && y >= 0.0 && y <= height
    };
    let mut lit: Option<usize> = None;
    if !draw(&mut surface, lit) {
        window.destroy();
        return None;
    }
    window.place(rect);
    window.show();
    // Up, and gone: a widget that took the desktop with it there takes it
    // again.
    crate::widget::own_window_changed(rect);
    // In front, with the keyboard and the mouse, as a system menu is: its
    // keys come to it, not to the program behind; a click anywhere comes
    // to it too; and it is left as anything else comes to the front.
    let before = unsafe { GetForegroundWindow() };
    let active = unsafe { SetForegroundWindow(window.hwnd) }.as_bool();
    // Not let to the front (Windows keeps it for the program in use): it
    // could neither hear its keys nor know it was left, so it goes.
    if !active {
        crate::journal::note("menu: not let to the front; not shown");
        window.destroy();
        return None;
    }
    unsafe { SetCapture(window.hwnd) };
    // What comes meanwhile for this thread's other windows, or for the
    // thread, that only its own loop knows what to do with: posted again,
    // as it came, once the menu is gone.
    let mut deferred: Vec<MSG> = Vec::new();
    let mut quit = None;
    let chosen = 'menu: loop {
        if unsafe { GetForegroundWindow() } != window.hwnd {
            break None;
        }
        unsafe { MsgWaitForMultipleObjects(None, false, INFINITE, QS_ALLINPUT) };
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                quit = Some(msg.wParam.0 as i32);
                break 'menu None;
            }
            if msg.hwnd == window.hwnd {
                let (x, y) = ((msg.lParam.0 & 0xFFFF) as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32);
                match msg.message {
                    WM_MOUSEMOVE => {
                        let row = row_at(x, y);
                        if row != lit {
                            lit = row;
                            if !draw(&mut surface, lit) {
                                break 'menu None;
                            }
                        }
                    }
                    // A press outside leaves it; inside, the choice is made
                    // as the button comes up.
                    WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !inside(x, y) => break 'menu None,
                    WM_LBUTTONUP | WM_RBUTTONUP if inside(x, y) => {
                        if let Some(row) = row_at(x, y) {
                            break 'menu Some(row);
                        }
                    }
                    WM_KEYDOWN => match VIRTUAL_KEY(msg.wParam.0 as u16) {
                        VK_ESCAPE => break 'menu None,
                        VK_UP | VK_DOWN => {
                            let n = items.len();
                            lit = Some(match (lit, VIRTUAL_KEY(msg.wParam.0 as u16) == VK_UP) {
                                (None, true) => n - 1,
                                (None, false) => 0,
                                (Some(i), true) => (i + n - 1) % n,
                                (Some(i), false) => (i + 1) % n,
                            });
                            if !draw(&mut surface, lit) {
                                break 'menu None;
                            }
                        }
                        VK_RETURN if lit.is_some() => break 'menu lit,
                        _ => {}
                    },
                    // Alt, as with a system menu, or the mouse taken from it.
                    WM_SYSKEYDOWN | CAPTURE_LOST => break 'menu None,
                    _ => {}
                }
                continue;
            }
            // A shortcut leaves the menu; Esc (the panel's, while it is open)
            // is the menu's own, the rest are taken up once it is gone.
            if msg.message == WM_HOTKEY {
                let (modifiers, key) = (msg.lParam.0 & 0xFFFF, (msg.lParam.0 >> 16) & 0xFFFF);
                if !(modifiers == 0 && key == VK_ESCAPE.0 as isize) {
                    deferred.push(msg);
                }
                break 'menu None;
            }
            if msg.hwnd.is_invalid() || (WM_APP..0xC000).contains(&msg.message) {
                deferred.push(msg);
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    };
    // The focus back where it was, unless something else has taken it.
    let held = unsafe { GetForegroundWindow() } == window.hwnd;
    unsafe {
        let _ = ReleaseCapture();
    }
    window.destroy();
    crate::widget::own_window_changed(rect);
    // (A game that went down, a fullscreen one stepping out as the menu
    // came, is restored as a click on its taskbar button would.)
    if held && !before.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(before);
            if IsIconic(before).as_bool() {
                let _ = PostMessageW(Some(before), WM_SYSCOMMAND, WPARAM(SC_RESTORE as usize), LPARAM(0));
            }
        }
    }
    let thread = unsafe { GetCurrentThreadId() };
    for msg in deferred {
        unsafe {
            let _ = if msg.hwnd.is_invalid() { PostThreadMessageW(thread, msg.message, msg.wParam, msg.lParam) } else { PostMessageW(Some(msg.hwnd), msg.message, msg.wParam, msg.lParam) };
        }
    }
    if let Some(code) = quit {
        unsafe { PostQuitMessage(code) };
    }
    chosen
}

/// A menu's sheet laid out for its items: its size (DIPs) and where each
/// row starts.
pub struct Sheet {
    pub width: f32,
    pub height: f32,
    tops: Vec<f32>,
}

impl Sheet {
    pub fn new(gfx: &gfx::Gfx, items: &[Item]) -> Self {
        // As wide as its longest choice.
        let widest = items.iter().map(|item| gfx.measure(item.label, FONT)).fold(0.0, f32::max);
        let width = (TEXT_LEFT + widest + 48.0).max(MIN_WIDTH);
        let tops: Vec<f32> = items
            .iter()
            .scan(INSET, |y, item| {
                *y += if item.rule_before { RULE } else { 0.0 };
                let top = *y;
                *y += ROW;
                Some(top)
            })
            .collect();
        let height = tops.last().map_or(INSET, |top| top + ROW) + INSET;
        Sheet { width, height, tops }
    }

    /// Draws it, its corner at the frame's origin (its shadow beyond),
    /// light or `dark`, the choice `lit` highlighted.
    pub fn paint(&self, frame: &gfx::Frame, items: &[Item], dark: bool, lit: Option<usize>) {
        let (width, height) = (self.width, self.height);
        let colours = Colours::new(dark);
        frame.shadow(colours.shadow, 0.0, 0.0, width, height, RADIUS, 12.0, 6.0);
        frame.fill_rounded(colours.sheet, 0.0, 0.0, width, height, RADIUS);
        frame.stroke_rounded(Fill::Solid(colours.stroke), 0.5, 0.5, width - 1.0, height - 1.0, RADIUS, 1.0);
        for (i, (item, top)) in items.iter().zip(&self.tops).enumerate() {
            if item.rule_before {
                frame.fill(colours.rule, 0.0, top - RULE / 2.0 - 0.5, width, 1.0);
            }
            if lit == Some(i) {
                frame.fill_rounded(colours.highlight, INSET, *top, width - 2.0 * INSET, ROW, HIGHLIGHT_RADIUS);
            }
            let middle = top + ROW / 2.0;
            if let Some(icon) = item.icon {
                frame.icon(icon, Point { x: INSET + 16.0, y: middle }, ICON, colours.icon);
            }
            let (ink_top, ink_bottom) = frame.ink(item.label, FONT);
            frame.text(item.label, FONT, colours.text, TEXT_LEFT, middle - (ink_top + ink_bottom) / 2.0, width - TEXT_LEFT - 36.0, Align::Start);
            if item.checked {
                frame.icon(Icon::Check, Point { x: width - INSET - 18.0, y: middle }, ICON, colours.text);
            }
        }
    }
}

/// What a menu is drawn in, light or dark, as Windows 11's are.
struct Colours {
    sheet: Color,
    stroke: Color,
    shadow: Color,
    text: Color,
    icon: Color,
    highlight: Color,
    rule: Color,
}

impl Colours {
    fn new(dark: bool) -> Self {
        if dark {
            Colours {
                sheet: Color::hex(0x2C2C2C, 0.98),
                stroke: Color::hex(0x000000, 0.2),
                shadow: Color::hex(0x000000, 0.35),
                text: Color::hex(0xFFFFFF, 1.0),
                icon: Color::hex(0xFFFFFF, 0.86),
                highlight: Color::hex(0xFFFFFF, 0.06),
                rule: Color::hex(0xFFFFFF, 0.08),
            }
        } else {
            Colours {
                sheet: Color::hex(0xF9F9F9, 0.98),
                stroke: Color::hex(0x000000, 0.08),
                shadow: Color::hex(0x000000, 0.16),
                text: Color::hex(0x000000, 0.9),
                icon: Color::hex(0x000000, 0.72),
                highlight: Color::hex(0x000000, 0.04),
                rule: Color::hex(0x000000, 0.08),
            }
        }
    }
}
