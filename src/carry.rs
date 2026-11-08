//! A thing dragged off a station's lane into another program (a thing on it, copied lately, a file come lately),
//! carried on the screen by Glance as a note pulled off the panel is (see `widget::Pulled`): lifted off its room,
//! leaning as it is moved, in a window of its own over every other that the pointer passes through to what is under
//! it; what the program under the pointer says a drop does, said under it; let go where it is taken, given over there;
//! let go where it is not, or given up, back into its room. The drag itself is Windows' (see `station::drag`), which
//! tells on as it goes.
//!
//! Things chosen together and dragged off together (see `station::select`) are carried as one stack: each flies from its
//! own room into its place, the one pressed on top and up to two others behind it, each a little up, to the right and
//! turned, and on their corner how many files land where they are let go (see `paint_stack`). Let go where nothing takes
//! them, each flies back apart into its own room, one after another, drawn as its room draws it by the time it lands
//! there (see `Flight`, `paint_tile`). Taken, they go in together.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows_numerics::{Matrix3x2, Vector2};

use crate::station::drag::{Dragged, Over};
use crate::ui::canvas::{Align, Canvas, Color, Point};
use crate::ui::gfx::{Frame, Gfx, Surface};
use crate::ui::morph::Part;
use crate::ui::motion::Spring;
use crate::ui::view::Hit;
use crate::ui::window::Window;
use crate::widget::{faded, lifted_shadow, GOING_BACK, LEANING, LEAN_PULLED, LEAN_SPAN, LIFTED_BY, LIFTING};

/// A thing carried as it was drawn on its lane: what it is pressed as, what is drawn of it there at rest (from its
/// box's corner, DIPs), what its room draws of it where that is otherwise (its mark chosen with others), its box's size,
/// and its box's corner on the screen as it was lifted (physical px): its room.
pub struct Tile {
    pub hit: Hit,
    pub parts: Vec<Part>,
    pub home: Option<Vec<Part>>,
    pub size: (f32, f32),
    pub at: POINT,
}

/// What is lifted off a station's lane to be carried: all that is carried (the one pressed first, then what is chosen
/// with it: see `station::select`), how many files land where they are let go, the drag that carries them off (see
/// `station::drag::announce`), those of them drawn there as they were (`tiles`: the one pressed first, then those of
/// the others whose rooms show, in that order, `FLOWN` at most), where it was pressed (physical px, `px` a DIP), what it was lifted off
/// (the panel, a widget: let go over it, it goes back) and that window, and the look of what it was lifted off (its
/// plates').
pub struct Lifted {
    pub carried: Vec<Hit>,
    pub count: usize,
    pub drag: u64,
    pub tiles: Vec<Tile>,
    pub from: POINT,
    pub px: f32,
    pub over: RECT,
    pub window: HWND,
    pub look: crate::ui::menu::Look,
}

/// The room round what is carried on its plate, and its window's room round all it draws for their shadows and
/// leaning, and under it for what a drop does (DIPs).
pub(crate) const PAD: f32 = 8.0;
const ROOM: f32 = 40.0;
const SAYS_GAP: f32 = 10.0;
const SAYS_TALL: f32 = 28.0;
/// How lifted it is over what it was lifted off (let go there, it goes back), and how much of it shows over a program
/// that would not take it.
const HOME_LIFT: f32 = 0.5;
const REFUSED: f32 = 0.55;
/// Carried with others: how many plates show behind it at most, how far each is from the one before it (up and to the
/// right, DIPs) and how far turned (degrees), and how much of each shows. The count of files on their corner: how tall,
/// how far in from the backmost plate's corner (across, up), and the room round its figure (DIPs).
const BEHIND_MOST: usize = 2;
const BEHIND_STEP: f32 = 9.0;
const BEHIND_TURN: f32 = 4.5;
const BEHIND_SEEN: [f32; BEHIND_MOST] = [0.85, 0.7];
const COUNT_TALL: f32 = 22.0;
const COUNT_AT: (f32, f32) = (12.0, 10.0);
const COUNT_PAD: f32 = 7.0;
/// How many of the things carried together show in the stack: the one on top and those behind it.
const SHOWN: usize = BEHIND_MOST + 1;
/// How many of the things carried together fly from their rooms and back at most: the rest are outlined there while
/// they are carried, and drawn full again as the last lands.
pub const FLOWN: usize = 30;
/// How near its room a thing flying back is (DIPs) as it lands there, and its room is drawn full again, and how much of
/// its way it has left at most then (of its lift, larger, turned, its shadow and plate: none of it seen).
const LANDS_NEAR: f32 = 0.2;
const LANDS_LIFTED: f32 = 0.005;
/// Let go where nothing takes them, how long after the one before it each sets off back (seconds), and how long after
/// the first the last sets off at most (closer together where there are many).
const STAGGER: f32 = 0.03;
const STAGGER_ALL: f32 = 0.15;
/// How it fades as the program under it changes, and what a drop does comes and goes (stiffness, damping: see `Spring`).
const FADING: (f32, f32) = (400.0, 36.0);
/// Given over where it is let go: how long it takes, and how large it is by its end.
const GIVEN: Duration = Duration::from_millis(180);
const GIVEN_TO: f32 = 0.6;

thread_local! {
    /// What is carried now, on the panel's thread.
    static CARRIED: RefCell<Option<Carry>> = const { RefCell::new(None) };
}

struct Carry {
    lifted: Lifted,
    window: Window,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    /// How lifted it is (0 at rest, 1 in hand), how it leans (degrees), how much of it shows, how much of what a drop
    /// does is said, and that.
    lift: Spring,
    lean: Spring,
    seen: Spring,
    saying: Spring,
    says: String,
    /// The pointer lately (when, where), for how fast it goes across.
    trail: Vec<(Instant, POINT)>,
    /// The box of the one pressed, as it is held: its corner on the screen (physical px), where the stack stands.
    corner: (f32, f32),
    /// Each thing drawn on its way between its room and its place in the stack, and how far apart those are (DIPs, as
    /// it was last drawn).
    flight: Flight,
    apart: Vec<f32>,
    /// Given over where it was let go: since when.
    given: Option<Instant>,
    last: Instant,
}

/// What `lifted` is carried off its lane from here (one carried still, put back first), drawn on `gfx`'s device: each of
/// its tiles flying from its room into its place in the stack.
pub fn begin(gfx: &Rc<Gfx>, lifted: Lifted) {
    CARRIED.with_borrow_mut(|carried| {
        if let Some(was) = carried.take() {
            crate::ui::view::land_tiles(was.lifted.carried, Instant::now());
        }
        let Ok(mut window) = Window::new() else { return };
        window.set_click_through(true);
        // (Never in what a widget sees behind it: it is in hand, not part of the desktop.)
        crate::ui::scene::leave_out(window.hwnd, true);
        let surface = Surface::new(gfx, window.hwnd).ok();
        crate::ui::view::press_tile(None);
        crate::ui::view::carry_tiles(lifted.carried.clone());
        let corner = (lifted.tiles[0].at.x as f32, lifted.tiles[0].at.y as f32);
        let mut lift = Spring::at(0.0, LIFTING.0, LIFTING.1);
        lift.target = 1.0;
        let flight = Flight::lifted(lifted.tiles.len());
        // (How far each is from its room is known as it is drawn: till then, none lands.)
        let apart = vec![f32::INFINITY; lifted.tiles.len()];
        *carried = Some(Carry {
            lifted,
            window,
            gfx: gfx.clone(),
            surface,
            lift,
            lean: Spring::at(0.0, LEANING.0, LEANING.1),
            seen: Spring::at(1.0, FADING.0, FADING.1),
            saying: Spring::at(0.0, FADING.0, FADING.1),
            says: String::new(),
            trail: Vec::new(),
            corner,
            flight,
            apart,
            given: None,
            last: Instant::now(),
        });
    });
}

/// What is carried a step on to `now`, drawn: when next to (none, nothing carried).
pub fn tick(now: Instant) -> Option<Duration> {
    CARRIED.with_borrow_mut(|carried| {
        let carry = carried.as_mut()?;
        if carry.step(now) {
            Some(Duration::ZERO)
        } else {
            crate::ui::scene::leave_out(carry.window.hwnd, false);
            *carried = None;
            None
        }
    })
}

/// What it wants as the pointer is over what it was lifted off (`home`), or else over a program that would take it or
/// not, saying what a drop does or not (see `Over`): how lifted, how much of it shows, whether what a drop does is said.
fn wants(home: bool, over: &Over) -> (f32, f32, bool) {
    match home {
        true => (HOME_LIFT, 1.0, false),
        false => (1.0, if over.takes { 1.0 } else { REFUSED }, over.says.is_some()),
    }
}

/// How far it leans (degrees), moved across as `trail` says (when, where: physical px) at `px` a DIP lately (see
/// `LEAN_SPAN`), as a note pulled off the panel does.
fn lean_of(trail: &[(Instant, POINT)], px: f32, now: Instant) -> f32 {
    let recent: Vec<&(Instant, POINT)> = trail.iter().filter(|(at, _)| now.saturating_duration_since(*at) <= LEAN_SPAN).collect();
    let across = match (recent.first(), recent.last()) {
        (Some(first), Some(last)) if last.0 > first.0 => (last.1.x - first.1.x) as f32 / px / (last.0.duration_since(first.0).as_secs_f32() * 1000.0),
        _ => 0.0,
    };
    (across * LEAN_PULLED.1).clamp(-LEAN_PULLED.0, LEAN_PULLED.0)
}

/// Things carried together on their way, each between its own room (0) and its place in the stack (1): lifted, each
/// flies from its room to its place (those not shown in the stack, see `SHOWN`, to where the backmost is, fading as
/// they come, see `seen_of`). Let go where nothing takes them, each flies back to its own room, one setting off after
/// another in their order (see `stagger`), those not shown coming out from behind the backmost, and lands there.
struct Flight {
    ways: Vec<Spring>,
    /// Each set off back yet, and landed in its room.
    turned: Vec<bool>,
    landed: Vec<bool>,
    /// Let go where nothing takes them: how long since (seconds).
    back: Option<f32>,
}

impl Flight {
    /// `tiles` of them lifted out of their rooms.
    fn lifted(tiles: usize) -> Flight {
        let way = || {
            let mut way = Spring::at(0.0, LIFTING.0, LIFTING.1);
            way.target = 1.0;
            way
        };
        Flight { ways: (0..tiles).map(|_| way()).collect(), turned: vec![false; tiles], landed: vec![false; tiles], back: None }
    }

    /// Let go where nothing takes them: each flying back from where it is to its room as its turn comes, as a thing
    /// alone goes back to its own.
    fn go_back(&mut self) {
        self.back = Some(0.0);
        self.turn();
    }

    /// Those whose turn has come set off back, from where they are and as fast as they go.
    fn turn(&mut self) {
        let Some(since) = self.back else { return };
        let tiles = self.ways.len();
        for (at, way) in self.ways.iter_mut().enumerate() {
            if !self.turned[at] && since >= stagger(at, tiles) {
                let speed = way.speed;
                *way = Spring::at(way.value, GOING_BACK.0, GOING_BACK.1);
                (way.speed, way.target) = (speed, 0.0);
                self.turned[at] = true;
            }
        }
    }

    /// `seconds` on, each `apart` from its room in its place (DIPs, as it was last drawn): those come back to their
    /// rooms now (by their order), each landing once, as near as `LANDS_NEAR` and `LANDS_LIFTED`.
    fn step(&mut self, seconds: f32, apart: &[f32]) -> Vec<usize> {
        if let Some(since) = &mut self.back {
            *since += seconds;
        }
        self.turn();
        for way in &mut self.ways {
            way.step(seconds);
        }
        let near = |at: usize| (LANDS_NEAR / apart[at]).min(LANDS_LIFTED);
        let come: Vec<usize> = (0..self.ways.len()).filter(|&at| self.turned[at] && !self.landed[at] && self.ways[at].rests(near(at))).collect();
        for &at in &come {
            self.landed[at] = true;
        }
        come
    }

    /// Whether they are on their way back.
    fn going_back(&self) -> bool {
        self.back.is_some()
    }

    /// Whether all are back in their rooms.
    fn home(&self) -> bool {
        self.back.is_some() && self.landed.iter().all(|landed| *landed)
    }

    /// How far each is on its way.
    fn ways(&self) -> Vec<f32> {
        self.ways.iter().map(|way| way.value).collect()
    }
}

/// How long after they are let go the `at`th of `tiles` things flying back sets off (seconds): one after another in
/// their order, a `STAGGER` apart, closer where there are many, the last `STAGGER_ALL` after the first at most.
fn stagger(at: usize, tiles: usize) -> f32 {
    at as f32 * STAGGER.min(STAGGER_ALL / tiles.saturating_sub(1).max(1) as f32)
}

/// Where a plate is drawn: its middle, how far turned (degrees) and how large (DIPs, see `between`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    centre: (f32, f32),
    turn: f32,
    scale: f32,
}

/// Where a plate `plate` large placed by `placed` (moved, turned and made larger alike across and down) is drawn.
fn placed_of(placed: &Matrix3x2, plate: (f32, f32)) -> Placed {
    let (x, y) = (plate.0 / 2.0, plate.1 / 2.0);
    Placed { centre: (x * placed.M11 + y * placed.M21 + placed.M31, x * placed.M12 + y * placed.M22 + placed.M32), turn: placed.M12.atan2(placed.M11).to_degrees(), scale: placed.M11.hypot(placed.M12) }
}

/// What places a plate `plate` large where `placed` says: made larger and turned about its middle, its middle there.
fn matrix_of(placed: Placed, plate: (f32, f32)) -> Matrix3x2 {
    Matrix3x2::translation(-plate.0 / 2.0, -plate.1 / 2.0) * Matrix3x2::scale(placed.scale, placed.scale) * Matrix3x2::rotation(placed.turn) * Matrix3x2::translation(placed.centre.0, placed.centre.1)
}

/// `way` of the way from `from` to `to`: its middle along the line between theirs, turned and made larger between,
/// as a thing flies (and turns as it goes) from one place to the other.
fn between(from: Placed, to: Placed, way: f32) -> Placed {
    let at = |from: f32, to: f32| from + (to - from) * way;
    Placed { centre: (at(from.centre.0, to.centre.0), at(from.centre.1, to.centre.1)), turn: at(from.turn, to.turn), scale: at(from.scale, to.scale) }
}

/// Where the `at`th of the things carried together (`shown` of them shown) stands in the stack, its plate `plate` large,
/// the one on top placed by `held`: it as it is held, those behind it each placed further behind (see `behind_at`),
/// those not shown where the backmost is (they fade as they come there, see `seen_of`).
fn slot(at: usize, shown: usize, plate: (f32, f32), held: Matrix3x2) -> Matrix3x2 {
    match at.min(shown - 1) {
        0 => held,
        behind => behind_at(behind, plate) * held,
    }
}

/// How much of the `at`th of the things carried together (`shown` of them shown) shows, `way` of the way from its room
/// to its place: in its room, all of it, in its place, the one on top all, those behind it fainter (see
/// `BEHIND_SEEN`), those not shown none.
fn seen_of(at: usize, shown: usize, way: f32) -> f32 {
    let there = match at {
        0 => 1.0,
        at if at < shown => BEHIND_SEEN[at - 1],
        _ => 0.0,
    };
    1.0 + (there - 1.0) * way.clamp(0.0, 1.0)
}

/// How a thing carried with others is drawn: where (on the screen, DIPs), how much of it shows, how lifted it looks (0
/// as its room draws it, 1 in hand: see `paint_tile`), and how far its place in the stack is from its room (DIPs).
#[derive(Clone, Copy, Debug)]
struct Drawn {
    placed: Matrix3x2,
    seen: f32,
    lifted: f32,
    apart: f32,
}

/// A thing carried, as it is placed in its room: its plate's size, and its plate's corner there (DIPs).
type Room = ((f32, f32), (f32, f32));

/// A box placed: where (what places it), and its size (DIPs).
type Boxed = (Matrix3x2, (f32, f32));

/// How each of the things carried together is drawn, `tiles` (each its plate's size, and its plate's corner in its
/// room) each `ways` of the way from its room to its place in the stack, the one on top placed by `held` (see `slot`),
/// `seen` of them shown in hand (see `REFUSED`): each moved, turned, made larger, lifted and faded by its own way, so
/// that in its room it is drawn just as its room draws it.
fn placements(tiles: &[Room], held: Matrix3x2, ways: &[f32], seen: f32) -> Vec<Drawn> {
    let shown = tiles.len().min(SHOWN);
    tiles
        .iter()
        .zip(ways)
        .enumerate()
        .map(|(at, (&(plate, room), &way))| {
            let home = placed_of(&Matrix3x2::translation(room.0, room.1), plate);
            let there = placed_of(&slot(at, shown, plate, held), plate);
            let lifted = way.clamp(0.0, 1.0);
            Drawn {
                placed: matrix_of(between(home, there, way), plate),
                seen: seen_of(at, shown, way) * (1.0 - (1.0 - seen) * lifted),
                lifted,
                apart: (there.centre.0 - home.centre.0).hypot(there.centre.1 - home.centre.1),
            }
        })
        .collect()
}

/// The plate under a thing carried (from its plate's corner, `plate` large, DIPs), `lifted` of the way from its room:
/// in hand all of it, the room round what it is (`PAD`) round that, drawn in to the thing's own box as it comes to its
/// room.
fn plate_of(plate: (f32, f32), lifted: f32) -> (f32, f32, f32, f32) {
    let inset = PAD * (1.0 - lifted);
    (inset, inset, plate.0 - 2.0 * inset, plate.1 - 2.0 * inset)
}

/// How much shows of what was drawn of a thing as it was lifted, and of what its room draws of it (its mark chosen with
/// others), `lifted` of the way from its room: in hand the one, in its room the other, both whole half way (so what is
/// whole in both never shows through).
fn looks_of(lifted: f32) -> (f32, f32) {
    ((2.0 * lifted).min(1.0), (2.0 * (1.0 - lifted)).min(1.0))
}

/// The box (left, top, right, bottom) boxes `boxes` (each where it is placed, and its size) reach over, turned as
/// they may be.
fn reach_of(boxes: &[Boxed]) -> (f32, f32, f32, f32) {
    boxes.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(left, top, right, bottom), (placed, size)| {
        [(0.0, 0.0), (size.0, 0.0), (0.0, size.1), (size.0, size.1)].into_iter().fold((left, top, right, bottom), |(left, top, right, bottom), (x, y)| {
            let (x, y) = (x * placed.M11 + y * placed.M21 + placed.M31, x * placed.M12 + y * placed.M22 + placed.M32);
            (left.min(x), top.min(y), right.max(x), bottom.max(y))
        })
    })
}

/// Its window's box on the screen (physical px, `px` a DIP): what it draws reaching over `reach` (on the screen, DIPs),
/// with room round it for shadows.
fn window_rect((left, top, right, bottom): (f32, f32, f32, f32), px: f32) -> RECT {
    RECT { left: ((left - ROOM) * px).floor() as i32, top: ((top - ROOM) * px).floor() as i32, right: ((right + ROOM) * px).ceil() as i32, bottom: ((bottom + ROOM) * px).ceil() as i32 }
}

/// Where the `at`th plate behind one `plate` large is placed (from the first behind it, 1): a step up and to the right
/// for each, turned about its middle a step for each.
fn behind_at(at: usize, plate: (f32, f32)) -> Matrix3x2 {
    let step = at as f32 * BEHIND_STEP;
    Matrix3x2::rotation_around(at as f32 * BEHIND_TURN, Vector2 { X: plate.0 / 2.0, Y: plate.1 / 2.0 }) * Matrix3x2::translation(step, -step)
}

/// Where the count's corner is on a plate `plate` large with `behind` plates behind it: in from the backmost's corner.
fn count_at(plate: (f32, f32), behind: usize) -> (f32, f32) {
    let back = behind as f32 * BEHIND_STEP;
    (plate.0 + back - COUNT_AT.0, -back - COUNT_AT.1)
}

/// The count's figure's font in `theme`: its small one, bold.
fn count_font(theme: &crate::ui::theme::Theme) -> crate::ui::canvas::Font {
    crate::ui::canvas::Font { weight: 700.0, ..theme.small }
}

/// How wide the count is, its figure `inked` wide: the room round it each side, round as a dot at least.
fn count_wide(inked: f32) -> f32 {
    (inked + 2.0 * COUNT_PAD).max(COUNT_TALL)
}

impl Carry {
    /// A step on to `now`: in hand, it follows the pointer. Let go, it is given over, or each of it flies back into
    /// its own room. Whether it is carried still.
    fn step(&mut self, now: Instant) -> bool {
        let seconds = now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
        self.last = now;
        let going = crate::station::drag::going();
        // (Another drag asked for meanwhile: this one is over, as given up.)
        let ended = if going.drag == self.lifted.drag { going.ended } else { Some(Dragged::Not) };
        let px = self.lifted.px;
        let pressed = self.lifted.tiles[0].at;
        let grab = ((self.lifted.from.x - pressed.x) as f32, (self.lifted.from.y - pressed.y) as f32);
        if self.given.is_none() && !self.flight.going_back() {
            match ended {
                None => {
                    if let Some(cursor) = crate::mouse::cursor_position() {
                        self.corner = (cursor.x as f32 - grab.0, cursor.y as f32 - grab.1);
                        self.trail.push((now, cursor));
                        self.trail.retain(|(at, _)| now.saturating_duration_since(*at) <= LEAN_SPAN);
                        let home = crate::panel::contains(&self.lifted.over, cursor);
                        let (lift, seen, says) = wants(home, &going.over);
                        self.lift.target = lift;
                        self.seen.target = seen;
                        self.saying.target = if says { 1.0 } else { 0.0 };
                        if let Some(said) = going.over.says.filter(|_| !home) {
                            self.says = said;
                        }
                    }
                    self.lean.target = lean_of(&self.trail, px, now);
                }
                // Taken: given over where it was let go, their rooms as they were at once.
                Some(Dragged::Taken) => {
                    self.given = Some(now);
                    self.saying.target = 0.0;
                    crate::ui::view::land_tiles(self.lifted.carried.clone(), now);
                }
                // Not taken, or given up: each back into its room, where what it was lifted off is still shown, its
                // lift eased away on its own way there (see `placements`). Else gone where it is.
                Some(Dragged::Not) => {
                    self.saying.target = 0.0;
                    self.lean.target = 0.0;
                    self.seen.target = 1.0;
                    if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.lifted.window) }.as_bool() {
                        self.flight.go_back();
                    } else {
                        self.given = Some(now);
                        crate::ui::view::land_tiles(self.lifted.carried.clone(), now);
                    }
                }
            }
        }
        for spring in [&mut self.lift, &mut self.lean, &mut self.seen, &mut self.saying] {
            spring.step(seconds);
        }
        // Each back in its room: put down there, its room drawn full again as it was drawn coming (and no longer drawn
        // here).
        for at in self.flight.step(seconds, &self.apart) {
            crate::ui::view::settle_tiles(&[self.lifted.tiles[at].hit]);
        }
        // All back: the rooms of those that did not fly drawn full again.
        if self.flight.home() {
            let flown: Vec<Hit> = self.lifted.tiles.iter().map(|tile| tile.hit).collect();
            crate::ui::view::land_tiles(self.lifted.carried.iter().copied().filter(|hit| !flown.contains(hit)).collect(), now);
            return false;
        }
        let given = self.given.map(|since| (now.saturating_duration_since(since).as_secs_f32() / GIVEN.as_secs_f32()).min(1.0));
        if given == Some(1.0) {
            return false;
        }
        self.draw(given.unwrap_or(0.0));
        true
    }

    /// Draws it where it is, given over by `given` of the way.
    fn draw(&mut self, given: f32) {
        let (px, look) = (self.lifted.px, self.lifted.look);
        let theme = look_theme(look);
        let tiles = &self.lifted.tiles;
        let plates: Vec<(f32, f32)> = tiles.iter().map(|tile| (tile.size.0 + 2.0 * PAD, tile.size.1 + 2.0 * PAD)).collect();
        let shown = tiles.len().min(SHOWN);
        // Held where it was pressed: lifted larger and leaning about there, all of it made smaller together as it is
        // given over (on the screen, DIPs).
        let about = (PAD + (self.lifted.from.x - tiles[0].at.x) as f32 / px, PAD + (self.lifted.from.y - tiles[0].at.y) as f32 / px);
        let eased = given * given;
        let grown = (1.0 + LIFTED_BY * self.lift.value.max(0.0)) * (1.0 - (1.0 - GIVEN_TO) * eased);
        let corner = (self.corner.0 / px - PAD, self.corner.1 / px - PAD);
        let held = Matrix3x2::translation(-about.0, -about.1) * Matrix3x2::scale(grown, grown) * Matrix3x2::rotation(self.lean.value) * Matrix3x2::translation(about.0 + corner.0, about.1 + corner.1);
        let rooms: Vec<Room> = tiles.iter().zip(&plates).map(|(tile, plate)| (*plate, (tile.at.x as f32 / px - PAD, tile.at.y as f32 / px - PAD))).collect();
        let ways = self.flight.ways();
        let drawn = placements(&rooms, held, &ways, self.seen.value);
        self.apart = drawn.iter().map(|drawn| drawn.apart).collect();
        // (Those landed are drawn by their rooms.)
        let shows = |at: usize| drawn[at].seen > 0.0 && !self.flight.landed[at];
        // The count on the stack's corner, there as they all are in their places.
        let count = (self.lifted.carried.len() > 1).then(|| {
            let figure = self.lifted.count.to_string();
            let wide = count_wide(self.gfx.measure(&figure, count_font(&theme)));
            let at = count_at(plates[0], shown - 1);
            (Matrix3x2::translation(at.0, at.1) * held, wide, ways.iter().take(shown).fold(1.0f32, |least, way| least.min(*way)).clamp(0.0, 1.0))
        });
        let seen = self.seen.value * (1.0 - eased);
        let (lift, saying) = (self.lift.value, self.saying.value * (1.0 - eased));
        // What a drop does, under the one on top as large as it is drawn there.
        let says = &self.says;
        let says_at = (corner.0 + 2.0, corner.1 + about.1 + (plates[0].1 - about.1) * grown + SAYS_GAP);
        let says_wide = if saying > 0.01 && !says.is_empty() { self.gfx.measure(says, theme.small) + 34.0 } else { 0.0 };
        let mut boxes: Vec<Boxed> = (0..tiles.len()).filter(|at| shows(*at)).map(|at| (drawn[at].placed, plates[at])).collect();
        boxes.extend(count.map(|(placed, wide, _)| (placed, (wide, COUNT_TALL))));
        if says_wide > 0.0 {
            boxes.push((Matrix3x2::translation(says_at.0, says_at.1), (says_wide, SAYS_TALL)));
        }
        let rect = window_rect(reach_of(&boxes), px);
        if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.window.hwnd) }.as_bool() {
            self.window.place_keeping(rect);
            self.window.show();
            self.window.lift(true);
            self.window.raise();
        }
        self.window.place_keeping(rect);
        let Some(surface) = self.surface.as_mut() else { return };
        let pixels = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        // (What is on the screen at its window's corner, drawn at its frame's.)
        let origin = Matrix3x2::translation(-rect.left as f32 / px, -rect.top as f32 / px);
        let _ = surface.draw(&self.gfx, pixels, px, |frame| {
            faded(frame, 1.0 - eased, || {
                // The backmost first, the one on top last.
                for at in (0..tiles.len()).rev().filter(|at| shows(*at)) {
                    frame.place(drawn[at].placed * origin);
                    faded(frame, drawn[at].seen, || paint_tile(frame, look, &tiles[at].parts, tiles[at].home.as_deref(), plates[at], lift, drawn[at].lifted));
                }
                if let Some((placed, _, shows)) = count.filter(|count| count.2 > 0.0) {
                    frame.place(placed * origin);
                    faded(frame, shows * self.seen.value, || paint_count(frame, look, self.lifted.count, (0.0, 0.0)));
                }
            });
            if says_wide > 0.0 {
                faded(frame, saying * seen, || {
                    frame.place(Matrix3x2::translation(says_at.0, says_at.1) * origin);
                    paint_says(frame, look, says);
                });
            }
        });
    }
}

/// A thing carried, its plate `plate` large (DIPs) from where the frame is placed, `lifted` of the way from its room
/// (see `placements`): its shadow under it as lifted as it is (`lift` in hand), its plate in the look of what it was
/// lifted off, drawn in to its own box and fading as it comes to its room, and on it what was drawn of it there
/// (`parts`), turning into what its room draws of it (`home`, where that is otherwise: see `looks_of`). In its room,
/// just what its room draws.
fn paint_tile(frame: &Frame, look: crate::ui::menu::Look, parts: &[Part], home: Option<&[Part]>, plate: (f32, f32), lift: f32, lifted: f32) {
    let (x, y, w, h) = plate_of(plate, lifted);
    let shadow = lifted_shadow(lift * lifted);
    if shadow.dark > 0.0 {
        let radius = look_theme(look).radius.min(h / 2.0);
        frame.shadow(Color::hex(0, shadow.dark), x, y, w, h, radius, shadow.blur, shadow.drop);
    }
    if lifted > 0.0 {
        faded(frame, lifted, || {
            crate::ui::menu::plate(frame, look, (x, y, w, h));
        });
    }
    let mut placed = Matrix3x2::default();
    unsafe { frame.dc.GetTransform(&mut placed) };
    unsafe { frame.dc.SetTransform(&(Matrix3x2::translation(PAD, PAD) * placed)) };
    match home {
        Some(home) => {
            let (as_lifted, as_home) = looks_of(lifted);
            if as_lifted > 0.0 {
                faded(frame, as_lifted, || crate::ui::morph::draw(frame, parts));
            }
            if as_home > 0.0 {
                faded(frame, as_home, || crate::ui::morph::draw(frame, home));
            }
        }
        None => crate::ui::morph::draw(frame, parts),
    }
    unsafe { frame.dc.SetTransform(&placed) };
}

/// Things carried together, all in their places, the one on top from where the frame is placed (`tiles`: what was drawn
/// of each and its plate's size, the one on top first): lifted `lift` of the way, those behind it drawn first, each
/// with what it is, fainter, and on their corner `count` (where more than one is carried, how many files land).
#[cfg(test)]
pub(crate) fn paint_stack(frame: &Frame, look: crate::ui::menu::Look, tiles: &[(&[Part], (f32, f32))], lift: f32, count: Option<usize>) {
    let shown = tiles.len().min(SHOWN);
    let mut placed = Matrix3x2::default();
    unsafe { frame.dc.GetTransform(&mut placed) };
    for (at, (parts, plate)) in tiles.iter().enumerate().take(shown).rev() {
        unsafe { frame.dc.SetTransform(&(slot(at, shown, *plate, Matrix3x2::identity()) * placed)) };
        faded(frame, seen_of(at, shown, 1.0), || paint_tile(frame, look, parts, None, *plate, lift, 1.0));
    }
    unsafe { frame.dc.SetTransform(&placed) };
    if let Some(count) = count {
        paint_count(frame, look, count, count_at(tiles[0].1, shown - 1));
    }
}

/// How many files land where what is carried is let go (`count`), from (`left`, `top`) where the frame is placed: its
/// figure on a pill in the skin's own mark (glass's blue, chart paper's ink, Windows 11's accent), as wide as it needs.
fn paint_count(frame: &Frame, look: crate::ui::menu::Look, count: usize, (left, top): (f32, f32)) {
    let theme = look_theme(look);
    let (ground, ink) = count_colours(&theme);
    let font = count_font(&theme);
    let figure = count.to_string();
    let inked = frame.measure(&figure, font);
    let wide = count_wide(inked);
    let radius = if theme.skin == crate::ui::theme::Skin::Paper { 4.0 } else { COUNT_TALL / 2.0 };
    frame.shadow(Color::hex(0, 0.35), left, top, wide, COUNT_TALL, radius, 6.0, 2.0);
    frame.fill_rounded(ground, left, top, wide, COUNT_TALL, radius);
    frame.text(&figure, font, ink, left + (wide - inked) / 2.0, frame.line_in(font, top, COUNT_TALL), inked + 1.0, Align::Start);
}

/// The count's pill and figure in `theme`: on glass its own blue under white, on chart paper its ink under the paper's
/// colour, on Windows 11 its accent under black (white, light).
fn count_colours(theme: &crate::ui::theme::Theme) -> (Color, Color) {
    use crate::ui::theme::Skin;
    match theme.skin {
        Skin::Glass => (theme.ink("station").trace, Color::hex(0xFFFFFF, 1.0)),
        Skin::Paper => (theme.text, Color { a: 1.0, ..theme.paper }),
        Skin::Fluent => (crate::ui::theme::accent(theme.dark), if theme.dark { Color::hex(0x000000, 1.0) } else { Color::hex(0xFFFFFF, 1.0) }),
    }
}

/// What a drop does (`says`), on a plate of its own from where the frame is placed, in the look of what it was lifted
/// off: a plus before it, as a copy is.
fn paint_says(frame: &Frame, look: crate::ui::menu::Look, says: &str) {
    let theme = look_theme(look);
    let font = theme.small;
    let wide = frame.measure(says, font) + 14.0 + 20.0;
    let plate = crate::ui::menu::plate(frame, look, (0.0, 0.0, wide, SAYS_TALL));
    frame.icon(crate::ui::icons::Icon::Add, Point { x: 14.0, y: SAYS_TALL / 2.0 }, 13.0, plate.highlight);
    frame.text(says, font, theme.text, 24.0, frame.line_in(font, 0.0, SAYS_TALL), wide - 24.0, Align::Start);
}

/// The skin a look is in (what a station's lane is lifted off is always in one), as light or dark.
fn look_theme(look: crate::ui::menu::Look) -> crate::ui::theme::Theme {
    crate::ui::theme::Theme::new(look.skin.unwrap_or(crate::ui::theme::Skin::Paper), look.dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count is drawn in each skin's own mark, its figure plain on it: glass's blue under white, chart paper's ink
    /// under the paper's colour, Windows 11's accent under black, dark, and white, light.
    #[test]
    fn the_count_is_in_each_skins_mark() {
        use crate::ui::theme::{Skin, Theme};
        for dark in [true, false] {
            let glass = Theme::new(Skin::Glass, dark);
            assert_eq!(count_colours(&glass), (glass.ink("station").trace, Color::hex(0xFFFFFF, 1.0)));
            let paper = Theme::new(Skin::Paper, dark);
            assert_eq!(count_colours(&paper), (paper.text, Color { a: 1.0, ..paper.paper }));
            let fluent = Theme::new(Skin::Fluent, dark);
            assert_eq!(count_colours(&fluent).0, crate::ui::theme::accent(dark));
            assert_eq!(count_colours(&fluent).1, Color::hex(if dark { 0x000000 } else { 0xFFFFFF }, 1.0));
        }
    }

    /// Over what it was lifted off, it is lifted half way, all of it seen, nothing said (let go there, it goes back);
    /// over a program that would take it, lifted, said what the program says a drop does where it says one; over one
    /// that would not, seen faintly, nothing said.
    #[test]
    fn what_is_carried_shows_what_is_under_it() {
        let says = |takes: bool, says: Option<&str>| Over { takes, says: says.map(str::to_string) };
        assert_eq!(wants(true, &says(true, Some("复制到 下载"))), (HOME_LIFT, 1.0, false));
        assert_eq!(wants(false, &says(true, Some("复制到 下载"))), (1.0, 1.0, true));
        assert_eq!(wants(false, &says(true, None)), (1.0, 1.0, false));
        assert_eq!(wants(false, &says(false, None)), (1.0, REFUSED, false));
    }

    /// Moved across lately, it leans that way, the faster the further, so far at most; still, or moved long ago, not.
    #[test]
    fn what_is_carried_leans_as_it_is_moved_across() {
        let now = Instant::now();
        let at = |ms: u64, x: i32| (now - Duration::from_millis(ms), POINT { x, y: 0 });
        assert_eq!(lean_of(&[], 1.0, now), 0.0);
        let slow = lean_of(&[at(40, 0), at(0, 8)], 1.0, now);
        assert!(slow > 0.0 && slow < LEAN_PULLED.0, "{slow}");
        assert_eq!(lean_of(&[at(40, 400), at(0, 0)], 1.0, now), -LEAN_PULLED.0);
        // (Twice the pixels at twice the scale: as fast, in DIPs.)
        assert!((lean_of(&[at(40, 0), at(0, 16)], 2.0, now) - slow).abs() < 1e-4);
        assert_eq!(lean_of(&[at(400, 0), at(300, 100)], 1.0, now), 0.0);
    }

    /// A plate's place read off what places it is the place that places it so: where its middle is, how far turned and
    /// how large. Between two places, it is at the one at the start and the other at the end, half way half of each.
    #[test]
    fn a_plates_place_is_read_off_what_places_it() {
        let plate = (120.0, 60.0);
        let placed = Placed { centre: (300.0, 200.0), turn: 9.0, scale: 1.04 };
        let read = placed_of(&matrix_of(placed, plate), plate);
        assert!((read.centre.0 - 300.0).abs() < 1e-3 && (read.centre.1 - 200.0).abs() < 1e-3, "{read:?}");
        assert!((read.turn - 9.0).abs() < 1e-3 && (read.scale - 1.04).abs() < 1e-5, "{read:?}");
        let room = placed_of(&Matrix3x2::translation(10.0, 20.0), plate);
        assert_eq!(room, Placed { centre: (70.0, 50.0), turn: 0.0, scale: 1.0 });
        assert_eq!(between(room, placed, 0.0), room);
        assert_eq!(between(room, placed, 1.0), placed);
        assert_eq!(between(room, placed, 0.5), Placed { centre: (185.0, 125.0), turn: 4.5, scale: 1.02 });
    }

    /// In the stack, the one pressed is on top as it is held, the next two behind it each placed further behind, all
    /// of it shown, then fainter, the rest where the backmost is, not shown (they fade as they come). Two carried, the
    /// rest behind the one behind it.
    #[test]
    fn things_carried_together_stand_in_the_stack_in_their_order() {
        let (plate, held) = ((100.0, 80.0), Matrix3x2::translation(400.0, 300.0));
        assert_eq!(slot(0, 3, plate, held), held);
        assert_eq!(slot(1, 3, plate, held), behind_at(1, plate) * held);
        assert_eq!(slot(2, 3, plate, held), behind_at(2, plate) * held);
        assert_eq!(slot(5, 3, plate, held), behind_at(2, plate) * held);
        assert_eq!(slot(3, 2, plate, held), behind_at(1, plate) * held);
        assert_eq!(slot(0, 1, plate, held), held);
        assert_eq!([0, 1, 2, 3].map(|at| seen_of(at, 3, 1.0)), [1.0, BEHIND_SEEN[0], BEHIND_SEEN[1], 0.0]);
        assert_eq!([0, 1, 2, 3].map(|at| seen_of(at, 3, 0.0)), [1.0; 4]);
        assert_eq!(seen_of(3, 3, 0.5), 0.5);
    }

    /// Lifted, each is drawn in its own room, then flies into its place in the stack: at the start each plate's corner
    /// where its room's is, unturned, as large, at the end each where `slot` puts it.
    #[test]
    fn each_flies_from_its_own_room_into_its_place() {
        let plate = (100.0, 80.0);
        let rooms = [(plate, (40.0, 400.0)), (plate, (40.0, 300.0)), (plate, (160.0, 300.0)), (plate, (280.0, 300.0))];
        let held = Matrix3x2::scale(1.04, 1.04) * Matrix3x2::rotation(3.0) * Matrix3x2::translation(700.0, 120.0);
        for (at, drawn) in placements(&rooms, held, &[0.0; 4], 1.0).into_iter().enumerate() {
            assert!(near(&drawn.placed, &Matrix3x2::translation(rooms[at].1 .0, rooms[at].1 .1)), "{at}: {drawn:?}");
            assert_eq!((drawn.seen, drawn.lifted), (1.0, 0.0));
        }
        for (at, drawn) in placements(&rooms, held, &[1.0; 4], 1.0).into_iter().enumerate() {
            assert!(near(&drawn.placed, &slot(at, 3, plate, held)), "{at}: {drawn:?}");
            assert_eq!(drawn.lifted, 1.0);
        }
    }

    /// Whether two placings place alike (within a hundredth of a DIP over a plate's size).
    fn near(one: &Matrix3x2, other: &Matrix3x2) -> bool {
        [(one.M11, other.M11), (one.M12, other.M12), (one.M21, other.M21), (one.M22, other.M22)].iter().all(|(a, b)| (a - b).abs() < 1e-4) && (one.M31 - other.M31).abs() < 0.01 && (one.M32 - other.M32).abs() < 0.01
    }

    /// Six chosen and carried, three shown in the stack: their rooms (plates' sizes and corners), and where the one on
    /// top is held, larger and turned about its middle just over its own room (so it is no way off, but lifted).
    fn six_rooms() -> ([Room; 6], Matrix3x2) {
        let plate = (100.0, 80.0);
        let rooms = [(plate, (40.0, 400.0)), (plate, (40.0, 300.0)), (plate, (160.0, 300.0)), (plate, (280.0, 300.0)), (plate, (40.0, 500.0)), (plate, (160.0, 500.0))];
        let middle = (plate.0 / 2.0, plate.1 / 2.0);
        let held = Matrix3x2::translation(-middle.0, -middle.1) * Matrix3x2::scale(1.04, 1.04) * Matrix3x2::rotation(6.0) * Matrix3x2::translation(middle.0 + 40.0, middle.1 + 400.0);
        (rooms, held)
    }

    /// Gathered into the stack, then let go where nothing takes them: `landed` told of each as it lands (its place in
    /// their order, how it is drawn then), its rooms `rooms` as `six_rooms` has them, `seen` of them shown in hand.
    fn fly_back(seen: f32, mut landed: impl FnMut(usize, Drawn)) -> Flight {
        let (rooms, held) = six_rooms();
        let mut flight = Flight::lifted(rooms.len());
        let apart: Vec<f32> = placements(&rooms, held, &flight.ways(), seen).iter().map(|drawn| drawn.apart).collect();
        for _ in 0..60 {
            assert!(flight.step(1.0 / 60.0, &apart).is_empty());
        }
        assert!(flight.ways().iter().all(|way| (way - 1.0).abs() < 0.01), "{:?}", flight.ways());
        flight.go_back();
        for _ in 0..120 {
            for at in flight.step(1.0 / 60.0, &apart) {
                landed(at, placements(&rooms, held, &flight.ways(), seen)[at]);
            }
        }
        flight
    }

    /// Let go where nothing takes them, all six chosen fly back apart, those shown and those not, each to its own room,
    /// and land there, each once, as it comes: where it lands, its plate's corner is its room's. Lifted, none lands.
    #[test]
    fn each_flies_back_apart_and_lands_in_its_own_room() {
        let (rooms, _) = six_rooms();
        let mut landed = Vec::new();
        let flight = fly_back(1.0, |at, drawn| {
            let room = rooms[at].1;
            assert!((drawn.placed.M31 - room.0).abs() <= LANDS_NEAR && (drawn.placed.M32 - room.1).abs() <= LANDS_NEAR, "{at} lands at {drawn:?}");
            landed.push(at);
        });
        landed.sort_unstable();
        assert_eq!(landed, [0, 1, 2, 3, 4, 5]);
        assert!(flight.home());
    }

    /// Landing, each is drawn just as its room draws it, so that its room drawn in its stead shows nothing new: where
    /// its room is, as large, unturned, all of it showing (though it showed less over a program that would not take
    /// it), no shadow, no plate under it but its own box, drawn as its room draws it (none of it as it was lifted).
    /// The one pressed too, held just over its room, no way off but lifted.
    #[test]
    fn what_lands_is_drawn_as_its_room_draws_it() {
        let (rooms, _) = six_rooms();
        let mut landed = 0;
        fly_back(REFUSED, |at, drawn| {
            let (placed, plate) = (drawn.placed, rooms[at].0);
            assert!((placed.M11.hypot(placed.M12) - 1.0).abs() < 1e-3 && placed.M12.atan2(placed.M11).to_degrees().abs() < 0.05, "{at}: {drawn:?}");
            assert!((placed.M31 - rooms[at].1 .0).abs() <= LANDS_NEAR && (placed.M32 - rooms[at].1 .1).abs() <= LANDS_NEAR, "{at}: {drawn:?}");
            assert!(drawn.seen > 0.995 && drawn.lifted < 0.01, "{at}: {drawn:?}");
            assert!(lifted_shadow(drawn.lifted).dark < 0.003, "{at}: {drawn:?}");
            let (x, y, w, h) = plate_of(plate, drawn.lifted);
            assert!((x - PAD).abs() < 0.1 && (y - PAD).abs() < 0.1 && (w - (plate.0 - 2.0 * PAD)).abs() < 0.2 && (h - (plate.1 - 2.0 * PAD)).abs() < 0.2, "{at}: {drawn:?}");
            let (as_lifted, as_home) = looks_of(drawn.lifted);
            assert!(as_lifted < 0.02 && as_home == 1.0, "{at}: {drawn:?}");
            landed += 1;
        });
        assert_eq!(landed, 6);
        // In hand all of it lifted, its plate all of it, as it was lifted.
        assert_eq!((plate_of((100.0, 80.0), 1.0), looks_of(1.0), looks_of(0.5), looks_of(0.0)), ((0.0, 0.0, 100.0, 80.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)));
    }

    /// Those chosen but not shown in the stack wait where the backmost is, as it is placed, turned and large, not
    /// shown, and set off back from there, coming out from behind it as they leave.
    #[test]
    fn those_not_shown_set_off_from_behind_the_backmost() {
        let (rooms, held) = six_rooms();
        let carried = placements(&rooms, held, &[1.0; 6], 1.0);
        for (at, drawn) in carried.iter().enumerate().skip(3) {
            assert!(near(&drawn.placed, &slot(2, 3, rooms[at].0, held)) && near(&drawn.placed, &carried[2].placed), "{at}: {drawn:?}");
            assert_eq!(drawn.seen, 0.0);
        }
        let leaving = placements(&rooms, held, &[1.0, 1.0, 1.0, 0.9, 0.97, 1.0], 1.0);
        assert!(leaving[3].seen > leaving[4].seen && leaving[4].seen > 0.0 && leaving[5].seen == 0.0, "{leaving:?}");
    }

    /// Let go where nothing takes them, they set off back one after another in their order, a `STAGGER` apart, closer
    /// where there are many, so that the last sets off `STAGGER_ALL` after the first at most.
    #[test]
    fn they_set_off_back_one_after_another() {
        let six: Vec<f32> = (0..6).map(|at| stagger(at, 6)).collect();
        for (at, after) in six.iter().enumerate() {
            assert!((after - at as f32 * STAGGER).abs() < 1e-6, "{six:?}");
        }
        assert!((stagger(29, 30) - STAGGER_ALL).abs() < 1e-6 && stagger(1, 30) < STAGGER);
        assert!((1..30).all(|at| stagger(at, 30) > stagger(at - 1, 30)));
        assert_eq!(stagger(0, 1), 0.0);
        // As flown: each sets off as its turn comes, not before.
        let mut flight = Flight::lifted(6);
        let apart = [500.0; 6];
        for _ in 0..60 {
            flight.step(1.0 / 60.0, &apart);
        }
        flight.go_back();
        let mut set_off = [None; 6];
        set_off[0] = flight.turned[0].then_some(0.0);
        let frame = 1.0 / 240.0;
        for step in 1..=60 {
            flight.step(frame, &apart);
            for (when, turned) in set_off.iter_mut().zip(&flight.turned) {
                if when.is_none() && *turned {
                    *when = Some(step as f32 * frame);
                }
            }
        }
        for (at, when) in set_off.iter().enumerate() {
            let when = when.unwrap();
            assert!(when >= stagger(at, 6) - 1e-4 && when < stagger(at, 6) + frame + 1e-4, "{at}: {set_off:?}");
        }
    }

    /// What it draws reaches as far as each box placed, turned as it may be: its window that far and room round it.
    #[test]
    fn its_window_holds_all_it_draws() {
        let square = Matrix3x2::translation(100.0, 50.0);
        let turned = Matrix3x2::rotation_around(90.0, Vector2 { X: 0.0, Y: 0.0 }) * Matrix3x2::translation(400.0, 300.0);
        let reach = reach_of(&[(square, (20.0, 10.0)), (turned, (30.0, 10.0))]);
        assert!((reach.0 - 100.0).abs() < 1e-3 && (reach.1 - 50.0).abs() < 1e-3 && (reach.2 - 400.0).abs() < 1e-3 && (reach.3 - 330.0).abs() < 1e-3, "{reach:?}");
        let rect = window_rect((100.0, 50.0, 400.0, 330.0), 1.5);
        assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (90, 15, 660, 555));
    }

    /// The plates behind the one on top draw what each of them is, each its own: carried together, a red thing on top
    /// and behind it one drawn blue over its head, the blue shows above the one on top, where the plate behind it is.
    #[test]
    fn the_plates_behind_draw_their_own_things() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = Gfx::new().unwrap();
        let filled = |hex: u32, y: f32, h: f32| vec![Part { key: Some("tile".into()), ops: vec![crate::ui::morph::Op::Fill { color: Color::hex(hex, 1.0), x: 0.0, y, w: 40.0, h }] }];
        let (red, blue) = (filled(0xFF0000, 0.0, 40.0), filled(0x0000FF, -16.0, 10.0));
        let look = crate::ui::menu::Look::of(crate::ui::theme::Skin::Paper, false);
        let plate = (40.0 + 2.0 * PAD, 40.0 + 2.0 * PAD);
        let pixels = gfx
            .draw_offscreen((120, 120), 1.0, |frame| {
                frame.origin(30.0, 40.0);
                paint_stack(frame, look, &[(&red, plate), (&blue, plate)], 1.0, Some(2));
            })
            .unwrap();
        let at = |x: usize, y: usize| {
            let p = &pixels[(y * 120 + x) * 4..][..4];
            (p[2], p[1], p[0])
        };
        // (Red in the top one's middle, blue over its top edge, a step up, where only the one behind reaches.)
        let (r, _, b) = at(30 + 28, 40 + 28);
        assert!(r > 200 && b < 60, "{:?}", at(58, 68));
        let (r, _, b) = at(30 + PAD as usize + 20, 28);
        assert!(b > 120 && r < 120, "{:?}", at(58, 28));
    }

    /// What is drawn of a thing as it was on its lane is drawn on its plate, its plate's room in from the plate's
    /// corner: lifted, a little larger, about where it is held.
    #[test]
    fn what_is_carried_is_drawn_as_it_was_on_its_plate() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = Gfx::new().unwrap();
        let parts = vec![Part { key: Some("tile".into()), ops: vec![crate::ui::morph::Op::Fill { color: Color::hex(0xFF0000, 1.0), x: 0.0, y: 0.0, w: 20.0, h: 20.0 }] }];
        let look = crate::ui::menu::Look::of(crate::ui::theme::Skin::Paper, false);
        let red_at = |lift: f32| {
            let pixels = gfx
                .draw_offscreen((100, 100), 1.0, |frame| {
                    frame.origin(0.0, 0.0);
                    frame.place(Matrix3x2::translation(-18.0, -18.0) * Matrix3x2::scale(1.0 + LIFTED_BY * lift, 1.0 + LIFTED_BY * lift) * Matrix3x2::translation(18.0 + 10.0, 18.0 + 10.0));
                    paint_stack(frame, look, &[(&parts, (36.0, 36.0))], lift, None);
                })
                .unwrap();
            // The red's leftmost and rightmost columns along its middle row.
            let row = 10 + PAD as usize + 10;
            let red: Vec<usize> = (0..100).filter(|&x| {
                let p = &pixels[(row * 100 + x) * 4..][..4];
                p[2] > 200 && p[1] < 60 && p[0] < 60
            }).collect();
            (red[0], red[red.len() - 1])
        };
        assert_eq!(red_at(0.0), (10 + PAD as usize, 10 + PAD as usize + 19));
        let (left, right) = red_at(1.0);
        assert!(right - left > 19, "{left} {right}");
    }
}
