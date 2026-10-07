//! The overlay's window: over the game, in a corner of its screen, never
//! taking the focus or a click (a click goes to the game beneath). Drawn
//! by the panel's thread at every sample, while it is on and a game runs.

use std::rc::Rc;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::reading::Sample;
use crate::settings::{Corner, OverlaySettings};
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::overlay;
use crate::ui::text::Lang;
use crate::ui::window::Window;

/// How far it sits from its screen's corner (DIPs).
const INSET: f32 = 16.0;

pub struct Overlay {
    gfx: Rc<Gfx>,
    pub window: Window,
    /// None after the device was lost, until it is made again on the new one.
    surface: Option<Surface>,
    shown: bool,
}

impl Overlay {
    pub fn new() -> windows::core::Result<Self> {
        let gfx = gfx::current()?;
        let mut window = Window::new()?;
        window.set_click_through(true);
        let surface = Some(Surface::new(&gfx, window.hwnd)?);
        Ok(Overlay { gfx, window, surface, shown: false })
    }

    /// Shows the readings of `sample`'s game as `settings` ask, on the
    /// game's screen; hides the overlay while it is off or no game runs.
    pub fn show(&mut self, sample: Option<&Sample>, settings: &OverlaySettings, lang: Lang) {
        let game = sample.and_then(|s| s.game.as_ref()).filter(|game| game.is_game);
        let lines = sample.filter(|_| game.is_some()).map(|s| overlay::lines(s, settings.detail, lang)).unwrap_or_default();
        let screen = game.and_then(|game| game.screen);
        let Some([left, top, right, bottom]) = screen.filter(|_| settings.on && !lines.is_empty()) else {
            return self.hide();
        };
        if !self.follow_device() {
            return;
        }
        // Drawn at its screen's scale.
        let monitor = unsafe { MonitorFromPoint(POINT { x: (left + right) / 2, y: (top + bottom) / 2 }, MONITOR_DEFAULTTONEAREST) };
        let (mut dpi, mut unused) = (96u32, 96u32);
        let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
        let scale = dpi as f32 / 96.0;
        let (width, height) = overlay::size(&lines, |text, font| self.gfx.measure(text, font));
        let (width, height) = ((width * scale).ceil() as i32, (height * scale).ceil() as i32);
        let inset = (INSET * scale).round() as i32;
        let (x, y) = match settings.corner {
            Corner::TopLeft => (left + inset, top + inset),
            Corner::TopRight => (right - inset - width, top + inset),
            Corner::BottomLeft => (left + inset, bottom - inset - height),
            Corner::BottomRight => (right - inset - width, bottom - inset - height),
        };
        // Placed above all again each time: a game may have taken the top.
        self.window.place(RECT { left: x, top: y, right: x + width, bottom: y + height });
        let surface = self.surface.as_mut().unwrap();
        if surface.draw(&self.gfx, (width as u32, height as u32), scale, |frame| overlay::paint(frame, &lines)).is_err() {
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

    fn hide(&mut self) {
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
