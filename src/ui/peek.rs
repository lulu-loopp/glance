//! The notice at the screen's edge for nearby transfer (see `docs/nearby.md`): a device asking to send is asked about
//! there (its name, what it sends, Accept and Decline, and, for a device that showed its certificate, whether its
//! files are taken from now on without asking); and while no panel is up, what comes is told as it comes, and what
//! came by its picture. Pressed then, the panel opens where the station is, and the picture is carried onto its face
//! there.
//!
//! A plate drawn as the panel's skin draws its plates (see `menu::plate`), slid out of the edge of the pointer's screen
//! that opens the workbench, in a window of the panel's thread. What came goes back in after a while, held while the
//! pointer is on it. Nothing of it is shown while a game is in front: a device asking meanwhile is asked once the game
//! is left (it waits a minute for its answer).

use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_LAYER_PARAMETERS1;
use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};

use super::canvas::{Align, Canvas, Point};
use super::gfx::{self, Frame, Gfx, Surface};
use super::icons::Icon;
use super::life::{Lives, Showing};
use super::text::Lang;
use super::theme::{Skin, Theme};
use super::window::Window;
use crate::mouse::Contact;
use crate::nearby::{Asking, Offered, Receiving};
use crate::settings::{Edge, Part, Places};

/// Its plate's measures (DIPs): how wide, the room inside its edge, how far from the screen's edge, and the room round
/// it for its shadow (a menu's).
const WIDE: f32 = 288.0;
const PAD: f32 = 14.0;
const GAP: f32 = 12.0;
const ROOM: f32 = 16.0;
/// How far over the middle of a side edge its top stands (DIPs).
const ABOVE_MIDDLE: f32 = 60.0;
/// Its rows (DIPs): the device's, the picture's beside what is told, a file offered, the line saying how many more,
/// the box to trust the device, the buttons, the bar of what has come, and the space between them.
const HEAD: f32 = 34.0;
const THUMB: f32 = 48.0;
const FILE_ROW: f32 = 30.0;
const MORE_ROW: f32 = 20.0;
const TRUST_ROW: f32 = 20.0;
const BUTTON: f32 = 32.0;
const BAR: f32 = 4.0;
const SPACE: f32 = 10.0;
/// The most files an asking lists, each on its row (the rest counted under them).
const LISTED: usize = 3;

/// How long it takes to slide out and back, and to carry the picture onto the station.
const SLIDE_OUT: Duration = Duration::from_millis(340);
const SLIDE_IN: Duration = Duration::from_millis(260);
const FLIGHT: Duration = Duration::from_millis(520);
/// How long what came is told before it goes back (held while the pointer is on it), and how long after it came it may
/// still be told (it came while a panel was up, or a game was in front: no longer news after that).
const STAYS: Duration = Duration::from_secs(4);
const FRESH: Duration = Duration::from_secs(10);
/// How often it looks whether the pointer is still on it, while that holds it.
const LOOK_EVERY: Duration = Duration::from_millis(120);

/// What it says.
#[derive(Clone, Debug, PartialEq)]
enum Says {
    /// A device asks whether it may send.
    Ask(Asking),
    /// What a device sends, as it comes.
    Coming(Receiving),
    /// What came (the thing `id` on the station), from whom, how many files, and whether all are pictures.
    Came { id: u64, from: String, kind: Option<String>, files: usize, pictures: bool },
}

/// A part of it a press does something on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    /// The whole of it (what comes, what came): the panel opened there.
    Body,
    Trust,
    Decline,
    Accept,
}

/// What it asks of the panel's thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    /// The panel opened at `edge` of the screen `at` is on, along it at `at`.
    Open { edge: Edge, at: POINT },
}

/// Where it stands: its screen, the edge it slides out of, its plate at rest (physical px) and its window (its plate,
/// with room for its shadow, and on to the screen's edge, which it slides out of).
#[derive(Clone, Copy, PartialEq)]
struct Place {
    contact: Contact,
    edge: Edge,
    plate: RECT,
    window: RECT,
    /// The line it slides out of, where that is not its screen's edge: the side of a panel up there.
    line: Option<i32>,
}

/// The picture of what came carried onto its face on the station: from where its picture was in the notice (physical
/// px), the thing it is, since when, and where it lands (as the panel draws it: found once it is drawn).
struct Flight {
    from: RECT,
    id: u64,
    began: Instant,
    to: Option<RECT>,
}

/// Its window, what it draws with there, and its being shown on that device (see `life`).
struct Drawn {
    window: Window,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    showing: Option<Showing>,
}

impl Lives for Drawn {
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

/// What the panel's thread knows as the notice looks again (see `Peek::tick`).
pub struct Seen<'a> {
    /// Whether a panel is up.
    pub panel_up: bool,
    /// Whether a game is in front (asked only while something would be told).
    pub game: &'a dyn Fn() -> bool,
    /// Where the face of the thing on the station `id` is drawn on a panel up, on the screen (physical px).
    pub face: &'a dyn Fn(u64) -> Option<RECT>,
    /// The panels up at an edge: each one's screen, its edge, and where it is (physical px).
    pub panels: &'a [(RECT, Edge, RECT)],
}

/// The notice: made once something is first told, kept while Glance runs.
#[derive(Default)]
pub struct Peek {
    drawn: Option<Drawn>,
    /// What it tells now, and what its plate shows (what it told, while it slides back in).
    says: Option<Says>,
    plate: Option<Says>,
    place: Option<Place>,
    /// How far out it is (0: in the edge, 1: out), and where it is going: out, from where it was, since when.
    out: f32,
    sliding: Option<(f32, bool, Instant)>,
    /// What came last that was looked at (its moment, as the station has it): each is told once.
    told: Option<Instant>,
    /// When what came goes back in (none: it stays, as an asking does).
    until: Option<Instant>,
    /// The asking it shows and whether its device is to be trusted (the box ticked).
    trust: (u64, bool),
    pressed: Option<Spot>,
    over: Option<Spot>,
    /// Its parts a press does something on, as last drawn (DIPs, from its plate's corner).
    spots: Vec<(f32, f32, f32, f32, Spot)>,
    flight: Option<Flight>,
    /// Drawn again on the next look: what it shows changed, or its picture was not yet made.
    stale: bool,
}

impl Peek {
    /// Whether `hwnd` is its window.
    pub fn owns(&self, hwnd: windows::Win32::Foundation::HWND) -> bool {
        self.drawn.as_ref().is_some_and(|drawn| drawn.window.hwnd == hwnd)
    }

    /// Looks again at what is to be told and draws it as it goes: what is asked, what comes, what came. When it wants
    /// to look again (none: only as it is woken).
    pub fn tick(&mut self, now: Instant, seen: &Seen) -> Option<Duration> {
        if self.flight.is_some() {
            return self.fly(now, seen);
        }
        let wanted = self.wanted(now, seen);
        if wanted != self.says {
            self.change(wanted, now, seen.panels);
        }
        // Sliding out or back in.
        let mut due = None;
        if let Some((from, out, since)) = self.sliding {
            let lasts = if out { SLIDE_OUT } else { SLIDE_IN };
            let t = (now.saturating_duration_since(since).as_secs_f32() / lasts.as_secs_f32()).min(1.0);
            let eased = if out { super::morph::ease(t) } else { t * t };
            self.out = from + (if out { 1.0 } else { 0.0 } - from) * eased;
            self.stale = true;
            if t >= 1.0 {
                self.sliding = None;
                if !out {
                    self.hide();
                }
            } else {
                due = Some(Duration::ZERO);
            }
        }
        // What came going back in its time, but while the pointer is on it.
        if let (Some(until), None) = (self.until, self.sliding) {
            if self.pointer_on() {
                self.until = Some(until.max(now + LOOK_EVERY));
            }
            match self.until {
                Some(until) if until <= now => {
                    self.until = None;
                    self.says = None;
                    self.slide(false, now);
                    due = Some(Duration::ZERO);
                }
                Some(until) => due = Some(until - now),
                None => {}
            }
        }
        // Lit as the pointer is on a part of it: looked at again till it leaves (no message says it has).
        if self.over.is_some() {
            if !self.pointer_on() {
                self.over = None;
                self.stale = true;
            } else {
                due = Some(due.map_or(LOOK_EVERY, |due: Duration| due.min(LOOK_EVERY)));
            }
        }
        if self.stale && self.plate.is_some() {
            self.stale = false;
            if !self.draw() {
                // (Its picture still being made: told as it is, and looked at again meanwhile.)
                self.stale = true;
                due = Some(due.map_or(LOOK_EVERY, |due: Duration| due.min(LOOK_EVERY)));
            }
        }
        due
    }

    /// What is to be told now: the first asking (but over a game); else, no panel up, what comes, or what came lately
    /// and was not told; nothing while a game is in front.
    fn wanted(&mut self, now: Instant, seen: &Seen) -> Option<Says> {
        let nearby = crate::nearby::shown();
        let station = crate::station::run::shown();
        let came = station.arrived.as_ref().filter(|(_, at)| self.told.is_none_or(|told| *at > told)).cloned();
        let notice = crate::app().settings.lock().unwrap().nearby_notice;
        let wanted = if let Some(asking) = nearby.asks.first() {
            Some(Says::Ask(asking.clone()))
        } else if !notice || seen.panel_up {
            None
        } else if let Some(receiving) = nearby.receiving {
            Some(Says::Coming(receiving))
        } else if let Some((ids, _)) = came.as_ref().filter(|(_, at)| now.saturating_duration_since(*at) < FRESH) {
            ids.last().and_then(|id| came_of(&station.items, *id))
        } else {
            // What it told, while it is told still (it goes in its time).
            self.says.clone().filter(|says| matches!(says, Says::Came { .. }) && self.until.is_some())
        };
        // (What came, looked at: told now, or not at all, a panel up or a game in front.)
        if let Some((_, at)) = came {
            self.told = Some(at);
        }
        wanted.filter(|_| !(seen.game)())
    }

    /// It says `wanted` from now: out of the edge where it was not out (beside the panel up there, if one is: of
    /// `panels`), sized and placed again for it, or back in.
    fn change(&mut self, wanted: Option<Says>, now: Instant, panels: &[(RECT, Edge, RECT)]) {
        let was = std::mem::replace(&mut self.says, wanted.clone());
        self.pressed = None;
        self.stale = true;
        let Some(says) = wanted else {
            self.until = None;
            if was.is_some() {
                self.slide(false, now);
            }
            return;
        };
        if let Says::Ask(asking) = &says {
            if self.trust.0 != asking.id {
                self.trust = (asking.id, false);
            }
        }
        self.until = matches!(says, Says::Came { .. }).then(|| now + SLIDE_OUT + STAYS);
        let size = (WIDE, tall(&says));
        // (Out already: where it is, resized; else where the pointer is, sliding out.)
        let out = was.is_some() && self.sliding.is_none_or(|(_, out, _)| out);
        let place = match self.place.filter(|_| out) {
            Some(place) => Some(placed(place.contact, place.edge, size, place.line)),
            None => where_now().map(|(contact, edge)| placed(contact, edge, size, beside(panels, contact.monitor, edge))),
        };
        let Some(place) = place else { return };
        self.place = Some(place);
        if !self.make(place) {
            self.says = None;
            return;
        }
        self.plate = Some(says);
        if !out {
            self.slide(true, now);
        }
    }

    /// Its window made (once), on the device for its screen, placed at `place`: whether it is there to draw on.
    fn make(&mut self, place: Place) -> bool {
        let Ok(gfx) = gfx::for_area(place.window) else { return false };
        if self.drawn.is_none() {
            let Ok(window) = Window::new() else { return false };
            self.drawn = Some(Drawn { window, gfx: gfx.clone(), surface: None, showing: None });
        }
        let drawn = self.drawn.as_mut().unwrap();
        // (What it drew with on another device goes with it: see `life`.)
        if !Rc::ptr_eq(&drawn.gfx, &gfx) {
            drawn.gone();
            drawn.gfx = gfx;
        }
        drawn.window.place(place.window);
        true
    }

    /// Slides out (`out`) or back in, from where it is now.
    fn slide(&mut self, out: bool, now: Instant) {
        self.sliding = Some((self.out, out, now));
        self.stale = true;
    }

    /// Hidden, in the edge, saying nothing.
    fn hide(&mut self) {
        self.says = None;
        self.plate = None;
        self.out = 0.0;
        self.over = None;
        self.pressed = None;
        if let Some(drawn) = self.drawn.as_mut() {
            drawn.window.hide();
            drawn.hidden();
        }
    }

    /// Whether the pointer is on its plate.
    fn pointer_on(&self) -> bool {
        let (Some(place), Some(at)) = (self.place, crate::mouse::cursor_position()) else { return false };
        (place.plate.left..place.plate.right).contains(&at.x) && (place.plate.top..place.plate.bottom).contains(&at.y)
    }

    /// A message of its window: the pointer moved over it, a press on it and let go. What the panel's thread is to do.
    pub fn handle(&mut self, msg: &MSG, now: Instant) -> Option<Act> {
        let at = self.on_plate(msg.lParam);
        match msg.message {
            WM_MOUSEMOVE => {
                let over = at.and_then(|at| self.spot_at(at));
                if over != self.over {
                    self.over = over;
                    self.stale = true;
                }
                None
            }
            WM_LBUTTONDOWN => {
                self.pressed = at.and_then(|at| self.spot_at(at));
                None
            }
            WM_LBUTTONUP => {
                let pressed = self.pressed.take();
                let spot = at.and_then(|at| self.spot_at(at)).filter(|spot| Some(*spot) == pressed)?;
                self.pressed_on(spot, now)
            }
            _ => None,
        }
    }

    /// A press on `spot` let go on it.
    fn pressed_on(&mut self, spot: Spot, now: Instant) -> Option<Act> {
        let says = self.says.clone()?;
        let place = self.place?;
        self.stale = true;
        match (spot, says) {
            (Spot::Trust, Says::Ask(_)) => {
                self.trust.1 = !self.trust.1;
                None
            }
            (Spot::Accept | Spot::Decline, Says::Ask(asking)) => {
                crate::nearby::answer(asking.id, spot == Spot::Accept, spot == Spot::Accept && asking.from.verified && self.trust.1);
                None
            }
            (Spot::Body, Says::Coming(_)) => {
                self.slide(false, now);
                self.says = None;
                Some(open_at(place))
            }
            (Spot::Body, Says::Came { id, .. }) => {
                // The picture carried off it onto the station's face, the panel opened under it.
                let thumb = self.thumb_on_screen(place);
                self.says = None;
                self.until = None;
                self.sliding = None;
                self.flight = Some(Flight { from: thumb, id, began: now, to: None });
                Some(open_at(place))
            }
            _ => None,
        }
    }

    /// Where a point of its window (`lparam`, physical px) is on its plate (DIPs from its corner, as it stands out).
    fn on_plate(&self, lparam: windows::Win32::Foundation::LPARAM) -> Option<(f32, f32)> {
        let place = self.place?;
        let (x, y) = ((lparam.0 & 0xFFFF) as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as i16 as i32);
        let scale = place.contact.scale;
        let (cx, cy) = (place.plate.left - place.window.left, place.plate.top - place.window.top);
        Some(((x - cx) as f32 / scale, (y - cy) as f32 / scale))
    }

    fn spot_at(&self, (x, y): (f32, f32)) -> Option<Spot> {
        self.spots.iter().rev().find(|(sx, sy, w, h, _)| x >= *sx && x < sx + w && y >= *sy && y < sy + h).map(|spot| spot.4)
    }

    /// Where its picture is on the screen (physical px), at rest.
    fn thumb_on_screen(&self, place: Place) -> RECT {
        let px = |dips: f32| (dips * place.contact.scale).round() as i32;
        let (left, top) = (place.plate.left + px(PAD), place.plate.top + px(PAD));
        RECT { left, top, right: left + px(THUMB), bottom: top + px(THUMB) }
    }

    /// Draws what it says, as far out as it is: whether it drew all of it (its picture made).
    fn draw(&mut self) -> bool {
        let (Some(says), Some(place)) = (self.plate.clone(), self.place) else { return true };
        let Some(drawn) = self.drawn.as_mut() else { return true };
        let (skin, dark, lang) = look();
        let theme = Theme::new(skin, dark);
        let scale = place.contact.scale;
        let size = ((place.window.right - place.window.left) as u32, (place.window.bottom - place.window.top) as u32);
        let corner = ((place.plate.left - place.window.left) as f32 / scale, (place.plate.top - place.window.top) as f32 / scale);
        // (Slid back by as much as it is not out, toward its edge, past which it is cut off.)
        let back = (1.0 - self.out) * match place.edge {
            Edge::Left => -((place.plate.right - place.window.left) as f32 / scale + ROOM),
            Edge::Right => (place.window.right - place.plate.left) as f32 / scale + ROOM,
            Edge::Top => -((place.plate.bottom - place.window.top) as f32 / scale + ROOM),
        };
        let (dx, dy) = if place.edge == Edge::Top { (0.0, back) } else { (back, 0.0) };
        let surface = match drawn.surface.take() {
            Some(surface) => Ok(surface),
            None => Surface::new(&drawn.gfx, drawn.window.hwnd),
        };
        let Ok(mut surface) = surface else { return true };
        drawn.shown();
        let mut whole = true;
        let mut spots = Vec::new();
        let (over, trust) = (self.over, self.trust.1);
        let done = surface.draw(&drawn.gfx, size, scale, |frame| {
            frame.origin(corner.0 + dx, corner.1 + dy);
            whole = paint(frame, &theme, lang, &says, (over, trust), &mut spots);
            frame.origin(0.0, 0.0);
        });
        if let Err(error) = &done {
            if !gfx::failed("nearby notice", &drawn.gfx, Some(error)) {
                drawn.surface = Some(surface);
            }
            return true;
        }
        drawn.surface = Some(surface);
        self.spots = spots;
        drawn.window.show();
        whole
    }

    /// The picture carried onto the station's face: from where it was in the notice to where the panel drew the face,
    /// followed as the panel comes out (it moves); landed, gone, the face coming in as it lands. Where no panel draws
    /// it, it fades where it is.
    fn fly(&mut self, now: Instant, seen: &Seen) -> Option<Duration> {
        let flight = self.flight.as_mut()?;
        let t = (now.saturating_duration_since(flight.began).as_secs_f32() / FLIGHT.as_secs_f32()).min(1.0);
        if let Some(to) = (seen.face)(flight.id) {
            // (Found: its face kept from showing till the picture is nearly there, then coming in under it.)
            if flight.to.is_none() {
                let lands = flight.began + FLIGHT;
                let comes = Duration::from_secs_f32(crate::station::run::ARRIVED_FOR.as_secs_f32() * 0.3);
                crate::station::run::ask(crate::station::run::Ask::Landing(vec![flight.id], lands.checked_sub(comes).unwrap_or(now).max(now)));
            }
            flight.to = Some(to);
        }
        let (id, from, to) = (flight.id, flight.from, flight.to);
        if t >= 1.0 {
            self.flight = None;
            self.hide();
            return None;
        }
        let eased = super::morph::ease(t);
        let rect = match to {
            Some(to) => {
                let mix = |a: i32, b: i32| a + ((b - a) as f32 * eased).round() as i32;
                RECT { left: mix(from.left, to.left), top: mix(from.top, to.top), right: mix(from.right, to.right), bottom: mix(from.bottom, to.bottom) }
            }
            None => from,
        };
        // (Nowhere to land: fading where it was.)
        let opacity = if to.is_some() { 1.0 } else { 1.0 - t };
        let Some(drawn) = self.drawn.as_mut() else { return None };
        let scale = self.place.map_or(1.0, |place| place.contact.scale);
        let size = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        let surface = match drawn.surface.take() {
            Some(surface) => Ok(surface),
            None => Surface::new(&drawn.gfx, drawn.window.hwnd),
        };
        let Ok(mut surface) = surface else { return Some(Duration::ZERO) };
        let (skin, dark, lang) = look();
        let theme = Theme::new(skin, dark);
        let (w, h) = (size.0 as f32 / scale, size.1 as f32 / scale);
        let done = surface.draw(&drawn.gfx, size, scale, |frame| {
            let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
            let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
            unsafe { frame.dc.PushLayer(&layer, None) };
            super::view::paint_kept_face(frame, &theme, id, (0.0, 0.0, w, h), 1.0, lang);
            unsafe { frame.dc.PopLayer() };
        });
        if done.is_ok() {
            drawn.surface = Some(surface);
            drawn.window.place(rect);
            drawn.window.show();
        } else if !gfx::failed("nearby notice", &drawn.gfx, done.as_ref().err()) {
            drawn.surface = Some(surface);
        }
        Some(Duration::ZERO)
    }

    /// The screens changed: where it stands found again for what it says (its screen may be gone).
    pub fn screens_changed(&mut self) {
        if let (Some(says), Some(place)) = (self.plate.clone(), self.place) {
            let size = (WIDE, tall(&says));
            match crate::mouse::monitor_at(POINT { x: place.plate.left, y: place.plate.top }) {
                Some(contact) => {
                    let place = placed(contact, place.edge, size, None);
                    self.place = Some(place);
                    if self.make(place) {
                        self.stale = true;
                    }
                }
                None => self.hide(),
            }
        }
    }
}

/// What came, as the thing `id` on the station is, told (none: not on it, or not from a device).
fn came_of(items: &[crate::station::shelf::Item], id: u64) -> Option<Says> {
    let item = items.iter().find(|item| item.id == id)?;
    let crate::station::shelf::What::Came { files } = &item.what else { return None };
    let from = item.from.as_ref()?;
    let pictures = files.iter().all(|file| is_picture(&file.name));
    Some(Says::Came { id, from: from.alias.clone(), kind: from.kind.clone(), files: files.len(), pictures })
}

/// Whether a file named `name` is a picture, by its extension.
fn is_picture(name: &str) -> bool {
    let extension = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "tif", "tiff", "avif", "jfif", "dng"].contains(&extension.as_str())
}

/// Whether what is offered is all pictures (by its type, else its name).
fn all_pictures(files: &[Offered]) -> bool {
    !files.is_empty() && files.iter().all(|file| file.kind.starts_with("image/") || is_picture(&file.name))
}

/// The skin, light or dark, and the language Glance is drawn in now.
fn look() -> (Skin, bool, Lang) {
    let settings = crate::app().settings.lock().unwrap();
    let pref = serde_json::from_value(settings.view["theme"].clone()).unwrap_or_default();
    (Skin::named(&settings.skin), super::theme::is_dark(pref, None), crate::update::language_in(&settings.view))
}

/// The screen the notice stands on (the pointer's) and the edge it slides out of there: the one that opens the
/// workbench (where the station is), else what a panel opened by hand opens at.
fn where_now() -> Option<(Contact, Edge)> {
    let at = crate::mouse::cursor_position()?;
    let contact = crate::mouse::monitor_at(at)?;
    let places = crate::app().controller.edges().places;
    Some((contact, edge_on(&places, crate::screens::id_of(contact.monitor).as_deref())))
}

/// The edge of the screen named `screen` that opens the workbench, else the one a panel opened by hand opens at.
fn edge_on(places: &Places, screen: Option<&str>) -> Edge {
    screen
        .and_then(|screen| places.holds(screen).iter().find(|hold| hold.part == Part::Workbench).map(|hold| hold.edge))
        .unwrap_or_else(|| places.by_hand().0)
}

/// The line a notice at `edge` of the screen `monitor` slides out of where a panel is up there (of `panels`): the
/// panel's inner side.
fn beside(panels: &[(RECT, Edge, RECT)], monitor: RECT, edge: Edge) -> Option<i32> {
    let (.., panel) = panels.iter().find(|(on, at, _)| *on == monitor && *at == edge)?;
    Some(match edge {
        Edge::Left => panel.right,
        Edge::Right => panel.left,
        Edge::Top => panel.bottom,
    })
}

/// Where a plate `size` large (DIPs) stands on `contact`'s screen at `edge`: its plate away from the edge by `GAP` (from
/// `line`, where it is not the edge it slides out of: a panel's side), along it from `ABOVE_MIDDLE` over its middle,
/// whatever its height (it grows down as it says more: the top edge's, under it, in its middle); its window round it,
/// on to that line.
fn placed(contact: Contact, edge: Edge, (wide, tall): (f32, f32), line: Option<i32>) -> Place {
    let (work, scale) = (contact.work, contact.scale);
    let px = |dips: f32| (dips * scale).round() as i32;
    let (w, h, gap, room) = (px(wide), px(tall), px(GAP), px(ROOM));
    let from = line.unwrap_or(match edge {
        Edge::Left => work.left,
        Edge::Right => work.right,
        Edge::Top => work.top,
    });
    let (left, top) = match edge {
        Edge::Left => (from + gap, (work.top + work.bottom) / 2 - px(ABOVE_MIDDLE)),
        Edge::Right => (from - gap - w, (work.top + work.bottom) / 2 - px(ABOVE_MIDDLE)),
        Edge::Top => ((work.left + work.right - w) / 2, from + gap),
    };
    let plate = RECT { left, top, right: left + w, bottom: top + h };
    let mut window = RECT { left: left - room, top: top - room, right: left + w + room, bottom: top + h + room };
    match edge {
        Edge::Left => window.left = from,
        Edge::Right => window.right = from,
        Edge::Top => window.top = from,
    }
    Place { contact, edge, plate, window, line }
}

/// The panel opened where the notice stands: at its edge, along it at its middle.
fn open_at(place: Place) -> Act {
    let at = POINT { x: (place.plate.left + place.plate.right) / 2, y: (place.plate.top + place.plate.bottom) / 2 };
    let at = match place.edge {
        Edge::Left => POINT { x: place.contact.monitor.left, y: at.y },
        Edge::Right => POINT { x: place.contact.monitor.right - 1, y: at.y },
        Edge::Top => POINT { x: at.x, y: place.contact.monitor.top },
    };
    Act::Open { edge: place.edge, at }
}

/// How tall its plate is for `says` (DIPs).
fn tall(says: &Says) -> f32 {
    match says {
        Says::Ask(asking) => {
            let listed = asking.files.len().min(LISTED);
            let more = if asking.files.len() > LISTED { MORE_ROW } else { 0.0 };
            let trust = if asking.from.verified { TRUST_ROW + SPACE } else { 0.0 };
            PAD + HEAD + SPACE + listed as f32 * FILE_ROW + more + SPACE + trust + BUTTON + PAD
        }
        Says::Coming(_) => PAD + THUMB + SPACE + BAR + PAD,
        Says::Came { .. } => PAD + THUMB + PAD,
    }
}

/// How large `bytes` is, said.
fn size_of_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else if size < 10.0 { format!("{size:.1} {}", UNITS[unit]) } else { format!("{size:.0} {}", UNITS[unit]) }
}

/// How many files (all pictures: photos) there are, said.
fn counted(count: usize, pictures: bool, lang: Lang) -> String {
    match (lang, pictures) {
        (Lang::Zh, true) => format!("{count} 张照片"),
        (Lang::Zh, false) => format!("{count} 个文件"),
        (Lang::En, true) if count == 1 => "1 photo".to_string(),
        (Lang::En, true) => format!("{count} photos"),
        (Lang::En, false) if count == 1 => "1 file".to_string(),
        (Lang::En, false) => format!("{count} files"),
    }
}

/// What it says as words: its title and the line under it (an asking's are its device's name and what it would send).
fn words(says: &Says, lang: Lang) -> (String, String) {
    match says {
        Says::Ask(asking) => {
            let what = counted(asking.files.len(), all_pictures(&asking.files), lang);
            (asking.from.alias.clone(), match lang {
                Lang::Zh => format!("想发送 {what}"),
                Lang::En => format!("Wants to send {what}"),
            })
        }
        Says::Coming(receiving) => {
            let what = counted(receiving.files.len(), all_pictures(&receiving.files), lang);
            let of = size_of_bytes(receiving.of);
            match lang {
                Lang::Zh => (format!("正在接收 {what}"), format!("来自 {} · {of}", receiving.from.alias)),
                Lang::En => (format!("Receiving {what}"), format!("From {} · {of}", receiving.from.alias)),
            }
        }
        Says::Came { from, files, pictures, .. } => {
            let what = counted(*files, *pictures, lang);
            match lang {
                Lang::Zh => (format!("收到 {what}"), format!("来自 {from} · 已放入中转站")),
                Lang::En => (format!("Received {what}"), format!("From {from} · In the station")),
            }
        }
    }
}

/// Draws `says` on its plate from the frame's origin, `over` the part the pointer is on and the asking's device to be
/// trusted or not (`trust`); its parts a press does something on put in `spots`. Whether all of it was drawn (its
/// picture made).
fn paint(frame: &Frame, theme: &Theme, lang: Lang, says: &Says, (over, trust): (Option<Spot>, bool), spots: &mut Vec<(f32, f32, f32, f32, Spot)>) -> bool {
    let canvas: &dyn Canvas = frame;
    let height = tall(says);
    let plate = super::menu::plate(canvas, super::menu::Look::of(theme.skin, theme.dark), (0.0, 0.0, WIDE, height));
    let accent = match theme.skin {
        Skin::Paper => theme.text,
        _ => super::theme::accent(theme.dark),
    };
    let ink = theme.ink("station");
    let (title, under) = words(says, lang);
    let inner = WIDE - 2.0 * PAD;
    let title_font = super::canvas::Font { weight: 600.0, ..theme.body };
    let mut whole = true;
    match says {
        Says::Ask(asking) => {
            // The device: its mark, its name, what it would send.
            let centre = Point { x: PAD + HEAD / 2.0, y: PAD + HEAD / 2.0 };
            canvas.fill_circle(theme.track, centre, HEAD / 2.0);
            canvas.icon(super::view::device_mark(asking.from.kind.as_deref()), centre, 17.0, plate.icon);
            let left = PAD + HEAD + 10.0;
            canvas.text(&title, title_font, theme.text, left, PAD + 1.0, inner - HEAD - 10.0, Align::Start);
            canvas.text(&under, theme.small, theme.text2, left, PAD + 19.0, inner - HEAD - 10.0, Align::Start);
            // What it would send: each file on a row of its own (its mark, its name, its size), the rest counted.
            let mut y = PAD + HEAD + SPACE;
            for file in asking.files.iter().take(LISTED) {
                canvas.fill_rounded(theme.track, PAD, y + 2.0, inner, FILE_ROW - 4.0, 8.0);
                let mark = if file.kind.starts_with("image/") || is_picture(&file.name) { Icon::Image } else { Icon::File };
                canvas.icon(mark, Point { x: PAD + 15.0, y: y + FILE_ROW / 2.0 }, 14.0, theme.text2);
                let size = size_of_bytes(file.size);
                let size_w = canvas.measure(&size, theme.small) + 1.0;
                let middle = canvas.line_in(theme.small, y, FILE_ROW);
                canvas.text(&file.name, theme.small, theme.text, PAD + 30.0, middle, inner - 30.0 - size_w - 16.0, Align::Start);
                canvas.text(&size, theme.small, theme.text3, PAD + inner - 8.0 - size_w, middle, size_w, Align::Start);
                y += FILE_ROW;
            }
            if asking.files.len() > LISTED {
                let more = asking.files.len() - LISTED;
                let said = match lang {
                    Lang::Zh => format!("以及另外 {more} 个文件"),
                    Lang::En => format!("and {more} more"),
                };
                canvas.text(&said, theme.small, theme.text3, PAD + 2.0, canvas.line_in(theme.small, y, MORE_ROW), inner, Align::Start);
                y += MORE_ROW;
            }
            y += SPACE;
            // Whether to take its files from now on without asking: only for a device that showed its certificate
            // (one that did not is not known to be itself the next time).
            if asking.from.verified {
                let (bx, by, side) = (PAD + 1.0, y + (TRUST_ROW - 15.0) / 2.0, 15.0);
                if trust {
                    canvas.fill_rounded(accent, bx, by, side, side, 4.0);
                    canvas.icon(Icon::Check, Point { x: bx + side / 2.0, y: by + side / 2.0 }, 11.0, super::view::ink_on(accent));
                } else {
                    canvas.stroke_rounded(super::canvas::Fill::Solid(theme.text3), bx + 0.75, by + 0.75, side - 1.5, side - 1.5, 3.5, 1.5);
                }
                let said = lang.pick("以后直接接收这台设备的文件", "Accept files from this device without asking");
                canvas.text(said, theme.small, if over == Some(Spot::Trust) { theme.text } else { theme.text2 }, bx + side + 8.0, canvas.line_in(theme.small, y, TRUST_ROW), inner - side - 9.0, Align::Start);
                spots.push((PAD, y, inner, TRUST_ROW, Spot::Trust));
                y += TRUST_ROW + SPACE;
            }
            // Decline, Accept.
            let wide = (inner - 8.0) / 2.0;
            for (at, spot, said) in [(PAD, Spot::Decline, lang.pick("拒绝", "Decline")), (PAD + wide + 8.0, Spot::Accept, lang.pick("接收", "Accept"))] {
                let lit = over == Some(spot);
                let (ground, words) = if spot == Spot::Accept { (accent, super::view::ink_on(accent)) } else { (theme.track, theme.text) };
                let ground = if lit { ground.alpha((ground.a * 0.82).max(0.0)) } else { ground };
                canvas.fill_rounded(ground, at, y, wide, BUTTON, theme.control_radius.max(6.0));
                if lit && spot == Spot::Decline {
                    canvas.fill_rounded(plate.highlight, at, y, wide, BUTTON, theme.control_radius.max(6.0));
                }
                let said_w = canvas.measure(said, title_font);
                canvas.text(said, title_font, words, at + (wide - said_w) / 2.0, canvas.line_in(title_font, y, BUTTON), said_w + 1.0, Align::Start);
                spots.push((at, y, wide, BUTTON, spot));
            }
        }
        Says::Coming(receiving) => {
            // A picture's mark where the picture will be, and a bar of how much has come.
            canvas.fill_rounded(theme.track, PAD, PAD, THUMB, THUMB, 10.0);
            canvas.icon(Icon::Image, Point { x: PAD + THUMB / 2.0, y: PAD + THUMB / 2.0 }, 20.0, theme.text3);
            text_beside(canvas, theme, title_font, &title, &under);
            let done = if receiving.of == 0 { 0.0 } else { (receiving.done as f64 / receiving.of as f64).clamp(0.0, 1.0) as f32 };
            let y = PAD + THUMB + SPACE;
            canvas.fill_rounded(theme.track, PAD, y, inner, BAR, BAR / 2.0);
            if done > 0.0 {
                canvas.fill_rounded(ink.trace, PAD, y, (inner * done).max(BAR), BAR, BAR / 2.0);
            }
            spots.push((0.0, 0.0, WIDE, height, Spot::Body));
        }
        Says::Came { id, .. } => {
            whole = super::view::paint_kept_face(canvas, theme, *id, (PAD, PAD, THUMB, THUMB), 1.0, lang).unwrap_or(true);
            text_beside(canvas, theme, title_font, &title, &under);
            spots.push((0.0, 0.0, WIDE, height, Spot::Body));
        }
    }
    // (Lit as the pointer is on it, where all of it is pressed.)
    if over == Some(Spot::Body) {
        canvas.fill_rounded(plate.highlight, 0.0, 0.0, WIDE, height, plate.radius);
    }
    whole
}

/// A title and the line under it beside the picture.
fn text_beside(canvas: &dyn Canvas, theme: &Theme, title_font: super::canvas::Font, title: &str, under: &str) {
    let left = PAD + THUMB + 12.0;
    let room = WIDE - left - PAD;
    let top = PAD + (THUMB - 36.0) / 2.0;
    canvas.text(title, title_font, theme.text, left, top, room, Align::Start);
    canvas.text(under, theme.small, theme.text2, left, top + 19.0, room, Align::Start);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(verified: bool) -> crate::nearby::Sender {
        crate::nearby::Sender { alias: "Z Fold".into(), model: None, kind: Some("mobile".into()), fingerprint: "AB".into(), verified, address: "10.0.0.5".parse().unwrap() }
    }

    fn offered(name: &str, kind: &str, size: u64) -> Offered {
        Offered { name: name.into(), size, kind: kind.into() }
    }

    /// What it says, in Chinese and in English: a device's asking by its name and what it would send (photos where
    /// all are pictures), what comes and what came, from whom.
    #[test]
    fn it_says_what_comes_from_whom() {
        let photo = offered("IMG_1.jpg", "image/jpeg", 3_400_000);
        let asking = Asking { id: 1, from: sender(true), files: vec![photo.clone()] };
        assert_eq!(words(&Says::Ask(asking), Lang::Zh), ("Z Fold".to_string(), "想发送 1 张照片".to_string()));
        let mixed = Asking { id: 2, from: sender(false), files: vec![photo.clone(), offered("纪要.docx", "", 86_000)] };
        assert_eq!(words(&Says::Ask(mixed), Lang::En).1, "Wants to send 2 files");
        let coming = Receiving { session: "s".into(), from: sender(true), files: vec![photo], done: 0, of: 3_400_000 };
        assert_eq!(words(&Says::Coming(coming), Lang::Zh), ("正在接收 1 张照片".to_string(), "来自 Z Fold · 3.2 MB".to_string()));
        let came = Says::Came { id: 4, from: "Z Fold".into(), kind: None, files: 3, pictures: true };
        assert_eq!(words(&came, Lang::Zh), ("收到 3 张照片".to_string(), "来自 Z Fold · 已放入中转站".to_string()));
        assert_eq!(words(&came, Lang::En).0, "Received 3 photos");
    }

    /// Sizes said as Explorer says them, a decimal under ten.
    #[test]
    fn sizes_are_said_shortly() {
        assert_eq!(size_of_bytes(512), "512 B");
        assert_eq!(size_of_bytes(86_000), "84 KB");
        assert_eq!(size_of_bytes(3_400_000), "3.2 MB");
        assert_eq!(size_of_bytes(5 << 30), "5.0 GB");
    }

    /// An asking is as tall as what it lists: three files at most, a line for the rest, the box to trust the device
    /// only for one that showed its certificate.
    #[test]
    fn an_asking_is_as_tall_as_what_it_lists() {
        let files = |count: usize| (0..count).map(|at| offered(&format!("{at}.jpg"), "image/jpeg", 1)).collect::<Vec<_>>();
        let ask = |count: usize, verified: bool| tall(&Says::Ask(Asking { id: 1, from: sender(verified), files: files(count) }));
        assert_eq!(ask(2, true) - ask(1, true), FILE_ROW);
        assert_eq!(ask(4, true) - ask(3, true), MORE_ROW);
        assert_eq!(ask(9, true), ask(4, true));
        assert_eq!(ask(1, true) - ask(1, false), TRUST_ROW + SPACE);
    }

    /// It stands at the edge that opens the workbench on its screen, its plate off the edge by its gap, in the middle
    /// along it (under the top edge), its window on to the edge it slides out of, room round the rest for its shadow.
    #[test]
    fn it_stands_at_the_workbench_s_edge() {
        let contact = Contact { monitor: RECT { left: 0, top: 0, right: 2560, bottom: 1440 }, work: RECT { left: 0, top: 0, right: 2560, bottom: 1392 }, scale: 1.5 };
        let place = placed(contact, Edge::Right, (WIDE, 100.0), None);
        assert_eq!(place.plate, RECT { left: 2560 - 18 - 432, top: 1392 / 2 - 90, right: 2560 - 18, bottom: 1392 / 2 - 90 + 150 });
        assert_eq!((place.window.right, place.window.left, place.window.top), (2560, place.plate.left - 24, place.plate.top - 24));
        let place = placed(contact, Edge::Left, (WIDE, 100.0), None);
        assert_eq!((place.plate.left, place.window.left, place.window.right), (18, 0, 18 + 432 + 24));
        let place = placed(contact, Edge::Top, (WIDE, 100.0), None);
        assert_eq!((place.plate.top, place.window.top, place.plate.left), (18, 0, (2560 - 432) / 2));
        let mut places = Places::default();
        assert_eq!(edge_on(&places, Some("screen")), Edge::Left);
        places.every.retain(|hold| hold.part != Part::Workbench);
        assert_eq!(edge_on(&places, Some("screen")), places.by_hand().0);
        // Pressed, it opens the panel at its edge, along it where it stands.
        let place = placed(contact, Edge::Right, (WIDE, 100.0), None);
        assert_eq!(open_at(place), Act::Open { edge: Edge::Right, at: POINT { x: 2559, y: (place.plate.top + place.plate.bottom) / 2 } });
        // A panel up at that edge (asked meanwhile): out of the panel's side, beside it, not over it; a panel at
        // another edge, or on another screen, changes nothing.
        let panel = RECT { left: 2560 - 500, top: 100, right: 2560, bottom: 1300 };
        let other = RECT { left: 2560, top: 0, right: 5120, bottom: 1440 };
        let panels = [(contact.monitor, Edge::Right, panel), (contact.monitor, Edge::Left, RECT { left: 0, top: 0, right: 400, bottom: 900 }), (other, Edge::Right, panel)];
        assert_eq!(beside(&panels, contact.monitor, Edge::Right), Some(2060));
        assert_eq!(beside(&panels, contact.monitor, Edge::Top), None);
        let place = placed(contact, Edge::Right, (WIDE, 100.0), beside(&panels, contact.monitor, Edge::Right));
        assert_eq!((place.plate.right, place.window.right), (2060 - 18, 2060));
    }
}

#[cfg(all(test, feature = "studio"))]
mod pictures {
    use super::*;

    /// The notice as it asks (a device that showed its certificate, with the box ticked and Accept under the pointer;
    /// one that did not, sending many files), as what comes comes, and as what came, in each skin, light and dark, over
    /// a desktop's blue (`cargo test --features studio --lib -- --ignored peek_states` writes
    /// `target/peek/<skin>-<dark>-<state>.png`).
    #[test]
    #[ignore = "writes pictures to look at"]
    fn peek_states() {
        let _pretending = crate::pretending();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        crate::station::thumbs::start(|| {});
        use crate::station::shelf::{Device, Item, What};
        let photo = std::path::PathBuf::from(r"C:\Windows\Web\Screen\img100.jpg");
        let from = Device { alias: "Galaxy Z Fold6".into(), kind: Some("mobile".into()), fingerprint: "AB".into() };
        let item = Item { id: 7, what: What::Files { paths: vec![photo], kept: Vec::new() }, added_ms: crate::station::now_ms(), pinned: false, copied: false, stays_here: false, own_bytes: 0, from: Some(from) };
        crate::station::run::pretend(vec![item], crate::settings::ShelfView::Cards);
        let sender = |verified: bool| crate::nearby::Sender { alias: "Galaxy Z Fold6".into(), model: None, kind: Some("mobile".into()), fingerprint: "AB".into(), verified, address: "10.0.0.5".parse().unwrap() };
        let offered = |name: &str, kind: &str, size: u64| Offered { name: name.into(), size, kind: kind.into() };
        let photo = offered("IMG_20261112_143012.jpg", "image/jpeg", 3_500_000);
        let many = vec![photo.clone(), offered("会议纪要.docx", "", 86_000), offered("第三季度报表.xlsx", "", 1_200_000), offered("IMG_2.jpg", "image/jpeg", 2_900_000), offered("录音.m4a", "audio/mp4", 8_400_000)];
        let states = [
            ("ask", Says::Ask(Asking { id: 1, from: sender(true), files: vec![photo.clone()] }), (Some(Spot::Accept), true)),
            ("ask-many", Says::Ask(Asking { id: 2, from: sender(false), files: many }), (None, false)),
            ("coming", Says::Coming(Receiving { session: "s".into(), from: sender(true), files: vec![photo.clone()], done: 1_300_000, of: 3_500_000 }), (None, false)),
            ("came", Says::Came { id: 7, from: "Galaxy Z Fold6".into(), kind: Some("mobile".into()), files: 1, pictures: true }, (Some(Spot::Body), false)),
        ];
        let gfx = gfx::for_main_screen().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/peek", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let scale = 1.5;
        for pass in 0..2 {
            for (skin, name) in [(Skin::Glass, "glass"), (Skin::Fluent, "fluent"), (Skin::Paper, "paper")] {
                for dark in [false, true] {
                    let theme = Theme::new(skin, dark);
                    for (label, says, state) in &states {
                        let (w, h) = (WIDE + 2.0 * ROOM, tall(says) + 2.0 * ROOM);
                        let size = ((w * scale) as u32, (h * scale) as u32);
                        let pixels = gfx
                            .draw_offscreen(size, scale, |frame| {
                                frame.fill(super::super::canvas::Color::hex(0x1E5AA8, 1.0), 0.0, 0.0, w, h);
                                frame.origin(ROOM, ROOM);
                                paint(frame, &theme, Lang::Zh, says, *state, &mut Vec::new());
                            })
                            .unwrap();
                        if pass == 1 {
                            crate::studio::save_png(&out.join(format!("{name}-{}-{label}.png", if dark { "dark" } else { "light" })), size.0, size.1, &pixels).unwrap();
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
        // (Its device let go of before the thread ends: else it hung there.)
        drop(gfx);
        gfx::let_go();
    }
}
