//! A thing dragged off a station's lane into another program (a thing on it, copied lately, a file come lately),
//! carried on the screen by Glance as a note pulled off the panel is (see `widget::Pulled`): lifted off its room,
//! leaning as it is moved, in a window of its own over every other that the pointer passes through to what is under
//! it; what the program under the pointer says a drop does, said under it; let go where it is taken, given over there;
//! let go where it is not, or given up, back into its room. The drag itself is Windows' (see `station::drag`), which
//! tells on as it goes.

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

/// What is lifted off a station's lane to be carried: what it is pressed as, the drag that carries it off (see
/// `station::drag::announce`), what is drawn of it there (from its box's corner, DIPs) and its box's size, where its box
/// is on the screen and where it was pressed (physical px, `px` a DIP), what it was lifted off (the panel, a widget: let
/// go over it, it goes back) and that window, and the look of what it was lifted off (its plate's under it).
pub struct Lifted {
    pub hit: Hit,
    pub drag: u64,
    pub parts: Vec<Part>,
    pub size: (f32, f32),
    pub at: POINT,
    pub from: POINT,
    pub px: f32,
    pub over: RECT,
    pub window: HWND,
    pub look: crate::ui::menu::Look,
}

/// The room round what is carried on its plate, and its window's room round that for its shadow and leaning, and under
/// it for what a drop does (DIPs).
const PAD: f32 = 8.0;
const ROOM: f32 = 40.0;
const SAYS_GAP: f32 = 10.0;
const SAYS_TALL: f32 = 28.0;
/// How lifted it is over what it was lifted off (let go there, it goes back), and how much of it shows over a program
/// that would not take it.
const HOME_LIFT: f32 = 0.5;
const REFUSED: f32 = 0.55;
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
    /// Its box's corner on the screen (physical px).
    corner: (f32, f32),
    /// Going back into its room: its corner on its way there.
    back: Option<(Spring, Spring)>,
    /// Given over where it was let go: since when.
    given: Option<Instant>,
    last: Instant,
}

/// What `lifted` is carried off its lane from here (one carried still, put back first), drawn on `gfx`'s device.
pub fn begin(gfx: &Rc<Gfx>, lifted: Lifted) {
    CARRIED.with_borrow_mut(|carried| {
        if let Some(was) = carried.take() {
            crate::ui::view::land_tile(was.lifted.hit, Instant::now());
        }
        let Ok(mut window) = Window::new() else { return };
        window.set_click_through(true);
        // (Never in what a widget sees behind it: it is in hand, not part of the desktop.)
        crate::ui::scene::leave_out(window.hwnd, true);
        let surface = Surface::new(gfx, window.hwnd).ok();
        crate::ui::view::press_tile(None);
        crate::ui::view::carry_tile(Some(lifted.hit));
        let corner = (lifted.at.x as f32, lifted.at.y as f32);
        let mut lift = Spring::at(0.0, LIFTING.0, LIFTING.1);
        lift.target = 1.0;
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
            back: None,
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

/// Its window's box on the screen (physical px): its plate's corner `corner`, it `size` large (DIPs, its plate's room
/// round it too), with room round it, and under it for what a drop does.
fn window_rect(corner: (f32, f32), size: (f32, f32), px: f32) -> RECT {
    let (left, top) = ((corner.0 - (PAD + ROOM) * px).round() as i32, (corner.1 - (PAD + ROOM) * px).round() as i32);
    let (w, h) = (((size.0 + 2.0 * (PAD + ROOM)) * px).ceil() as i32, ((size.1 + 2.0 * (PAD + ROOM) + SAYS_GAP + SAYS_TALL) * px).ceil() as i32);
    RECT { left, top, right: left + w, bottom: top + h }
}

impl Carry {
    /// A step on to `now`: in hand, it follows the pointer; let go, it is given over or goes back. Whether it is
    /// carried still.
    fn step(&mut self, now: Instant) -> bool {
        let seconds = now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
        self.last = now;
        let going = crate::station::drag::going();
        // (Another drag asked for meanwhile: this one is over, as given up.)
        let ended = if going.drag == self.lifted.drag { going.ended } else { Some(Dragged::Not) };
        let px = self.lifted.px;
        let grab = ((self.lifted.from.x - self.lifted.at.x) as f32, (self.lifted.from.y - self.lifted.at.y) as f32);
        if self.given.is_none() && self.back.is_none() {
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
                // Taken: given over where it was let go, its room as it was at once.
                Some(Dragged::Taken) => {
                    self.given = Some(now);
                    self.saying.target = 0.0;
                    crate::ui::view::land_tile(self.lifted.hit, now);
                }
                // Not taken, or given up: back into its room, where what it was lifted off is still shown; else
                // gone where it is.
                Some(Dragged::Not) => {
                    self.saying.target = 0.0;
                    self.lift.target = 0.0;
                    self.lean.target = 0.0;
                    self.seen.target = 1.0;
                    if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.lifted.window) }.as_bool() {
                        let (mut x, mut y) = (Spring::at(self.corner.0, GOING_BACK.0, GOING_BACK.1), Spring::at(self.corner.1, GOING_BACK.0, GOING_BACK.1));
                        x.target = self.lifted.at.x as f32;
                        y.target = self.lifted.at.y as f32;
                        self.back = Some((x, y));
                    } else {
                        self.given = Some(now);
                        crate::ui::view::land_tile(self.lifted.hit, now);
                    }
                }
            }
        }
        for spring in [&mut self.lift, &mut self.lean, &mut self.seen, &mut self.saying] {
            spring.step(seconds);
        }
        if let Some((x, y)) = &mut self.back {
            x.step(seconds);
            y.step(seconds);
            self.corner = (x.value, y.value);
            // Home: put down into its room, which springs back to its size.
            if x.rests(0.5) && y.rests(0.5) && self.lift.rests(0.05) {
                crate::ui::view::land_tile(self.lifted.hit, now);
                return false;
            }
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
        let px = self.lifted.px;
        let size = (self.lifted.size.0 + 2.0 * PAD, self.lifted.size.1 + 2.0 * PAD);
        let rect = window_rect(self.corner, self.lifted.size, px);
        if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.window.hwnd) }.as_bool() {
            self.window.place_keeping(rect);
            self.window.show();
            self.window.lift(true);
            self.window.raise();
        }
        self.window.place_keeping(rect);
        let Some(surface) = self.surface.as_mut() else { return };
        let pixels = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        // Held where it was pressed: lifted larger and leaning about there.
        let about = (PAD + (self.lifted.from.x - self.lifted.at.x) as f32 / px, PAD + (self.lifted.from.y - self.lifted.at.y) as f32 / px);
        let eased = given * given;
        let grown = (1.0 + LIFTED_BY * self.lift.value.max(0.0)) * (1.0 - (1.0 - GIVEN_TO) * eased);
        let held = Matrix3x2::translation(-about.0, -about.1) * Matrix3x2::scale(grown, grown) * Matrix3x2::rotation_around(self.lean.value, Vector2 { X: 0.0, Y: 0.0 }) * Matrix3x2::translation(about.0 + ROOM, about.1 + ROOM);
        let seen = self.seen.value * (1.0 - eased);
        let (lift, saying, look) = (self.lift.value, self.saying.value * (1.0 - eased), self.lifted.look);
        let (parts, says) = (&self.lifted.parts, &self.says);
        // What a drop does, under it as large as it is drawn there.
        let says_at = (ROOM + 2.0, ROOM + about.1 + (size.1 - about.1) * grown + SAYS_GAP);
        let _ = surface.draw(&self.gfx, pixels, px, |frame| {
            faded(frame, seen, || {
                frame.place(held);
                paint_lifted(frame, look, parts, size, lift);
            });
            if saying > 0.01 && !says.is_empty() {
                faded(frame, saying * seen, || {
                    frame.origin(says_at.0, says_at.1);
                    paint_says(frame, look, says);
                });
            }
        });
    }
}

/// What is carried, its plate `size` large (DIPs) from where the frame is placed: lifted `lift` of the way, its shadow
/// under it, its plate in the look of what it was lifted off, and on it what was drawn of it there.
fn paint_lifted(frame: &Frame, look: crate::ui::menu::Look, parts: &[Part], size: (f32, f32), lift: f32) {
    let shadow = lifted_shadow(lift);
    if shadow.dark > 0.0 {
        let radius = look_theme(look).radius.min(size.1 / 2.0);
        frame.shadow(Color::hex(0, shadow.dark), 0.0, 0.0, size.0, size.1, radius, shadow.blur, shadow.drop);
    }
    crate::ui::menu::plate(frame, look, (0.0, 0.0, size.0, size.1));
    let mut placed = Matrix3x2::default();
    unsafe { frame.dc.GetTransform(&mut placed) };
    unsafe { frame.dc.SetTransform(&(Matrix3x2::translation(PAD, PAD) * placed)) };
    crate::ui::morph::draw(frame, parts);
    unsafe { frame.dc.SetTransform(&placed) };
}

/// What a drop does (`says`), on a plate of its own from where the frame is placed, in the look of what it was lifted
/// off: a plus before it, as a copy is.
fn paint_says(frame: &Frame, look: crate::ui::menu::Look, says: &str) {
    let theme = look_theme(look);
    let font = theme.small;
    let wide = frame.measure(says, font) + 14.0 + 20.0;
    let plate = crate::ui::menu::plate(frame, look, (0.0, 0.0, wide, SAYS_TALL));
    frame.icon(crate::ui::icons::Icon::Add, Point { x: 14.0, y: SAYS_TALL / 2.0 }, 13.0, plate.highlight);
    frame.text(says, font, theme.text, 24.0, (SAYS_TALL - font.size * 1.3) / 2.0, wide - 24.0, Align::Start);
}

/// The skin a look is in (what a station's lane is lifted off is always in one), as light or dark.
fn look_theme(look: crate::ui::menu::Look) -> crate::ui::theme::Theme {
    crate::ui::theme::Theme::new(look.skin.unwrap_or(crate::ui::theme::Skin::Paper), look.dark)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Its window holds it, its plate's room round it, room round that for its shadow and leaning, and under it what a
    /// drop does: its plate's corner where the box carried has its corner, less the plate's room.
    #[test]
    fn what_is_carried_is_drawn_in_its_window_round_it() {
        let rect = window_rect((500.0, 300.0), (100.0, 80.0), 1.5);
        assert_eq!((rect.left, rect.top), ((500.0 - 48.0 * 1.5) as i32, (300.0 - 48.0 * 1.5) as i32));
        assert_eq!(rect.right - rect.left, (196.0 * 1.5f32).ceil() as i32);
        assert_eq!(rect.bottom - rect.top, ((176.0 + SAYS_GAP + SAYS_TALL) * 1.5f32).ceil() as i32);
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
                    paint_lifted(frame, look, &parts, (36.0, 36.0), lift);
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
