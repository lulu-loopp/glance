//! The card beside the AI component's dock: the lane of the source whose ring the pointer rests on, all it tells (its
//! limit's windows, what it spent today by model and by kind, its sessions), on a plate of the dock's skin beside the
//! dock, its middle by that ring's, on its screen. A window of its own, one for this thread, that every click passes
//! through: it goes as the pointer leaves the ring, or as the window the dock is drawn in is hidden or destroyed.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_numerics::Matrix3x2;

use super::gfx::{self, Gfx, Surface};
use super::life::{Lives, Showing};
use super::prefs::Prefs;
use super::text::Lang;
use super::theme::Theme;
use super::view::{self, ai::Source, Layout, Pass, Scene};
use super::window::Window;

/// Which side of the dock the card is on: away from the screen's edge the dock is at.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Side {
    Left,
    Right,
    Below,
    Above,
}

struct Card {
    window: Window,
    /// The device it draws with, and its surface on it.
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    /// Counted as shown on its device while it is (see `life`).
    showing: Option<Showing>,
    /// What it shows where, as last drawn (none: away), not drawn again for the same.
    shown: Option<u64>,
}

impl Lives for Card {
    fn device(&self) -> &Rc<Gfx> {
        &self.gfx
    }

    fn showing(&mut self) -> &mut Option<Showing> {
        &mut self.showing
    }

    fn let_go_of_shown(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
    }

    fn let_go_of_kept(&mut self) {
        self.surface = None;
    }
}

thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    /// The window the dock it is beside is drawn in: hidden or gone, the card goes with it (see `went`). Kept apart from
    /// the card, which is borrowed as its own window hides.
    static OWNER: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// How far from the dock it shows, and the room round its plate for its shadow (DIPs).
const GAP: f32 = 8.0;
const MARGIN: f32 = 16.0;

/// What the card shows of: `source`, in `theme`, as `prefs` show the AI component (its modules held: see
/// `Prefs::holding`), in `lang`.
pub struct Of<'a> {
    pub source: Source,
    pub theme: &'a Theme,
    pub prefs: &'a Prefs,
    pub lang: Lang,
}

/// Shows `of`'s card beside `by` (the dock, on the screen) on `side`, its middle by `about`'s (the source's ring), all
/// of it on that screen, till `hide` or till `owner` (the window the dock is drawn in) is hidden or destroyed. Shown
/// again as it is, nothing is drawn.
pub fn show(owner: HWND, of: Of, by: RECT, about: RECT, side: Side) {
    let controller = &crate::app().controller;
    let (info, history) = (controller.info(), controller.history.samples());
    // (Nothing read of the machine yet: the lanes are drawn from its last sample.)
    if history.is_empty() {
        return;
    }
    let seen = Default::default();
    let scene = Scene {
        info: &info,
        prefs: of.prefs,
        theme: of.theme,
        lang: of.lang,
        history: &history,
        seen: &seen,
        pen_ms: 0.0,
        scrolls: view::Scrolls::default(),
        tab: view::Tab::default(),
        hover: None,
        hint: None,
        thumbs: &view::RESTING,
        pinned: false,
        overlay: false,
        buttons: false,
        newer: false,
        settings_lit: None,
        music_apart: false,
    };
    let Some(lane) = view::ai::card(&scene, of.source) else { return hide() };
    let layout = Layout::with_columns(vec![lane.height(of.theme)], 1, of.theme);
    let plate = (layout.width(), layout.height());
    let at = POINT { x: (about.left + about.right) / 2, y: (about.top + about.bottom) / 2 };
    // Its screen, and how many pixels a DIP is there.
    let monitor = unsafe { MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (mut dpi, mut unused) = (96, 96);
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
    }
    let (scale, work) = (dpi as f32 / 96.0, info.rcWork);
    let px = |dips: f32| (dips * scale).round() as i32;
    let (plate_w, plate_h, gap, margin) = (px(plate.0), px(plate.1), px(GAP), px(MARGIN));
    // Beside the dock; along it, its middle by the ring's, all of it in the work area.
    let along = |middle: i32, length: i32, low: i32, high: i32| (middle - length / 2).min(high - gap - length).max(low + gap);
    let (left, top) = match side {
        Side::Left => (by.left - gap - plate_w, along(at.y, plate_h, work.top, work.bottom)),
        Side::Right => (by.right + gap, along(at.y, plate_h, work.top, work.bottom)),
        Side::Below => (along(at.x, plate_w, work.left, work.right), by.bottom + gap),
        Side::Above => (along(at.x, plate_w, work.left, work.right), by.top - gap - plate_h),
    };
    let rect = RECT { left: left - margin, top: top - margin, right: left + plate_w + margin, bottom: top + plate_h + margin };
    let mut key = std::hash::DefaultHasher::new();
    (&lane, of.theme.skin, of.theme.dark, rect.left, rect.top, rect.right, rect.bottom).hash(&mut key);
    let key = key.finish();
    // (On the device for the screen it shows on: see `gfx::on_screen`.)
    let Ok(gfx) = gfx::for_area(rect) else { return };
    CARD.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            let Ok(mut window) = Window::new() else { return };
            window.set_click_through(true);
            *kept = Some(Card { window, gfx: gfx.clone(), surface: None, showing: None, shown: None });
        }
        let card = kept.as_mut().unwrap();
        OWNER.set(owner);
        // (What it drew with on another device goes with it: see `life`.)
        if !Rc::ptr_eq(&card.gfx, &gfx) {
            card.gone();
            card.gfx = gfx.clone();
        }
        if card.shown == Some(key) {
            return;
        }
        let surface = match card.surface.take() {
            Some(surface) => Ok(surface),
            None => Surface::new(&gfx, card.window.hwnd),
        };
        let Ok(mut surface) = surface else { return };
        card.shown();
        let size = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
        let look = super::menu::Look::of(of.theme.skin, of.theme.dark);
        let mut sheet = Ok(());
        let drawn = surface.draw(&gfx, size, scale, |frame| {
            sheet = super::menu::sheet(frame, look, (MARGIN, MARGIN, plate.0, plate.1), false).map(|_| ());
            frame.place(Matrix3x2::translation(MARGIN, MARGIN));
            view::paint(frame, &scene, std::slice::from_ref(&lane), &layout, Pass::Content);
            frame.origin(0.0, 0.0);
        });
        let drawn = drawn.and(sheet);
        // (The device gone, its surface is made anew on the next.)
        if let Err(error) = &drawn {
            if !gfx::failed("ai card", &card.gfx, Some(error)) {
                card.surface = Some(surface);
            }
            return;
        }
        card.surface = Some(surface);
        card.window.place(rect);
        card.window.show();
        card.shown = Some(key);
    });
}

/// A window of this thread hidden or destroyed (told by `Window`): the one the dock is drawn in, the card goes with it.
pub fn went(hwnd: HWND) {
    if OWNER.get() == hwnd {
        hide();
    }
}

/// Puts it over the windows again, while it shows: the dock puts itself above all each time it is drawn.
pub fn above() {
    CARD.with(|kept| {
        if let Some(card) = kept.borrow().as_ref().filter(|card| card.shown.is_some()) {
            card.window.show();
        }
    });
}

/// The screens changed: a card not shown, on a device no screen would be drawn for any longer, let go of with all it
/// draws with (see `tip::screens_changed`).
pub fn screens_changed() {
    let gone = CARD.with(|kept| kept.borrow_mut().take_if(|card| card.shown.is_none() && !gfx::needed(&card.gfx)));
    drop(gone);
}

/// Shows nothing more.
pub fn hide() {
    CARD.with(|kept| {
        if let Some(card) = kept.borrow_mut().as_mut().filter(|card| card.shown.is_some()) {
            card.window.hide();
            card.shown = None;
            card.hidden();
        }
    });
}
