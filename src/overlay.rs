//! The overlay's window: never taking the focus, and while it is not being
//! placed, not a click either (a click goes to whatever is beneath). Drawn
//! by the panel's thread at every sample. While the settings window is open
//! it is being placed: shown even with nothing to show yet, set off in
//! colour, and dragged where it is to stay.

use std::rc::Rc;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, LoadCursorW, SetCursor, IDC_SIZEALL};

use crate::reading::Sample;
use crate::settings::{OverlaySettings, OverlayWhen};
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::overlay;
use crate::ui::text::Lang;
use crate::ui::window::Window;

/// How far it keeps from its screen's edges (DIPs).
const INSET: f32 = 16.0;

pub struct Overlay {
    gfx: Rc<Gfx>,
    pub window: Window,
    /// None after the device was lost, until it is made again on the new one.
    surface: Option<Surface>,
    shown: bool,
    /// Where it is on screen (physical pixels).
    rect: RECT,
    /// Being dragged: the pointer's offset from its corner.
    grab: Option<POINT>,
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

impl Overlay {
    pub fn new() -> windows::core::Result<Self> {
        let gfx = gfx::current()?;
        let mut window = Window::new()?;
        window.set_click_through(true);
        let surface = Some(Surface::new(&gfx, window.hwnd)?);
        Ok(Overlay { gfx, window, surface, shown: false, rect: RECT::default(), grab: None })
    }

    /// Shows the readings of `sample` as `settings` ask: in game mode
    /// (`game_mode`), with the frames of the program presenting them, on its
    /// screen; or always, on the screen it was put on; `placing` (the
    /// settings are open), always, to be dragged. Hidden while off, and
    /// while there is nothing to show.
    pub fn show(&mut self, sample: Option<&Sample>, settings: &OverlaySettings, lang: Lang, game_mode: bool, placing: bool) {
        let game = sample.and_then(|s| s.game.as_ref()).filter(|_| game_mode);
        let wanted = settings.on && (placing || game_mode || settings.when == OverlayWhen::Always);
        let lines = sample.filter(|_| wanted).map(|s| overlay::lines(s, game, &settings.items, lang, placing)).unwrap_or_default();
        if lines.is_empty() {
            return self.hide();
        }
        if !self.follow_device() {
            return;
        }
        self.window.set_click_through(!placing);
        // In game mode, the game's screen; otherwise the one it was put on.
        let point = match game.and_then(|game| game.screen) {
            Some([left, top, right, bottom]) => Some(((left + right) / 2, (top + bottom) / 2)),
            None => settings.screen,
        };
        let monitor = match point {
            Some((x, y)) => unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) },
            None => unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) },
        };
        let (screen, scale) = monitor_info(monitor);
        let (width, height) = overlay::size(&lines, |text, font| self.gfx.measure(text, font));
        let size = ((width * scale).ceil() as i32, (height * scale).ceil() as i32);
        // Held where it is while it is dragged.
        if self.grab.is_none() {
            let (left, top, across, down) = room(screen, size, scale);
            let x = left + (settings.at.0.clamp(0.0, 1.0) * across as f32).round() as i32;
            let y = top + (settings.at.1.clamp(0.0, 1.0) * down as f32).round() as i32;
            self.rect = RECT { left: x, top: y, right: x + size.0, bottom: y + size.1 };
        } else {
            self.rect.right = self.rect.left + size.0;
            self.rect.bottom = self.rect.top + size.1;
        }
        // Placed above all again each time: a game may have taken the top.
        self.window.place(self.rect);
        let surface = self.surface.as_mut().unwrap();
        if surface.draw(&self.gfx, (size.0 as u32, size.1 as u32), scale, |frame| overlay::paint(frame, &lines, placing)).is_err() {
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

    /// The button went down on it: it follows the pointer until it is let go.
    pub fn press(&mut self) {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            return;
        }
        self.grab = Some(POINT { x: cursor.x - self.rect.left, y: cursor.y - self.rect.top });
        unsafe { SetCapture(self.window.hwnd) };
    }

    /// The pointer moved over it, or while it is dragged.
    pub fn moved(&mut self) {
        unsafe { SetCursor(LoadCursorW(None, IDC_SIZEALL).ok()) };
        let (Some(grab), mut cursor) = (self.grab, POINT::default()) else { return };
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            return;
        }
        let (width, height) = (self.rect.right - self.rect.left, self.rect.bottom - self.rect.top);
        let (x, y) = (cursor.x - grab.x, cursor.y - grab.y);
        self.rect = RECT { left: x, top: y, right: x + width, bottom: y + height };
        self.window.place(self.rect);
    }

    /// Let go: where it now is on the screen it is over, as kept (see
    /// `OverlaySettings::at`), and a point on that screen; none unless it
    /// was being dragged.
    pub fn release(&mut self) -> Option<((f32, f32), (i32, i32))> {
        self.grab.take()?;
        unsafe {
            let _ = ReleaseCapture();
        }
        let rect = self.rect;
        let centre = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        let (screen, scale) = monitor_info(unsafe { MonitorFromPoint(centre, MONITOR_DEFAULTTONEAREST) });
        let (left, top, across, down) = room(screen, (rect.right - rect.left, rect.bottom - rect.top), scale);
        let share = |at: i32, room: i32| if room > 0 { (at as f32 / room as f32).clamp(0.0, 1.0) } else { 0.0 };
        Some(((share(rect.left - left, across), share(rect.top - top, down)), (centre.x, centre.y)))
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
