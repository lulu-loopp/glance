//! The card beside a panel or a widget, of what on it the pointer rests on that has one (see `Hit::carded`): of a spot
//! of the AI component's dock, a source's ring (the source's lane, all it tells: its limit's windows, what it spent
//! today by model and by kind, its sessions), what every source spent today, or the sessions at work (see
//! `view::ai::card`); of a folder in the storage lane, what it holds (see `view::folders::card`). On a sheet of its skin
//! as a menu opened there is (glass's a piece of the glass, frosted as what it is of is), beside it, its middle by that
//! spot's, on its screen. A window of its own, one for this thread, that goes as the window it is beside is hidden or
//! destroyed. A dock's takes the pointer on its sheet alone, and stays while the pointer is on the spot, on the card or
//! on its way between them (see `holds`); a folder's lets the pointer through, and goes as it leaves the folder's row.

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
use super::view::{self, Hit, Layout, Scene};
use super::backdrop::Capture;
use super::window::Window;

/// Which side of what it is of the card is on: away from the screen's edge that is at.
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
    /// Glass's, while it shows: where the card of each spot on the dock shows (see `reach`), and what is behind there
    /// as last seen with no card over it; the windows behind it put together (see `scene`) to see that again, and when
    /// it last asked for them; how frosted it was last drawn.
    reach: Option<RECT>,
    seen: Option<Capture>,
    behind: Option<super::scene::Scene>,
    asked: Option<Instant>,
    frost: f32,
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
        self.words.release();
    }

    fn let_go_of_kept(&mut self) {
        self.surface = None;
        self.seen = None;
        self.behind = None;
    }
}

impl Card {
    /// Drawn with `gfx` from now on: where that is another device, what it drew with on the one before goes with it
    /// (see `life`), what is written on it among that (made there, it cannot be drawn on another).
    fn moved_to(&mut self, gfx: &Rc<Gfx>) {
        if !Rc::ptr_eq(&self.gfx, gfx) {
            self.gone();
            self.gfx = gfx.clone();
        }
    }

    /// Whether it shows what is known by `key` as it is, drawn: not where what it drew with went (moved to another
    /// device), though it shows the same.
    fn drawn_as(&self, key: u64) -> bool {
        self.shown == Some(key) && self.surface.is_some()
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
    /// How frosted glass is drawn over what is behind `sheet` (physical px, `px` to a DIP), `dark` or light, a card
    /// beside the dock showing within `reach` wherever it shows: as the panel's glass is over what is behind it (see
    /// `frost_over`), not as the dock's is (what is behind the dock may be calm, and what is behind the card busy).
    /// Known as it first shows, and as it moves from spot to spot: what is behind all of `reach` is taken at once while
    /// no card is over it (a few milliseconds: it is no larger than the cards). While one is, the windows behind it are
    /// put together and taken instead (the screen has the card in it), every `LOOK_AGAIN` and as `reach` moves. Where
    /// none of that holds the sheet, as frosted as it was.
    fn frost(&mut self, sheet: RECT, reach: RECT, px: f32, dark: bool) -> f32 {
        let holds = |seen: &Option<Capture>| seen.as_ref().is_some_and(|seen| within_area(sheet, seen.rect));
        if self.shown.is_none() {
            if !holds(&self.seen) {
                self.seen = Capture::take(reach);
            }
        } else {
            if self.behind.is_none() {
                // (Read on another thread: what draws the dock draws again, and the card with it.)
                let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| crate::app().controller.wake());
                self.behind = super::scene::Scene::new(reach, wake, &self.gfx);
            }
            if let Some(scene) = &mut self.behind {
                let arranged = scene.arrange(reach, self.window.hwnd);
                if arranged || self.asked.is_none_or(|at| at.elapsed() >= LOOK_AGAIN) {
                    scene.want();
                    self.asked = Some(Instant::now());
                }
                if let Some(capture) = scene.ready() {
                    self.seen = Some(capture);
                }
            }
        }
        let tone = self.seen.as_ref().filter(|_| holds(&self.seen)).map(|seen| seen.luminance(sheet, px));
        self.frost = frost_over(tone, dark, self.frost);
        self.frost
    }
}

/// How frosted glass is drawn, `dark` or light, over what is behind it read as `tone` (its luminance's mean and
/// spread): as the panel's glass is (see `skins::frost`), in twentieths (not drawn again for less). Not read, as it
/// `was`.
fn frost_over(tone: Option<(f32, f32)>, dark: bool, was: f32) -> f32 {
    match tone {
        Some((mean, spread)) => (super::skins::frost(mean, spread, dark) * 20.0).round() / 20.0,
        None => was,
    }
}

/// Where the cards at `sheets` (on the screen) show between them: the least that holds all of them.
fn reach(sheets: impl IntoIterator<Item = RECT>) -> Option<RECT> {
    sheets.into_iter().reduce(|all, sheet| RECT { left: all.left.min(sheet.left), top: all.top.min(sheet.top), right: all.right.max(sheet.right), bottom: all.bottom.max(sheet.bottom) })
}

/// The middle of `rect`.
fn middle(rect: RECT) -> POINT {
    POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 }
}

/// Where a card `plate` large (physical px) shows beside `by` on `side`, `gap` from it: along it, its middle by `at`'s
/// (its ring's middle), all of it in `work` (`gap` in from its edges), and clear of `clear` (the dock's buttons outside
/// it, on the screen): where it would lie over them, slid along the dock past them (the nearer way that keeps it in
/// `work`), `gap` from them, as near the dock as every other card. (Placed beside the buttons too, every card stood off
/// the dock by their row, far from the ring it tells of.)
fn placed(by: RECT, at: POINT, side: Side, (plate_w, plate_h): (i32, i32), gap: i32, work: RECT, clear: Option<RECT>) -> RECT {
    let along = |middle: i32, length: i32, low: i32, high: i32| (middle - length / 2).min(high - gap - length).max(low + gap);
    let (left, top) = match side {
        Side::Left => (by.left - gap - plate_w, along(at.y, plate_h, work.top, work.bottom)),
        Side::Right => (by.right + gap, along(at.y, plate_h, work.top, work.bottom)),
        Side::Below => (along(at.x, plate_w, work.left, work.right), by.bottom + gap),
        Side::Above => (along(at.x, plate_w, work.left, work.right), by.top - gap - plate_h),
    };
    let sheet = RECT { left, top, right: left + plate_w, bottom: top + plate_h };
    let Some(clear) = clear.filter(|clear| overlap(sheet, *clear, gap)) else { return sheet };
    // (Along the dock: across it for a card under or over it, down it for one beside it.)
    let (at, length, before, after, low, high) = match side {
        Side::Below | Side::Above => (left, plate_w, clear.left - gap - plate_w, clear.right + gap, work.left + gap, work.right - gap),
        Side::Left | Side::Right => (top, plate_h, clear.top - gap - plate_h, clear.bottom + gap, work.top + gap, work.bottom - gap),
    };
    let fits = |start: i32| start >= low && start + length <= high;
    let to = match (fits(before), fits(after)) {
        (true, true) => if at - before <= after - at { before } else { after },
        (true, false) => before,
        (false, true) => after,
        // (No room past them either way: over them, as near its ring as it may be.)
        (false, false) => at,
    };
    match side {
        Side::Below | Side::Above => RECT { left: to, right: to + plate_w, ..sheet },
        Side::Left | Side::Right => RECT { top: to, bottom: to + plate_h, ..sheet },
    }
}

/// Where the card of the spot at `about` in `by` stands: beside `by` on `side` (see `placed`), or, where the screen has no
/// room for it there, by the spot itself, under it, else over it, all of it in `work`.
fn stood(by: RECT, about: RECT, side: Side, plate: (i32, i32), gap: i32, work: RECT, clear: Option<RECT>) -> RECT {
    let sheet = placed(by, middle(about), side, plate, gap, work, clear);
    let room = match side {
        Side::Left => sheet.left >= work.left,
        Side::Right => sheet.right <= work.right,
        Side::Below => sheet.bottom <= work.bottom,
        Side::Above => sheet.top >= work.top,
    };
    if room {
        return sheet;
    }
    let left = (middle(about).x - plate.0 / 2).min(work.right - gap - plate.0).max(work.left + gap);
    let under = about.bottom + gap;
    let top = if under + plate.1 <= work.bottom { under } else { (about.top - gap - plate.1).max(work.top) };
    RECT { left, top, right: left + plate.0, bottom: top + plate.1 }
}

/// Whether `a` and `b` lie nearer than `gap` to each other, or over each other.
fn overlap(a: RECT, b: RECT, gap: i32) -> bool {
    a.left < b.right + gap && b.left < a.right + gap && a.top < b.bottom + gap && b.top < a.bottom + gap
}

/// Whether all of `inner` is in `outer`.
fn within_area(inner: RECT, outer: RECT) -> bool {
    inner.left >= outer.left && inner.top >= outer.top && inner.right <= outer.right && inner.bottom <= outer.bottom
}

/// What the card of `hit` tells, laid out as a lane: none where it has none now (a folder not counted yet).
fn told(scene: &Scene, hit: Hit) -> Option<view::Lane> {
    match hit {
        Hit::AiSlot(spot) => view::ai::card(scene, spot),
        Hit::Folder(number) => view::folders::card(scene, number),
        _ => None,
    }
}

/// Whether the pointer may go from `hit` onto its card (a dock's: see `holds`).
fn reached(hit: Hit) -> bool {
    matches!(hit, Hit::AiSlot(_))
}

/// How long the pointer rests on a folder before its card shows, with none up.
const REST: Duration = Duration::from_millis(260);

/// What the pointer rests on that has a card, and since when.
#[derive(Default)]
pub struct Resting(Option<(Hit, Instant)>);

impl Resting {
    /// The pointer is on `hit`, a card `up` already or not: when its card shows (a dock's, and any while one is up, at
    /// once).
    pub fn on(&mut self, hit: Hit, up: bool, now: Instant) -> Instant {
        let since = match self.0 {
            Some((on, since)) if on == hit => since,
            _ => now,
        };
        self.0 = Some((hit, since));
        if up || reached(hit) { now } else { since + REST }
    }

    /// The pointer is on nothing that has one.
    pub fn off(&mut self) {
        self.0 = None;
    }

    /// When the card of what it rests on shows.
    pub fn due(&self) -> Option<Instant> {
        self.0.map(|(_, since)| since + REST)
    }
}

/// What the card shows of: `hit`, in `theme`, as `prefs` show the components (their modules held: see
/// `Prefs::holding`), in `lang`, on a sheet that looks as a menu opened there does (`look`).
pub struct Of<'a> {
    pub hit: Hit,
    pub theme: &'a Theme,
    pub prefs: &'a Prefs,
    pub lang: Lang,
    pub look: Look,
}

/// Shows `of`'s card beside `by` (what it is drawn in, on the screen) on `side`, its middle by its spot's (among `spots`,
/// every spot there that has a card and where it is), all of it on that screen (see `stood`), clear of `clear` (the
/// buttons outside `by`), till `hide` or till `owner` (the window it is drawn in) is hidden or destroyed. Shown again as
/// it is, nothing is drawn.
pub fn show(owner: HWND, of: Of, by: RECT, spots: &[(Hit, RECT)], side: Side, clear: Option<RECT>) {
    let Some(&(_, about)) = spots.iter().find(|(spot, _)| *spot == of.hit) else { return };
    let _timed = crate::perf::Timed::start("card", || format!("{:?}", of.hit));
    // (Not sized by hand, whatever the dock it is shown from is: what is written on it is kept, see `gfx::sizing`.)
    let _at_rest = gfx::sizing(false);
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
    let Some(lane) = told(&scene, of.hit) else { return hide() };
    let laid = |lane: &view::Lane| Layout::with_columns(vec![lane.height(of.theme)], 1, of.theme);
    let area = laid(&lane).lanes()[0];
    let (plate, shift) = sheet_of(of.theme, area);
    let at = middle(about);
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
    // Beside it; along it, its middle by the spot's, all of it in the work area.
    let sheet = stood(by, about, side, (plate_w, plate_h), gap, work, clear);
    let (left, top) = (sheet.left, sheet.top);
    let holds = reached(of.hit).then(|| held_by(about, sheet));
    // (On the device for the screen it shows on: see `gfx::on_screen`.)
    let Ok(gfx) = gfx::for_area(sheet) else { return };
    CARD.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            let Ok(window) = Window::new() else { return };
            *kept = Some(Card { window, gfx: gfx.clone(), surface: None, words: gfx::Layer::default(), holds: None, reach: None, seen: None, behind: None, asked: None, frost: 1.0, showing: None, shown: None });
        }
        let card = kept.as_mut().unwrap();
        OWNER.set(owner);
        card.moved_to(&gfx);
        card.holds = holds;
        // (The room round the sheet for its shadows is over the dock: the pointer there is the dock's. A card not
        // reached lets the pointer through, all of it.)
        card.window.set_click_through(!reached(of.hit));
        card.window.take_pointer_within(Some(sheet));
        // Glass frosted as what is behind the card asks (see `Card::frost`), what is behind every spot's card taken
        // (where they show found again as the dock or what a card tells moves this one out of it).
        let look = match of.theme.skin {
            super::theme::Skin::Glass => {
                if card.reach.is_none_or(|reach| !within_area(sheet, reach)) {
                    let sheets = spots.iter().filter(|(spot, _)| reached(*spot) == reached(of.hit)).filter_map(|&(spot, about)| {
                        let (plate, _) = sheet_of(of.theme, laid(&told(&scene, spot)?).lanes()[0]);
                        Some(stood(by, about, side, (px(plate.0), px(plate.1)), gap, work, clear))
                    });
                    card.reach = reach(sheets.chain([sheet]));
                }
                of.look.frosted(card.frost(sheet, card.reach.unwrap_or(sheet), scale, of.theme.dark), true)
            }
            _ => of.look,
        };
        let mut key = std::hash::DefaultHasher::new();
        (&lane, of.theme.skin, of.theme.dark, look.frost.to_bits(), look.zoom.to_bits(), left, top, plate_w, plate_h).hash(&mut key);
        let key = key.finish();
        if card.drawn_as(key) {
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
            write(frame, &scene, &lane, area, shift);
        });
        // (The device gone, its surface is made anew on the next.)
        if let Err(error) = &drawn {
            if !gfx::failed("card", &card.gfx, Some(error)) {
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

/// Where the pointer keeps the card at `card` (on the screen) up, beside the spot `about`: on the spot, on the card, and
/// on its way between them.
fn held_by(about: RECT, card: RECT) -> RECT {
    RECT { left: about.left.min(card.left), top: about.top.min(card.top), right: about.right.max(card.right), bottom: about.bottom.max(card.bottom) }
}

/// Writes on its sheet what `lane` shows, laid out in `area` and moved on the sheet by `shift` (see `sheet_of`).
pub(super) fn write(frame: &gfx::Frame, scene: &Scene, lane: &view::Lane, area: view::Rect, shift: (f32, f32)) {
    frame.within(windows_numerics::Matrix3x2::translation(shift.0, shift.1), |frame| view::paint_alone(frame, scene, lane, area));
}

/// Its sheet in `theme` for a lane laid out at `lane` (DIPs), and how far what is laid out is moved on it: what the lane
/// shows as far in from every edge as the sheet's corners are round (their arcs' centres), the same all round, and no
/// less than a sheet's least (see `look`).
pub(super) fn sheet_of(theme: &Theme, lane: view::Rect) -> ((f32, f32), (f32, f32)) {
    let inset = theme.radius.max(super::look::INSET_LEAST);
    let plate = (lane.w - 2.0 * theme.pad_x + 2.0 * inset, lane.h - theme.pad_top - theme.pad_bottom + 2.0 * inset);
    (plate, (inset - theme.pad_x - lane.x, inset - theme.pad_top - lane.y))
}

/// How round its sheet `plate` large (DIPs) is in `theme`: as the dock's pieces are, and the panel's, for it shows what
/// they do (not as a menu's, which are rounder in no skin).
pub(super) fn round(theme: &Theme, plate: (f32, f32)) -> f32 {
    theme.radius.min(plate.0.min(plate.1) / 2.0)
}

fn within(at: POINT, area: RECT) -> bool {
    at.x >= area.left && at.x < area.right && at.y >= area.top && at.y < area.bottom
}

/// Whether it shows, and the pointer at `at` (on the screen) keeps it: on the spot it is of, on the card, or on its
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
            // (Where it shows found, and what is behind there seen, again as it next shows, as they are then.)
            card.reach = None;
            card.seen = None;
            if let Some(scene) = &mut card.behind {
                scene.rest();
            }
            card.hidden();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card moved to another device (its screen driven by another adapter, or its device lost) lets go of what it
    /// drew with on the one before, what is written on it among that, and is drawn again there though it shows the
    /// same: what was written on it is written again on its new device, not drawn there from the old one's.
    #[test]
    fn a_card_moved_to_another_device_draws_its_words_again_there() {
        let _device = gfx::held().unwrap();
        let window = Window::new().expect("a window");
        let mut card = Card { window, gfx: gfx::for_main_screen().expect("graphics"), surface: None, words: gfx::Layer::default(), holds: None, reach: None, seen: None, behind: None, asked: None, frost: 1.0, showing: None, shown: None };
        let painted = Cell::new(0);
        let draw = |card: &mut Card| {
            let device = card.gfx.clone();
            gfx::Frame::whole(&device, (32, 32), 1.0, |frame| {
                card.words.draw(frame, 7, windows_numerics::Matrix3x2::identity(), (0.0, 0.0, 16.0, 16.0), 1.0, None, |_| {
                    painted.set(painted.get() + 1);
                    Ok(())
                })
                .expect("drawn");
            })
            .expect("frame");
            painted.get()
        };
        assert_eq!(draw(&mut card), 1);
        assert_eq!(draw(&mut card), 1, "the same: drawn from what was kept");
        let look = Look::of(super::super::theme::Skin::Paper, false);
        card.surface = Some((Drawn::new(&card.gfx, card.window.hwnd, look).expect("a surface"), look));
        card.shown = Some(7);
        assert!(card.drawn_as(7));
        card.moved_to(&gfx::in_software());
        assert!(!card.drawn_as(7), "moved to another device: drawn again there though it shows the same");
        assert_eq!(draw(&mut card), 2, "what is written on it written again on its new device");
    }

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
    /// dark ground under dark glass, frosted over text, and as it was while what is behind it is not read.
    #[test]
    fn the_cards_glass_is_frosted_as_what_is_behind_it_asks() {
        let (calm, text) = ((0.05, 0.01), (0.12, 0.3));
        assert_eq!(frost_over(Some(calm), true, 1.0), 0.0);
        assert_eq!(frost_over(Some(text), true, 0.0), 1.0);
        assert_eq!(frost_over(None, true, 0.4), 0.4);
    }

    /// What is taken of what is behind before any card shows holds the card of every ring on the dock, however tall
    /// each is, on every side, by the screen's edges too (the first card and each next one frosted as what is behind
    /// them asks from their first frame), and no more than they cover between them (taken at once, it is no larger).
    #[test]
    fn what_is_seen_before_a_card_shows_holds_every_card_beside_the_dock_and_no_more() {
        let (work, gap, plate_w) = (RECT { left: 0, top: 0, right: 3440, bottom: 1400 }, 12, 640);
        let docks = [
            (RECT { left: 1400, top: 0, right: 2060, bottom: 140 }, Side::Below),
            (RECT { left: 10, top: 0, right: 700, bottom: 140 }, Side::Below),
            (RECT { left: 2900, top: 0, right: 3430, bottom: 140 }, Side::Below),
            (RECT { left: 2900, top: 1260, right: 3430, bottom: 1400 }, Side::Above),
            (RECT { left: 0, top: 400, right: 160, bottom: 1000 }, Side::Right),
            (RECT { left: 3280, top: 20, right: 3440, bottom: 700 }, Side::Left),
        ];
        for (by, side) in docks {
            // (Four rings along the dock, their cards of four heights.)
            let (w, h) = (by.right - by.left, by.bottom - by.top);
            let sheets: Vec<RECT> = (0..4)
                .map(|i| {
                    let ring = match side {
                        Side::Below | Side::Above => RECT { left: by.left + w * i / 4, top: by.top, right: by.left + w * (i + 1) / 4, bottom: by.bottom },
                        _ => RECT { left: by.left, top: by.top + h * i / 4, right: by.right, bottom: by.top + h * (i + 1) / 4 },
                    };
                    placed(by, middle(ring), side, (plate_w, [300, 1100, 200, 700][i as usize]), gap, work, None)
                })
                .collect();
            let area = reach(sheets.iter().copied()).unwrap();
            for sheet in &sheets {
                assert!(within_area(*sheet, area), "{side:?} by {by:?}: {sheet:?} not in {area:?}");
            }
            // (Each of its edges is one of the cards'.)
            let edges = [|r: &RECT| r.left, |r: &RECT| r.top, |r: &RECT| r.right, |r: &RECT| r.bottom];
            for edge in edges {
                assert!(sheets.iter().any(|sheet| edge(sheet) == edge(&area)), "{side:?} by {by:?}: {area:?}");
            }
        }
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

    /// A card whose side of what it is beside has no room for it on the screen (a widget nearly as wide as that) stands
    /// by its spot: under it, or over it where there is no room under it, all of it on the screen.
    #[test]
    fn a_card_with_no_room_beside_stands_by_its_spot() {
        let (work, gap, plate) = (RECT { left: 0, top: 0, right: 1920, bottom: 1040 }, 12, (500, 400));
        let wide = RECT { left: 100, top: 40, right: 1800, bottom: 1000 };
        let row = |top: i32| RECT { left: 120, top, right: 600, bottom: top + 40 };
        let inside = |sheet: RECT| within_area(sheet, work);
        for side in [Side::Left, Side::Right, Side::Below, Side::Above] {
            let under = stood(wide, row(100), side, plate, gap, work, None);
            assert!(inside(under) && under.top == 140 + gap, "{side:?} {under:?}");
            let over = stood(wide, row(900), side, plate, gap, work, None);
            assert!(inside(over) && over.bottom == 900 - gap, "{side:?} {over:?}");
        }
        // With room there, beside it as ever.
        let narrow = RECT { left: 100, top: 40, right: 800, bottom: 1000 };
        assert_eq!(stood(narrow, row(100), Side::Right, plate, gap, work, None), placed(narrow, middle(row(100)), Side::Right, plate, gap, work, None));
    }

    /// A folder's card shows once the pointer has rested on it, at once while another's is up; a dock's at once.
    #[test]
    fn a_folder_s_card_waits_for_the_pointer_to_rest() {
        let now = Instant::now();
        let mut resting = Resting::default();
        assert_eq!(resting.on(Hit::Folder(1), false, now), now + REST);
        assert_eq!(resting.due(), Some(now + REST));
        let later = now + Duration::from_millis(100);
        assert_eq!(resting.on(Hit::Folder(1), false, later), now + REST);
        assert_eq!(resting.on(Hit::Folder(2), false, later), later + REST);
        assert_eq!(resting.on(Hit::Folder(3), true, later), later);
        resting.off();
        assert_eq!(resting.due(), None);
        assert_eq!(resting.on(Hit::AiSlot(view::ai::Spot::Today), false, now), now);
    }

    /// A card that would lie over the dock's buttons outside it slides along the dock past them, as near the dock as
    /// every other card: under a dock across the top, its buttons under its right end, the last ring's card slides left
    /// of them (to their right, where the screen ends there), the first ring's stays over its ring; beside a dock down
    /// the right edge, its buttons in a column on its left from its top, the first ring's card slides under them.
    #[test]
    fn a_card_slides_past_the_docks_buttons() {
        let (work, gap, plate) = (RECT { left: 0, top: 0, right: 3440, bottom: 1400 }, 16, (1100, 900));
        let (dock, buttons) = (RECT { left: 1400, top: 10, right: 2200, bottom: 140 }, RECT { left: 2080, top: 140, right: 2200, bottom: 200 });
        let plain = placed(dock, POINT { x: 2140, y: 75 }, Side::Below, plate, gap, work, None);
        assert!(overlap(plain, buttons, gap));
        let slid = placed(dock, POINT { x: 2140, y: 75 }, Side::Below, plate, gap, work, Some(buttons));
        assert_eq!((slid.top, slid.right), (plain.top, buttons.left - gap));
        let first = placed(dock, POINT { x: 1450, y: 75 }, Side::Below, plate, gap, work, Some(buttons));
        assert_eq!(first, placed(dock, POINT { x: 1450, y: 75 }, Side::Below, plate, gap, work, None));
        // (No room left of them: to their right.)
        let narrow = RECT { left: 1400, right: 3440, ..work };
        let right = placed(dock, POINT { x: 2140, y: 75 }, Side::Below, (800, 900), gap, narrow, Some(buttons));
        assert_eq!(right.left, buttons.right + gap);
        assert!(!overlap(right, buttons, gap) && right.top == plain.top);
        // (Beside a dock down the right edge, its buttons on its left from its top.)
        let (dock, buttons) = (RECT { left: 3300, top: 300, right: 3430, bottom: 1000 }, RECT { left: 3240, top: 300, right: 3300, bottom: 420 });
        let slid = placed(dock, POINT { x: 3360, y: 340 }, Side::Left, (500, 300), gap, work, Some(buttons));
        assert!(!overlap(slid, buttons, gap) && slid.right == dock.left - gap && slid.top == buttons.bottom + gap);
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
