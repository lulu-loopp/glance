//! The card beside the AI component's dock: the lane of the source whose ring the pointer rests on, all it tells (its
//! limit's windows, what it spent today by model and by kind, its sessions), on a sheet of the dock's skin as a menu
//! opened on the dock is (glass's a piece of the glass, frosted as the dock is), beside the dock, its middle by that
//! ring's, on its screen. A window of its own, one for this thread, that takes the pointer on its sheet alone: it stays
//! while the pointer is on the ring, on the card or on its way between them (see `holds`), and goes as the window the
//! dock is drawn in is hidden or destroyed.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use super::gfx::{self, Gfx};
use super::life::{Lives, Showing};
use super::menu::{Drawn, Look};
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
    /// The device it draws with, and what it draws with there, made for a look (see `menu::Drawn`).
    gfx: Rc<Gfx>,
    surface: Option<(Drawn, Look)>,
    /// What is written on it, kept while it shows the same (see `menu::Drawn::draw`).
    words: gfx::Layer,
    /// Where the pointer keeps it (see `holds`), on the screen, while it shows.
    holds: Option<RECT>,
    /// Glass's: the windows behind it put together (see `scene`), when it last asked for them, and how light what is
    /// behind its sheet is and how busy (its luminance's mean and spread), where it is now; none till that is read.
    behind: Option<super::scene::Scene>,
    asked: Option<Instant>,
    tone: Option<(RECT, (f32, f32))>,
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
        match &mut self.surface {
            Some((Drawn::Plain(surface), _)) => surface.release(&self.gfx),
            Some((Drawn::Frosted(frosted), _)) => frosted.release(),
            None => {}
        }
    }

    fn let_go_of_kept(&mut self) {
        self.surface = None;
        self.behind = None;
    }
}

thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    /// The window the dock it is beside is drawn in: hidden or gone, the card goes with it (see `went`). Kept apart from
    /// the card, which is borrowed as its own window hides.
    static OWNER: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// How far from the dock it shows (DIPs).
const GAP: f32 = 8.0;

/// How long what is behind it stands before it is taken again, while it shows.
const LOOK_AGAIN: Duration = Duration::from_secs(2);

impl Card {
    /// How frosted glass is drawn over what is behind `sheet` (physical px, `px` to a DIP), `dark` or light: as the
    /// panel's glass is over what is behind it (see `skins::frost`), not as the dock's is (what is behind the dock may
    /// be calm, and what is behind the card busy), in twentieths (not drawn again for less). As frosted as it may be
    /// till that is read. What is behind it is taken again as it moves, and while it stays every `LOOK_AGAIN`.
    fn frost(&mut self, sheet: RECT, px: f32, dark: bool) -> f32 {
        if self.behind.is_none() {
            // (Read on another thread: what draws the dock draws again, and the card with it.)
            let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| crate::app().controller.wake());
            self.behind = super::scene::Scene::new(sheet, wake, &self.gfx);
        }
        if let Some(scene) = &mut self.behind {
            let arranged = scene.arrange(sheet, self.window.hwnd);
            if arranged || self.asked.is_none_or(|at| at.elapsed() >= LOOK_AGAIN) {
                scene.want();
                self.asked = Some(Instant::now());
            }
            if let Some(capture) = scene.ready() {
                self.tone = Some((sheet, capture.luminance(sheet, px)));
            }
        }
        frost_over(self.tone, sheet, dark)
    }
}

/// How frosted glass is drawn over `sheet`, `dark` or light, what is behind it read as `tone` (where it was read, its
/// luminance's mean and spread): as that asks, in twentieths; as frosted as it may be where it was not read there.
fn frost_over(tone: Option<(RECT, (f32, f32))>, sheet: RECT, dark: bool) -> f32 {
    match tone {
        Some((at, (mean, spread))) if at == sheet => (super::skins::frost(mean, spread, dark) * 20.0).round() / 20.0,
        _ => 1.0,
    }
}

/// What the card shows of: `source`, in `theme`, as `prefs` show the AI component (its modules held: see
/// `Prefs::holding`), in `lang`, on a sheet that looks as a menu opened on the dock does (`look`).
pub struct Of<'a> {
    pub source: Source,
    pub theme: &'a Theme,
    pub prefs: &'a Prefs,
    pub lang: Lang,
    pub look: Look,
}

/// Shows `of`'s card beside `by` (the dock, on the screen) on `side`, its middle by `about`'s (the source's ring), all
/// of it on that screen, till `hide` or till `owner` (the window the dock is drawn in) is hidden or destroyed. Shown
/// again as it is, nothing is drawn.
pub fn show(owner: HWND, of: Of, by: RECT, about: RECT, side: Side) {
    let _timed = crate::perf::Timed::start("ai card", || format!("{:?}", of.source));
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
    let (plate, (shift_x, shift_y)) = sheet_of(of.theme, layout.lanes()[0]);
    let at = POINT { x: (about.left + about.right) / 2, y: (about.top + about.bottom) / 2 };
    // Its screen, and how many pixels a DIP is there.
    let monitor = unsafe { MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (mut dpi, mut unused) = (96, 96);
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
    }
    // (As large as the dock is drawn on that screen.)
    let (scale, work) = (of.look.px(dpi as f32 / 96.0), info.rcWork);
    let px = |dips: f32| (dips * scale).round() as i32;
    let (plate_w, plate_h, gap) = (px(plate.0), px(plate.1), px(GAP));
    // Beside the dock; along it, its middle by the ring's, all of it in the work area.
    let along = |middle: i32, length: i32, low: i32, high: i32| (middle - length / 2).min(high - gap - length).max(low + gap);
    let (left, top) = match side {
        Side::Left => (by.left - gap - plate_w, along(at.y, plate_h, work.top, work.bottom)),
        Side::Right => (by.right + gap, along(at.y, plate_h, work.top, work.bottom)),
        Side::Below => (along(at.x, plate_w, work.left, work.right), by.bottom + gap),
        Side::Above => (along(at.x, plate_w, work.left, work.right), by.top - gap - plate_h),
    };
    let sheet = RECT { left, top, right: left + plate_w, bottom: top + plate_h };
    let holds = held_by(about, sheet);
    // (On the device for the screen it shows on: see `gfx::on_screen`.)
    let Ok(gfx) = gfx::for_area(sheet) else { return };
    CARD.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            let Ok(window) = Window::new() else { return };
            *kept = Some(Card { window, gfx: gfx.clone(), surface: None, words: gfx::Layer::default(), holds: None, behind: None, asked: None, tone: None, showing: None, shown: None });
        }
        let card = kept.as_mut().unwrap();
        OWNER.set(owner);
        // (What it drew with on another device goes with it: see `life`.)
        if !Rc::ptr_eq(&card.gfx, &gfx) {
            card.gone();
            card.gfx = gfx.clone();
        }
        card.holds = Some(holds);
        // (The room round the sheet for its shadows is over the dock: the pointer there is the dock's.)
        card.window.take_pointer_within(Some(sheet));
        // Glass frosted as what is behind the card asks (see `Card::frost`).
        let look = match of.theme.skin {
            super::theme::Skin::Glass => of.look.frosted(card.frost(sheet, scale, of.theme.dark), true),
            _ => of.look,
        };
        let mut key = std::hash::DefaultHasher::new();
        (&lane, of.theme.skin, of.theme.dark, look.frost.to_bits(), look.zoom.to_bits(), left, top, plate_w, plate_h).hash(&mut key);
        let key = key.finish();
        if card.shown == Some(key) {
            return;
        }
        // (Made anew for another look: the dock's skin, its light, or how large it is drawn changed. Frosted otherwise
        // as it is now.)
        let made = match card.surface.take().filter(|(_, made)| (made.skin, made.dark, made.zoom) == (look.skin, look.dark, look.zoom)) {
            Some((mut drawn, _)) => {
                drawn.frost_as(look);
                Ok((drawn, look))
            }
            None => Drawn::new(&gfx, card.window.hwnd, look).map(|drawn| (drawn, look)),
        };
        let Ok((mut surface, look)) = made else { return };
        card.shown();
        // The sheet with the room round it for its shadows.
        let room = surface.room(look);
        let margin = px(room);
        let rect = RECT { left: left - margin, top: top - margin, right: left + plate_w + margin, bottom: top + plate_h + margin };
        let size = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
        let drawn = surface.draw(&gfx, size, scale, look, room, plate, round(of.theme, plate), &mut card.words, key, |_, _| {}, |frame, _| {
            frame.within(windows_numerics::Matrix3x2::translation(shift_x, shift_y), |frame| {
                view::paint(frame, &scene, std::slice::from_ref(&lane), &layout, Pass::Content);
            });
        });
        // (The device gone, its surface is made anew on the next.)
        if let Err(error) = &drawn {
            if !gfx::failed("ai card", &card.gfx, Some(error)) {
                card.surface = Some((surface, look));
            }
            return;
        }
        card.surface = Some((surface, look));
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

/// Where the pointer keeps the card at `card` (on the screen) up, beside the ring `about`: on the ring, on the card, and
/// on its way between them.
fn held_by(about: RECT, card: RECT) -> RECT {
    RECT { left: about.left.min(card.left), top: about.top.min(card.top), right: about.right.max(card.right), bottom: about.bottom.max(card.bottom) }
}

/// Its sheet in `theme` for a lane laid out at `lane` (DIPs), and how far what is laid out is moved on it: what the lane
/// shows as far in from every edge as the sheet's corners are round (their arcs' centres), the same all round, and no
/// less than a sheet's least (see `look`).
fn sheet_of(theme: &Theme, lane: view::Rect) -> ((f32, f32), (f32, f32)) {
    let inset = theme.radius.max(super::look::INSET_LEAST);
    let plate = (lane.w - 2.0 * theme.pad_x + 2.0 * inset, lane.h - theme.pad_top - theme.pad_bottom + 2.0 * inset);
    (plate, (inset - theme.pad_x - lane.x, inset - theme.pad_top - lane.y))
}

/// How round its sheet `plate` large (DIPs) is in `theme`: as the dock's pieces are, and the panel's, for it shows what
/// they do (not as a menu's, which are rounder in no skin).
fn round(theme: &Theme, plate: (f32, f32)) -> f32 {
    theme.radius.min(plate.0.min(plate.1) / 2.0)
}

fn within(at: POINT, area: RECT) -> bool {
    at.x >= area.left && at.x < area.right && at.y >= area.top && at.y < area.bottom
}

/// Whether it shows, and the pointer at `at` (on the screen) keeps it: on the ring it is of, on the card, or on its
/// way between them.
pub fn holds(at: POINT) -> bool {
    CARD.with(|kept| {
        let kept = kept.borrow();
        let holds = kept.as_ref().filter(|card| card.shown.is_some()).and_then(|card| card.holds);
        holds.is_some_and(|holds| within(at, holds))
    })
}

/// Shows nothing more.
pub fn hide() {
    CARD.with(|kept| {
        if let Some(card) = kept.borrow_mut().as_mut().filter(|card| card.shown.is_some()) {
            card.window.hide();
            card.shown = None;
            card.holds = None;
            card.hidden();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the lane shows is as far in from every edge of the sheet as the sheet's corners are round, in every skin.
    #[test]
    fn what_the_card_shows_is_as_far_in_as_its_corners_are_round() {
        use super::super::theme::Skin;
        for skin in [Skin::Glass, Skin::Paper, Skin::Fluent] {
            let theme = Theme::new(skin, true);
            let lane = view::Rect { x: 6.0, y: 10.0, w: 300.0, h: 400.0 };
            let ((width, height), (dx, dy)) = sheet_of(&theme, lane);
            // (Where what it shows begins and ends, on the sheet.)
            let (left, top) = (lane.x + dx + theme.pad_x, lane.y + dy + theme.pad_top);
            let (right, bottom) = (lane.x + dx + lane.w - theme.pad_x, lane.y + dy + lane.h - theme.pad_bottom);
            let insets = [left, top, width - right, height - bottom];
            let round = round(&theme, (width, height)).max(super::super::look::INSET_LEAST);
            assert!(insets.iter().all(|inset| (inset - round).abs() < 1e-4), "{skin:?} {insets:?} round {round}");
        }
    }

    /// The card's glass is frosted as what is behind it asks, not as what is behind the dock does: clear over a calm
    /// dark ground under dark glass, frosted over text, and as frosted as it may be where what is behind it was read
    /// elsewhere (it has moved) or not yet.
    #[test]
    fn the_cards_glass_is_frosted_as_what_is_behind_it_asks() {
        let (here, there) = (RECT { left: 100, top: 100, right: 500, bottom: 600 }, RECT { left: 300, top: 100, right: 700, bottom: 600 });
        let (calm, text) = ((0.05, 0.01), (0.12, 0.3));
        assert_eq!(frost_over(Some((here, calm)), here, true), 0.0);
        assert_eq!(frost_over(Some((here, text)), here, true), 1.0);
        assert_eq!(frost_over(Some((there, calm)), here, true), 1.0);
        assert_eq!(frost_over(None, here, true), 1.0);
    }

    /// Its sheet is as round as the dock's pieces in every skin (glass's 24, not a menu's 14), and no rounder than half
    /// its side.
    #[test]
    fn the_card_is_as_round_as_the_dock() {
        use super::super::theme::Skin;
        for skin in [Skin::Glass, Skin::Paper, Skin::Fluent] {
            let theme = Theme::new(skin, true);
            assert_eq!(round(&theme, (420.0, 460.0)), theme.radius, "{skin:?}");
        }
        assert_eq!(round(&Theme::new(Skin::Glass, true), (420.0, 30.0)), 15.0);
    }

    /// The pointer keeps the card up on the ring, on the card, and on its way from one to the other (over the rest of
    /// the dock, across the gap), and nowhere past them: a dock down the left edge, its card to its right, taller.
    #[test]
    fn the_pointer_keeps_the_card_on_its_way_from_the_ring() {
        let ring = RECT { left: 10, top: 300, right: 74, bottom: 380 };
        let card = RECT { left: 92, top: 100, right: 512, bottom: 560 };
        let held = held_by(ring, card);
        for (at, kept) in [((40, 340), true), ((80, 340), true), ((88, 340), true), ((300, 120), true), ((511, 559), true), ((5, 340), false), ((300, 90), false), ((520, 340), false), ((300, 570), false)] {
            assert_eq!(within(POINT { x: at.0, y: at.1 }, held), kept, "{at:?}");
        }
    }
}
