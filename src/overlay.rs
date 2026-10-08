//! The overlay's window: never taking the focus. Unlocked, it is dragged
//! where it is to stay; locked, it stays. Either way a right click on it
//! offers the lock, turning it off and the settings (so it takes the clicks that
//! land on it: a game holding the pointer is not one of them). Drawn by the
//! panel's thread at every sample.

use std::rc::Rc;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetForegroundWindow, LoadCursorW, PostMessageW, SetCursor, SetForegroundWindow,
    TrackPopupMenuEx, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, MF_CHECKED, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};

use crate::reading::Sample;
use crate::settings::OverlaySettings;
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::overlay::{self, INSET};
use crate::ui::text::Lang;
use crate::ui::window::Window;


/// What its right-click menu offers.
pub enum Choice {
    Lock(bool),
    Close,
    Settings,
}

pub struct Overlay {
    gfx: Rc<Gfx>,
    pub window: Window,
    /// None after the device was lost, until it is made again on the new one.
    surface: Option<Surface>,
    shown: bool,
    locked: bool,
    lang: Lang,
    /// Where it is on screen (physical pixels).
    rect: RECT,
    /// Its screen's scale, and the size it is drawn at (1 as designed).
    dpi: f32,
    size: f32,
    /// What it shows, and how opaque: drawn again as it is dragged.
    drawn: (Vec<overlay::Line>, f32),
    /// Being dragged by the pointer: to move it, or by its edges, to size it.
    grab: Option<Grab>,
    /// The screen of the game played, kept while it presents no frames
    /// for a moment.
    game_screen: Option<[i32; 4]>,
}

/// A drag under way: where the pointer, the overlay and its size were as it
/// began, and which edges it holds (none: the whole overlay, to move it).
#[derive(Clone, Copy)]
struct Grab {
    from: POINT,
    rect: RECT,
    size: f32,
    edges: Edges,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn any(self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// How near its edge the pointer sizes it rather than moves it (DIPs).
const EDGE: f32 = 6.0;

/// Where it is moved to, and how large it is, as a drag left it.
pub struct Placed {
    /// See `OverlaySettings::at`.
    pub at: (f32, f32),
    /// A point on the screen it is over.
    pub point: (i32, i32),
    pub size: f32,
}

/// A monitor's whole area, and its scale.
fn monitor_info(monitor: HMONITOR) -> (RECT, f32) {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
    let (mut dpi, mut unused) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
    (info.rcMonitor, dpi as f32 / 96.0)
}

/// The room a plate `size` large has on `screen`, inside the inset: its
/// left, top, and how far it can move across and down.
fn room(screen: RECT, size: (i32, i32), scale: f32) -> (i32, i32, i32, i32) {
    let inset = (INSET * scale).round() as i32;
    let across = (screen.right - screen.left - 2 * inset - size.0).max(0);
    let down = (screen.bottom - screen.top - 2 * inset - size.1).max(0);
    (screen.left + inset, screen.top + inset, across, down)
}

fn cursor() -> Option<POINT> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.ok().map(|_| cursor)
}

impl Overlay {
    pub fn new() -> windows::core::Result<Self> {
        let gfx = gfx::current()?;
        let window = Window::new()?;
        let surface = Some(Surface::new(&gfx, window.hwnd)?);
        Ok(Overlay {
            gfx,
            window,
            surface,
            shown: false,
            locked: false,
            lang: Lang::En,
            rect: RECT::default(),
            dpi: 1.0,
            size: 1.0,
            drawn: (Vec::new(), 1.0),
            grab: None,
            game_screen: None,
        })
    }

    /// Shows the readings of `sample` as `settings` ask, while `wanted`:
    /// while a game is played (`playing`), with its frames, on its screen;
    /// otherwise on the screen it was put on. Hidden while not wanted, and
    /// while there is nothing to show.
    pub fn show(&mut self, sample: Option<&Sample>, settings: &OverlaySettings, lang: Lang, playing: bool, wanted: bool) {
        self.locked = settings.locked;
        self.lang = lang;
        let game = sample.and_then(|s| s.game.as_ref()).filter(|_| playing);
        let lines = sample.filter(|_| wanted).map(|s| overlay::lines(s, game, playing, &settings.items, lang)).unwrap_or_default();
        if lines.is_empty() {
            return self.hide();
        }
        self.drawn = (lines, settings.opacity);
        // Held as the pointer has it while it is dragged.
        if self.grab.is_some() {
            let (width, height) = self.dims();
            self.rect.right = self.rect.left + width;
            self.rect.bottom = self.rect.top + height;
            return self.draw();
        }
        // Over a game, the game's screen; otherwise the one it was put on.
        if let Some(screen) = game.and_then(|game| game.screen) {
            self.game_screen = Some(screen);
        }
        if !playing {
            self.game_screen = None;
        }
        let point = match self.game_screen {
            Some([left, top, right, bottom]) => Some(((left + right) / 2, (top + bottom) / 2)),
            None => settings.screen,
        };
        let monitor = match point {
            Some((x, y)) => unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) },
            None => unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) },
        };
        let screen;
        (screen, self.dpi) = monitor_info(monitor);
        self.size = settings.size.clamp(crate::panel::SIZES.0, crate::panel::SIZES.1);
        let size = self.dims();
        let (left, top, across, down) = room(screen, size, self.dpi);
        let x = left + (settings.at.0.clamp(0.0, 1.0) * across as f32).round() as i32;
        let y = top + (settings.at.1.clamp(0.0, 1.0) * down as f32).round() as i32;
        self.rect = RECT { left: x, top: y, right: x + size.0, bottom: y + size.1 };
        self.draw();
    }

    /// Its width and height on screen, at its size (physical pixels).
    fn dims(&self) -> (i32, i32) {
        let scale = self.dpi * self.size;
        let (width, height) = overlay::size(&self.drawn.0, |text, font| self.gfx.measure(text, font));
        ((width * scale).ceil() as i32, (height * scale).ceil() as i32)
    }

    /// Draws what it shows where it is, set off in colour while it is dragged.
    fn draw(&mut self) {
        if !self.follow_device() {
            return;
        }
        // Placed above all again each time: a game may have taken the top.
        self.window.place(self.rect);
        let (lines, opacity, scale) = (&self.drawn.0, self.drawn.1, self.dpi * self.size);
        let size = ((self.rect.right - self.rect.left) as u32, (self.rect.bottom - self.rect.top) as u32);
        let dragged = self.grab.is_some();
        let surface = self.surface.as_mut().unwrap();
        let painted = surface.draw(&self.gfx, size, scale, |frame| {
            frame.crisp_text();
            overlay::paint(frame, lines, opacity, dragged, scale);
        });
        if painted.is_err() {
            // Made again on a new device at the next sample.
            gfx::lost();
            return;
        }
        self.gfx.sweep();
        if !self.shown {
            self.window.show();
            self.shown = true;
        }
    }

    /// The edges of it the pointer at `at` is on, if it is near them.
    fn edges_at(&self, at: POINT) -> Edges {
        let near = (EDGE * self.dpi).round() as i32;
        let r = self.rect;
        Edges { left: at.x < r.left + near, right: at.x >= r.right - near, top: at.y < r.top + near, bottom: at.y >= r.bottom - near }
    }

    /// The pointer's shape over edges `edges` (none: the whole overlay).
    fn point_at(edges: Edges) {
        let shape = match (edges.left || edges.right, edges.top || edges.bottom) {
            (true, true) if edges.left == edges.top => IDC_SIZENWSE,
            (true, true) => IDC_SIZENESW,
            (true, false) => IDC_SIZEWE,
            (false, true) => IDC_SIZENS,
            (false, false) => IDC_SIZEALL,
        };
        unsafe { SetCursor(LoadCursorW(None, shape).ok()) };
    }

    /// The left button went down on it: unless it is locked, it is moved, or
    /// if the pointer is on its edges, sized, until it is let go.
    pub fn press(&mut self) {
        let Some(from) = cursor().filter(|_| !self.locked) else { return };
        let edges = self.edges_at(from);
        self.grab = Some(Grab { from, rect: self.rect, size: self.size, edges });
        unsafe { SetCapture(self.window.hwnd) };
        self.draw();
    }

    /// The pointer moved over it, or while it is dragged: it moves, or grows
    /// and shrinks as a whole from the corner opposite the edges held (its
    /// shape is its readings').
    pub fn moved(&mut self) {
        let Some(at) = cursor() else { return };
        let Some(grab) = self.grab else {
            if !self.locked {
                Self::point_at(self.edges_at(at));
            }
            return;
        };
        Self::point_at(grab.edges);
        let (dx, dy) = (at.x - grab.from.x, at.y - grab.from.y);
        let r = grab.rect;
        if !grab.edges.any() {
            self.rect = RECT { left: r.left + dx, top: r.top + dy, right: r.right + dx, bottom: r.bottom + dy };
            self.window.place(self.rect);
            return;
        }
        let (width, height) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let e = grab.edges;
        let across = (e.left || e.right).then(|| (width + if e.right { dx } else { -dx } as f32) / width);
        let down = (e.top || e.bottom).then(|| (height + if e.bottom { dy } else { -dy } as f32) / height);
        let ratio = match (across, down) {
            (Some(a), Some(d)) => a.max(d),
            (Some(a), None) => a,
            (None, Some(d)) => d,
            (None, None) => 1.0,
        };
        self.size = (grab.size * ratio).clamp(crate::panel::SIZES.0, crate::panel::SIZES.1);
        let (width, height) = self.dims();
        let x = if e.left { r.right - width } else { r.left };
        let y = if e.top { r.bottom - height } else { r.top };
        self.rect = RECT { left: x, top: y, right: x + width, bottom: y + height };
        self.draw();
    }

    /// Let go: where it now is and how large, as kept; none unless it was
    /// being dragged.
    pub fn release(&mut self) -> Option<Placed> {
        self.grab.take()?;
        unsafe {
            let _ = ReleaseCapture();
        }
        self.draw();
        let rect = self.rect;
        let centre = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        let (screen, scale) = monitor_info(unsafe { MonitorFromPoint(centre, MONITOR_DEFAULTTONEAREST) });
        let (left, top, across, down) = room(screen, (rect.right - rect.left, rect.bottom - rect.top), scale);
        let share = |at: i32, room: i32| if room > 0 { (at as f32 / room as f32).clamp(0.0, 1.0) } else { 0.0 };
        Some(Placed { at: (share(rect.left - left, across), share(rect.top - top, down)), point: (centre.x, centre.y), size: self.size })
    }

    /// A right click on it: its menu, at the pointer, and what was chosen.
    /// The menu has to be in front to close on a click elsewhere: the window
    /// that was in front before (a game, to be given the front back) is
    /// returned too.
    pub fn menu(&self) -> (Option<Choice>, HWND) {
        let before = unsafe { GetForegroundWindow() };
        (self.choose(), before)
    }

    fn choose(&self) -> Option<Choice> {
        let zh = self.lang == Lang::Zh;
        const LOCK: usize = 1;
        const CLOSE: usize = 2;
        const SETTINGS: usize = 3;
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.ok()?;
        let chosen = unsafe {
            let menu = CreatePopupMenu().ok()?;
            let lock = HSTRING::from(if zh { "锁定位置" } else { "Lock position" });
            let _ = AppendMenuW(menu, if self.locked { MF_STRING | MF_CHECKED } else { MF_STRING }, LOCK, &lock);
            let _ = AppendMenuW(menu, MF_STRING, CLOSE, &HSTRING::from(if zh { "关闭悬浮窗" } else { "Close the overlay" }));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, SETTINGS, &HSTRING::from(if zh { "设置…" } else { "Settings…" }));
            let hwnd = self.window.hwnd;
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, cursor.x, cursor.y, hwnd, None).0 as usize;
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            chosen
        };
        match chosen {
            LOCK => Some(Choice::Lock(!self.locked)),
            CLOSE => Some(Choice::Close),
            SETTINGS => Some(Choice::Settings),
            _ => None,
        }
    }

    fn hide(&mut self) {
        if self.grab.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        if !self.shown {
            return;
        }
        self.shown = false;
        self.window.hide();
        // The drawing memory is given back while it is away.
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
    }

    /// Takes up this thread's current graphics device if it is a new one,
    /// making the surface again on it. False while there is none.
    fn follow_device(&mut self) -> bool {
        let Ok(current) = gfx::current() else { return false };
        if Rc::ptr_eq(&current, &self.gfx) && self.surface.is_some() {
            return true;
        }
        self.surface = None;
        let Ok(surface) = Surface::new(&current, self.window.hwnd) else {
            gfx::lost();
            return false;
        };
        self.surface = Some(surface);
        self.gfx = current;
        true
    }
}
