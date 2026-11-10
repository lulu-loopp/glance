//! The guide to Glance: shown once as Glance first starts with it (installed, or updated to a version with it), and
//! again from the general page. Five steps, each with what it teaches played over and over on Glance's own things
//! till the user leaves it: a panel opened from the screen's edge (in a dialog over the desktop, the user's own desktop
//! small at its top, the real panel sliding out on it); the panel made larger and smaller by its corner, and made its
//! own size again by a double-click there; a lane of it torn off by its head, a widget of its own, and the whole panel
//! by the rest of it (each in the dialog, the whole screen small at its top); a component dragged onto an edge (on the
//! layout page itself, its own editor played on, a teaching tip beside the window); search and the preview called up
//! by their shortcuts (in the dialog again). Each step says so once the user has done it themselves. Then a word that
//! it is done.
//!
//! It is the settings window's, drawn on its thread with its desktop and its panels (see `Ui::panel_on_stage`): the
//! window stays up all through it, the dialog over it in its middle (the window covered under it and taking nothing
//! meanwhile, as Windows 11 has a dialog over a window), the tip beside it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::D2D1_LAYER_PARAMETERS1;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MonitorFromWindow, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, IsWindowVisible, IsZoomed, LoadCursorW, LoadImageW, PostMessageW, RegisterClassExW, SetForegroundWindow,
    SetWindowPos, ShowWindow, HICON, HTCAPTION, HTCLIENT, IDC_ARROW, IMAGE_ICON, LR_SHARED, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOVING, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP, WS_SYSMENU,
};

use super::*;
use crate::settings::Places;
use crate::ui::motion::{Easing, LINEAR};

/// The dialog: its width, the room round what is in it, the small desktop's height, its footer, a button's height and
/// the least width of one, and a shortcut's row; the room round it in its window for its shadow (and for it to grow
/// into as it comes and goes: see `Change`).
const WIDE: f32 = 640.0;
const PAD: f32 = 24.0;
const DESK: f32 = 300.0;
const FOOTER: f32 = 64.0;
const BUTTON: f32 = 32.0;
const BUTTON_LEAST: f32 = 96.0;
const KEY_ROW: f32 = 52.0;
const ROOM: f32 = 40.0;
/// The teaching tip: its width, the room inside it, its beak (how far it stands out), the room round it for its shadow,
/// and how far from the window it stands.
const TIP_WIDE: f32 = 340.0;
const TIP_PAD: f32 = 16.0;
const BEAK: f32 = 10.0;
const SHADOW: f32 = 24.0;
const TIP_OFF: f32 = 6.0;
/// How soon it is drawn again while anything in it plays, and how long it is left without it.
const FRAME: Duration = Duration::from_millis(15);
const IDLE: Duration = Duration::from_secs(3600);

const TITLE: Font = Font::new(Family::Segoe, 20.0, 600.0);
const TIP_TITLE: Font = Font::new(Family::Segoe, 15.0, 600.0);
const BODY: Font = Font::new(Family::Segoe, 14.0, 400.0);
const SMALL: Font = Font::new(Family::Segoe, 12.0, 400.0);
const STATE: Font = Font::new(Family::Segoe, 13.0, 400.0);

/// How the pointer glides in the plays, and how the panel slides out and the look grows, as Glance's own do.
const GLIDE: Easing = Easing(0.45, 0.05, 0.3, 1.0);
const SLIDE_OUT: Easing = Easing(0.2, 0.9, 0.25, 1.04);
const GROW: Easing = Easing(0.2, 0.9, 0.25, 1.05);

/// How the guide changes from one step to the next (ms), as Windows 11 has its own dialogs come and go: the dialog
/// coming from a little larger and going a little larger, fading; its content sliding out the way it is left and the
/// next one in from the other side, the dialog growing or shrinking to it meanwhile; between the dialog and the tip, its
/// plate carried at once to where the other stands and made it, what it showed fading out on it as it starts and what
/// the other shows fading in on it after (never both at once); the cover over the window coming and going with the
/// dialog. Shown by the guide, the window opens first, as Windows has it, and
/// the dialog comes after.
const DIALOG_IN: f32 = 260.0;
const DIALOG_END: f32 = 160.0;
const CONTENT_OUT: f32 = 120.0;
const CONTENT_IN: f32 = 300.0;
const CONTENT_IN_AFTER: f32 = 40.0;
const GROWS: f32 = 320.0;
const CARRIED: f32 = 500.0;
const CARRIED_OUT: f32 = 150.0;
const COVER_IN: f32 = 200.0;
const COVER_OUT: f32 = 200.0;
const WINDOW_LEAD: f32 = 200.0;
/// How dark the cover over the window is at its darkest (black's opacity), as Windows 11's smoke under a dialog.
const COVER: f32 = 0.3;
/// How far the content slides (DIPs).
const SLIDE: f32 = 28.0;
/// The curves for what comes (slowing to rest, as Windows 11's), what goes (gathering speed, never so fast at its end
/// that it leaps out of sight) and what moves.
const DECELERATE: Easing = Easing(0.1, 0.9, 0.2, 1.0);
const ACCELERATE: Easing = Easing(0.7, 0.0, 0.84, 0.0);
const MOVE: Easing = Easing(0.3, 0.0, 0.12, 1.0);

/// The first step's play (ms): the pointer at rest, gliding to the edge, the panel sliding out as it is pushed, the
/// pointer back where it began, the panel put away as it leaves it.
const PUSH_LOOP: u64 = 5300;

/// How many steps there are (the word that it is done is none of them).
const STEPS: usize = 5;

/// The second step's play (ms), and the sizes it makes the panel; when it starts to make it smaller, and when the
/// corner is let go; when the pointer moves off the corner, how long it takes to go there and back, and when it comes
/// back; when each click of the double-click goes down, how long one is held, and how long the ring each leaves shows;
/// when the pointer goes back onto the panel.
const SIZE_LOOP: u64 = 9400;
const GROWN: f32 = 1.18;
const SHRUNK: f32 = 0.8;
const SHRINK_AT: f32 = 2800.0;
const LET_GO: f32 = 4000.0;
const ASIDE_AT: f32 = LET_GO + 150.0;
const AWAY: f32 = 400.0;
const RETURN_AT: f32 = ASIDE_AT + AWAY + SIZE_REST;
const CLICKS: [f32; 2] = [RETURN_AT + AWAY + 150.0, RETURN_AT + AWAY + 330.0];
const CLICK: f32 = 60.0;
const RING: f32 = 360.0;
const BACK_AT: f32 = CLICKS[1] + crate::panel::RESTORE.as_millis() as f32 + SIZE_REST + 200.0;
/// How long the second step's play rests off the corner before it comes back to double-click it, and after the panel
/// is back to its own size (ms), so that each is seen as a step of its own.
const SIZE_REST: f32 = 1200.0;
/// How the panel follows the hand in the second step's play: as a hand moves, gathering pace and slowing to rest.
const SIZE_EASE: Easing = Easing(0.4, 0.0, 0.2, 1.0);

/// The third step's play (ms).
const TEAR_LOOP: u64 = 9200;

/// What a step asks, done elsewhere: a panel opened by a push into an edge, search called up, something looked at
/// larger, a panel made larger or smaller by hand, a widget torn off a panel.
#[derive(Clone, Copy)]
pub(crate) enum Saw {
    Pushed,
    Searched,
    Looked,
    Sized,
    Torn,
}

impl Saw {
    fn bit(self) -> usize {
        match self {
            Saw::Pushed => 1,
            Saw::Searched => 2,
            Saw::Looked => 4,
            Saw::Sized => 8,
            Saw::Torn => 16,
        }
    }
}

/// Whether a guide is up (told of what is done elsewhere only then), and what it was told of since it last looked.
static UP: AtomicBool = AtomicBool::new(false);
static TOLD: AtomicUsize = AtomicUsize::new(0);

/// Tells the guide, where one is up, that `saw` was done: from any thread.
pub(crate) fn saw(saw: Saw) {
    if !UP.load(Ordering::Acquire) {
        return;
    }
    TOLD.fetch_or(saw.bit(), Ordering::AcqRel);
    if let Some(hwnd) = super::hwnd() {
        let _ = unsafe { PostMessageW(Some(hwnd), GUIDE_TOLD, WPARAM(0), LPARAM(0)) };
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Push,
    Size,
    Tear,
    Layout,
    Keys,
    /// The word that it is done.
    Ready,
}

impl Step {
    fn after(self) -> Option<Step> {
        match self {
            Step::Push => Some(Step::Size),
            Step::Size => Some(Step::Tear),
            Step::Tear => Some(Step::Layout),
            Step::Layout => Some(Step::Keys),
            Step::Keys => Some(Step::Ready),
            Step::Ready => None,
        }
    }

    fn before(self) -> Option<Step> {
        match self {
            Step::Push => None,
            Step::Size => Some(Step::Push),
            Step::Tear => Some(Step::Size),
            Step::Layout => Some(Step::Tear),
            Step::Keys => Some(Step::Layout),
            Step::Ready => Some(Step::Keys),
        }
    }

    /// Its place among the steps (the word that it is done has none).
    fn index(self) -> Option<usize> {
        (self != Step::Ready).then_some(self as usize)
    }
}

/// What may be pressed in its windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Skip,
    Back,
    Next,
    /// A shortcut's row on the last step: search's, the preview's.
    Search,
    Look,
}

/// One of its windows: the dialog, or the teaching tip.
struct Popup {
    hwnd: HWND,
    surface: Option<Surface>,
    scale: f32,
    /// Its size (DIPs): the tip's with the room for its shadow round it.
    size: (f32, f32),
    /// What may be pressed, and the pointer, from `hits_from` in it (DIPs: the dialog's from its plate's corner).
    hits: Vec<(Rect, Hit)>,
    hits_from: (f32, f32),
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    /// Its plate in it (DIPs), and the tip's beak: the side it is on and how far down that side it is; none where it
    /// stands over the window, pointing at nothing.
    plate: Rect,
    beak: Option<(Edge, f32)>,
}

impl Popup {
    fn hovered(&self) -> Option<Hit> {
        let (x, y) = self.pointer?;
        self.hits.iter().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, hit)| *hit)
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Guide {
    /// What its popups draw on let go of, with the window's (see `Ui::let_go_of_shown`): made again on the device the
    /// window draws with next.
    pub(super) fn let_go_of_surfaces(&mut self) {
        for popup in [&mut self.dialog, &mut self.tip].into_iter().flatten() {
            popup.surface = None;
        }
    }
}

pub(super) struct Guide {
    step: Step,
    /// Each step done by the user themselves.
    done: [bool; STEPS],
    /// Search and the preview called up, on the last step.
    tried: [bool; 2],
    /// Whether the window was up as it began (from the general page): shown again there as it ends, else closed.
    was_up: bool,
    /// When the step shown began to play.
    since: Instant,
    /// The edge the step shown has the panel on, and what that edge opens: the first step's opened from the right,
    /// where that opens anything, else the first that does; the second's and third's the readings', where they are on
    /// an edge, else the right.
    edge: (Edge, Option<Part>),
    dialog: Option<Popup>,
    tip: Option<Popup>,
    /// The change of step under way.
    change: Option<Change>,
    /// The play on the layout page, while it plays.
    drag: Option<Drag>,
    /// The skin's ground under search and under the look, in the small desktop.
    grounds: [render::GroundLayers; 2],
    _showing: crate::ui::life::Showing,
}

impl Drop for Guide {
    /// (Gone however it goes, the window closed under it among them: told of nothing more.)
    fn drop(&mut self) {
        UP.store(false, Ordering::Release);
    }
}

/// The play on the layout page: a component carried from those beside the editor onto an edge of its screen, then
/// what the edge held carried back onto it (where it held none, what was put there carried off it), over and over.
/// The edges it changes are the window's alone, never saved, and put back as they were as it stops.
pub(super) struct Drag {
    before: Places,
    screen: String,
    moves: Vec<Move>,
    /// The move it is on, since when, and where the pointer was as it began; the pointer now (none till it shows).
    at: usize,
    since: Instant,
    from: Option<(f32, f32)>,
    pointer: Option<(f32, f32)>,
}

/// Where the pointer goes in the play on the layout page: where it first shows (the foot of the editor), a component
/// beside the editor, the edge played on, the component on it, the middle of its screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Rest,
    Tile(Part),
    Edge,
    Pill,
    Middle,
}

/// What a move does once the pointer is there: takes up a component (from the edge played on, or from beside the
/// editor), lets go what it carries, or nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Take(Part, bool),
    Put,
    Wait,
}

/// The pointer gliding to `to` for `glide` ms, then `wait` ms there, then `act` done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Move {
    to: Spot,
    glide: u64,
    wait: u64,
    act: Act,
}

/// The layout page's play, once round, for `put` carried onto the edge that holds `held`: it ends with the edge as it
/// began, and each round glides on from where the last left the pointer.
fn script(put: Part, held: Option<Part>) -> Vec<Move> {
    let mut moves = vec![
        Move { to: Spot::Tile(put), glide: 900, wait: 150, act: Act::Take(put, false) },
        Move { to: Spot::Edge, glide: 900, wait: 350, act: Act::Put },
        Move { to: Spot::Edge, glide: 0, wait: 1900, act: Act::Wait },
    ];
    match held {
        Some(held) => moves.extend([
            Move { to: Spot::Tile(held), glide: 900, wait: 150, act: Act::Take(held, false) },
            Move { to: Spot::Edge, glide: 900, wait: 350, act: Act::Put },
            Move { to: Spot::Edge, glide: 0, wait: 1900, act: Act::Wait },
        ]),
        None => moves.extend([
            Move { to: Spot::Pill, glide: 900, wait: 150, act: Act::Take(put, true) },
            Move { to: Spot::Middle, glide: 900, wait: 350, act: Act::Put },
            Move { to: Spot::Middle, glide: 0, wait: 1200, act: Act::Wait },
        ]),
    }
    moves
}

/// What the layout page's play carries onto an edge that holds `held`: another component than that.
fn played_part(held: Option<Part>) -> Part {
    [Part::Music, Part::Monitor, Part::Workbench].into_iter().find(|&part| Some(part) != held).unwrap_or(Part::Music)
}

/// How far along `ms` is of what begins at `from` and lasts `length` (0 before, 1 after), eased by `easing`.
fn eased(ms: f32, from: f32, length: f32, easing: Easing) -> f32 {
    easing.at(((ms - from) / length).clamp(0.0, 1.0))
}

/// The first step's play, `t` into it: how far along the pointer is from where it rests to the edge (0 to 1), and how
/// far the panel is slid out (0 to 1). At its end as at its start, so that it plays on round with nothing jumping.
fn pushing(t: Duration) -> (f32, f32) {
    let ms = (t.as_millis() as u64 % PUSH_LOOP) as f32;
    let along = eased(ms, 500.0, 1100.0, GLIDE) - eased(ms, 3800.0, 1000.0, GLIDE);
    let out = eased(ms, 1800.0, 420.0, SLIDE_OUT) - eased(ms, 4150.0, 420.0, SLIDE_OUT);
    (along, out)
}

/// Where the pointer of a play is: at `from`, gliding from it to `to` (`along`, 0 to 1), or at `to`.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Hand<M> {
    from: M,
    to: M,
    along: f32,
}

impl<M: Copy> Hand<M> {
    fn at(mark: M) -> Self {
        Hand { from: mark, to: mark, along: 0.0 }
    }

    fn gliding(from: M, to: M, along: f32) -> Self {
        Hand { from, to, along }
    }

    /// Where it is, `place` saying where each mark is.
    fn point(self, mut place: impl FnMut(M) -> (f32, f32)) -> (f32, f32) {
        let (from, to) = (place(self.from), place(self.to));
        (from.0 + (to.0 - from.0) * self.along, from.1 + (to.1 - from.1) * self.along)
    }
}

/// Where the pointer goes in the second step's play: on the panel as large as it opens, on its free corner (the one
/// away from its edge, at its foot) as large as it is drawn at, and on the panel a little in from that corner.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SizeMark {
    Inside,
    Corner(f32),
    Aside(f32),
}

/// The second step's play at a moment: how large the panel is, where the pointer is, whether it is held down, whether
/// it shows as the pointer that sizes, and how far along a click's ring is (0 to 1), where one shows.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Sizing {
    size: f32,
    hand: Hand<SizeMark>,
    held: bool,
    sizes: bool,
    ring: Option<f32>,
}

/// The second step's play, `t` into it: the pointer gliding from the panel to its free corner, the pointer that sizes
/// shown once it is there; held down, the panel made larger, then smaller, following the hand (laid out in other
/// columns on the way, it turns into them as the panel does: see `render::Turn`), each resting a moment; let go, the
/// pointer moved off the corner onto the panel, a plain pointer again, resting there a while, then back to double-click
/// the corner as a plain pointer (a step of its own, not the sizing carried on), the panel back to its own size as
/// Glance has it (see `panel::RESTORE`), the pointer as it was once it is; a while after, the pointer back on the panel.
/// At its end as at its start, round and round.
fn sizing(t: Duration) -> Sizing {
    let ms = (t.as_millis() as u64 % SIZE_LOOP) as f32;
    let restore = crate::panel::RESTORE.as_secs_f32() * 1000.0;
    let restored = CLICKS[1] + restore;
    let ring = CLICKS.iter().rev().map(|&click| ms - click).find(|since| (0.0..RING).contains(since)).map(|since| since / RING);
    let corner = |size: f32| Sizing { size, hand: Hand::at(SizeMark::Corner(size)), held: true, sizes: true, ring: None };
    if ms < 200.0 {
        Sizing { size: 1.0, hand: Hand::at(SizeMark::Inside), held: false, sizes: false, ring: None }
    } else if ms < 1200.0 {
        Sizing { size: 1.0, hand: Hand::gliding(SizeMark::Inside, SizeMark::Corner(1.0), eased(ms, 200.0, 1000.0, GLIDE)), held: false, sizes: false, ring: None }
    } else if ms < 1400.0 {
        corner(1.0)
    } else if ms < SHRINK_AT {
        corner(1.0 + (GROWN - 1.0) * eased(ms, 1400.0, 1000.0, SIZE_EASE))
    } else if ms < LET_GO {
        corner(GROWN + (SHRUNK - GROWN) * eased(ms, SHRINK_AT, 1000.0, SIZE_EASE))
    } else if ms < ASIDE_AT {
        Sizing { size: SHRUNK, hand: Hand::at(SizeMark::Corner(SHRUNK)), held: false, sizes: true, ring: None }
    } else if ms < RETURN_AT {
        let along = eased(ms, ASIDE_AT, AWAY, GLIDE);
        let hand = if along < 1.0 { Hand::gliding(SizeMark::Corner(SHRUNK), SizeMark::Aside(SHRUNK), along) } else { Hand::at(SizeMark::Aside(SHRUNK)) };
        Sizing { size: SHRUNK, hand, held: false, sizes: false, ring: None }
    } else if ms < restored {
        // (Made its own size again as the second click goes down, as a double-click is.)
        let size = if ms < CLICKS[1] { SHRUNK } else { crate::panel::restored(SHRUNK, 1.0, (ms - CLICKS[1]) / restore) };
        let held = CLICKS.iter().any(|&click| (click..click + CLICK).contains(&ms));
        let along = eased(ms, RETURN_AT, AWAY, GLIDE);
        let hand = if along < 1.0 { Hand::gliding(SizeMark::Aside(SHRUNK), SizeMark::Corner(SHRUNK), along) } else { Hand::at(SizeMark::Corner(SHRUNK)) };
        Sizing { size, hand, held, sizes: false, ring }
    } else if ms < BACK_AT {
        Sizing { size: 1.0, hand: Hand::at(SizeMark::Corner(SHRUNK)), held: false, sizes: false, ring }
    } else {
        Sizing { size: 1.0, hand: Hand::gliding(SizeMark::Corner(SHRUNK), SizeMark::Inside, eased(ms, BACK_AT, 1000.0, GLIDE)), held: false, sizes: false, ring: None }
    }
}

/// Where the pointer goes in the third step's play: on the panel, on its first lane's head, on its foot, on the
/// screen's edge it opens from, where the lane torn off is put down, and where the whole panel is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TearMark {
    Inside,
    Head,
    Foot,
    Edge,
    LaneDrop,
    WholeDrop,
}

/// The third step's play at a moment: how far the panel is slid out, where the pointer is, whether it is held down,
/// and each widget torn off (the lane's, the whole panel's) where it shows: where the pointer holds it by (where it
/// was taken), and how opaque it is.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Tearing {
    out: f32,
    hand: Hand<TearMark>,
    held: bool,
    lane: Option<(Hand<TearMark>, f32)>,
    whole: Option<(Hand<TearMark>, f32)>,
}

/// The third step's play, `t` into it: the pointer on the first lane's head, held, pulled off: the lane a widget under
/// the hand, the panel gone once it shows, as Glance has it; put down. The panel pushed out again, held by its foot
/// and pulled off whole, put down. Both widgets faded, the panel slid out, the pointer back on it, round
/// and round.
fn tearing(t: Duration) -> Tearing {
    let ms = (t.as_millis() as u64 % TEAR_LOOP) as f32;
    let fade = 1.0 - eased(ms, 7000.0, 500.0, LINEAR);
    let mut play = Tearing { out: 1.0, hand: Hand::at(TearMark::Inside), held: false, lane: None, whole: None };
    if ms < 300.0 {
    } else if ms < 1100.0 {
        play.hand = Hand::gliding(TearMark::Inside, TearMark::Head, eased(ms, 300.0, 800.0, GLIDE));
    } else if ms < 1300.0 {
        (play.hand, play.held) = (Hand::at(TearMark::Head), true);
    } else if ms < 2300.0 {
        (play.hand, play.held) = (Hand::gliding(TearMark::Head, TearMark::LaneDrop, eased(ms, 1300.0, 1000.0, GLIDE)), true);
        // (Torn off a few DIPs on, the panel gone once the widget is up.)
        if ms >= 1340.0 {
            play.lane = Some((play.hand, 1.0));
        }
        if ms >= 1420.0 {
            play.out = 0.0;
        }
    } else {
        play.lane = Some((Hand::at(TearMark::LaneDrop), fade));
        play.out = 0.0;
        if ms < 2500.0 {
            play.hand = Hand::at(TearMark::LaneDrop);
        } else if ms < 3400.0 {
            play.hand = Hand::gliding(TearMark::LaneDrop, TearMark::Edge, eased(ms, 2500.0, 900.0, GLIDE));
        } else if ms < 3900.0 {
            (play.hand, play.out) = (Hand::at(TearMark::Edge), eased(ms, 3450.0, 420.0, SLIDE_OUT));
        } else if ms < 4800.0 {
            (play.hand, play.out) = (Hand::gliding(TearMark::Edge, TearMark::Foot, eased(ms, 3900.0, 900.0, GLIDE)), 1.0);
        } else if ms < 5000.0 {
            (play.hand, play.out, play.held) = (Hand::at(TearMark::Foot), 1.0, true);
        } else if ms < 6000.0 {
            (play.hand, play.held) = (Hand::gliding(TearMark::Foot, TearMark::WholeDrop, eased(ms, 5000.0, 1000.0, GLIDE)), true);
            play.out = if ms >= 5120.0 { 0.0 } else { 1.0 };
            if ms >= 5040.0 {
                play.whole = Some((play.hand, 1.0));
            }
        } else {
            play.whole = Some((Hand::at(TearMark::WholeDrop), fade));
            if ms >= 7000.0 {
                (play.hand, play.out) = (Hand::gliding(TearMark::WholeDrop, TearMark::Inside, eased(ms, 7000.0, 1200.0, GLIDE)), eased(ms, 7400.0, 420.0, SLIDE_OUT));
            } else {
                play.hand = Hand::at(TearMark::WholeDrop);
            }
        }
    }
    // (Faded all the way: gone.)
    play.lane = play.lane.filter(|(_, opacity)| *opacity > 0.0);
    play.whole = play.whole.filter(|(_, opacity)| *opacity > 0.0);
    play
}

/// The last step's play for shortcuts of `keys` keys (search's, the preview's), once round: when its keys go down one
/// after another and search shows, then the preview's and the look shows, then both go; how long it is (ms).
struct Keying {
    keys: (usize, usize),
    search_on: f32,
    look_on: f32,
    look_off: f32,
    search_off: f32,
    end: f32,
}

/// How long a key of the play stays down after the last of its shortcut's goes down, and how far apart they go down.
const KEY_HELD: f32 = 170.0;
const KEY_APART: f32 = 110.0;

impl Keying {
    fn new(keys: (usize, usize)) -> Self {
        let search_on = 600.0 + keys.0 as f32 * KEY_APART;
        let look_on = search_on + KEY_HELD + 1300.0 + keys.1 as f32 * KEY_APART;
        let look_off = look_on + KEY_HELD + 2100.0;
        let search_off = look_off + 250.0;
        Keying { keys, search_on, look_on, look_off, search_off, end: search_off + 300.0 }
    }

    /// `t` into it, round and round: how many of each shortcut's keys are down, and how far search and the look are up
    /// (0 to 1).
    fn at(&self, t: Duration) -> ([usize; 2], f32, f32) {
        let ms = (t.as_millis() as u64 % self.end as u64) as f32;
        let down = |first: f32, count: usize, up: f32| if ms >= first && ms < up { (((ms - first) / KEY_APART).floor() as usize + 1).min(count) } else { 0 };
        let search_first = self.search_on - self.keys.0 as f32 * KEY_APART;
        let look_first = self.look_on - self.keys.1 as f32 * KEY_APART;
        let caps = [down(search_first, self.keys.0, self.search_on + KEY_HELD), down(look_first, self.keys.1, self.look_on + KEY_HELD)];
        let search = eased(ms, self.search_on, 200.0, LINEAR) - eased(ms, self.search_off, 200.0, LINEAR);
        let look = eased(ms, self.look_on, 320.0, GROW) - eased(ms, self.look_off, 180.0, LINEAR);
        (caps, search, look)
    }
}

/// A change of the guide under way: it coming up, it going, or from one step to another, within the dialog, from it to
/// the window and the tip beside it, or back.
struct Change {
    kind: Kind,
    from: Step,
    /// When its first frame was drawn (none till it is: its clock starts once it shows, not as it is asked for, so
    /// that a slow first frame, windows made or a page turned to, does not eat into it).
    began: Option<Instant>,
    /// Going back (the content slides the other way).
    back: bool,
    /// When the step it changes from began to play (played on as it goes).
    from_since: Instant,
    /// How tall the dialog is for the step it changes from and for the one it changes to (DIPs).
    heights: (f32, f32),
    /// How long the guide coming up waits for the window to open (ms): none, where it was up.
    lead: f32,
    /// Whether the window has been drawn since it began (as what it changes to has it: its page turned to, the edges
    /// the layout's play changed put back), after which it is left as it is till it is over (see
    /// `guide_holds_window`).
    window_drawn: bool,
    /// Done at once (Windows' animations off).
    quick: bool,
}

/// What a change of the guide is: it coming up, it going, or from one step to another within the dialog, from the
/// dialog to the tip, or back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Opening,
    Ending,
    Swap,
    ToTip,
    FromTip,
}

impl Change {
    fn new(kind: Kind, from: Step, to: Step, from_since: Instant, heights: (f32, f32)) -> Change {
        Change { kind, from, began: None, back: from.before() == Some(to), from_since, heights, lead: 0.0, window_drawn: false, quick: crate::panel::reduced_motion() }
    }

    /// The change of step from `from` to `to`.
    fn between(from: Step, to: Step, from_since: Instant, heights: (f32, f32)) -> Change {
        let kind = if to == Step::Layout {
            Kind::ToTip
        } else if from == Step::Layout {
            Kind::FromTip
        } else {
            Kind::Swap
        };
        Change::new(kind, from, to, from_since, heights)
    }

    /// How long it lasts (ms).
    fn length(&self) -> f32 {
        match self.kind {
            Kind::Opening => self.lead + DIALOG_IN.max(COVER_IN),
            Kind::Ending => DIALOG_END.max(COVER_OUT),
            Kind::Swap => (CONTENT_OUT + GROWS).max(CONTENT_OUT + CONTENT_IN_AFTER + CONTENT_IN),
            Kind::ToTip | Kind::FromTip => CARRIED,
        }
    }

    /// How far into it `now` is (ms): done at once, all of it.
    fn ms(&self, now: Instant) -> f32 {
        if self.quick {
            f32::INFINITY
        } else {
            self.began.map_or(0.0, |began| now.saturating_duration_since(began).as_secs_f32() * 1000.0)
        }
    }

}

/// How the dialog shows: how opaque and how large (about its middle) it is, how tall (DIPs), whether it shows the step
/// changed from (else the one shown now), how far its content is slid (DIPs) and how opaque that is, how opaque its
/// footer is, and the buttons on it; between it and the tip, how far its plate is carried to the tip's (0 to 1) and
/// how opaque the tip's content is on it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct DialogLook {
    opacity: f32,
    scale: f32,
    height: f32,
    old: bool,
    slide: f32,
    content: f32,
    footer: f32,
    buttons: f32,
    carried: f32,
    tip: f32,
}

/// How the dialog shows `ms` into `change` (none: at rest), on `step`, `height` tall at rest: none, not shown (the tip
/// shows at rest on the layout's step, in its own window).
fn dialog_look(change: Option<(&Change, f32)>, step: Step, height: f32) -> Option<DialogLook> {
    let rest = DialogLook { opacity: 1.0, scale: 1.0, height, old: false, slide: 0.0, content: 1.0, footer: 1.0, buttons: 1.0, carried: 0.0, tip: 0.0 };
    let Some((change, ms)) = change else { return (step != Step::Layout).then_some(rest) };
    // (Between the dialog and the tip: the plate carried all along, what it shows going evenly as it starts (no later
    // than the plate shrinks under it), what the other shows coming after.)
    let carried = eased(ms, 0.0, CARRIED, MOVE);
    let emptied = eased(ms, 0.0, CARRIED_OUT, LINEAR);
    let filled = eased(ms, CARRIED_OUT, CARRIED - CARRIED_OUT, DECELERATE);
    match change.kind {
        Kind::Opening => (ms >= change.lead).then(|| {
            let t = eased(ms, change.lead, DIALOG_IN, DECELERATE);
            DialogLook { opacity: t, scale: 1.0 + 0.05 * (1.0 - t), ..rest }
        }),
        Kind::Ending => {
            let t = eased(ms, 0.0, DIALOG_END, ACCELERATE);
            Some(DialogLook { opacity: 1.0 - t, scale: 1.0 + 0.05 * t, ..rest })
        }
        Kind::ToTip => Some(DialogLook { height: change.heights.0, old: true, content: 1.0 - emptied, footer: 1.0 - emptied, buttons: 1.0 - emptied, carried, tip: filled, ..rest }),
        Kind::FromTip => Some(DialogLook { content: filled, footer: filled, buttons: filled, carried: 1.0 - carried, tip: 1.0 - emptied, ..rest }),
        Kind::Swap => {
            let way = if change.back { -1.0 } else { 1.0 };
            let (from, to) = change.heights;
            // (The buttons changed with the content where the steps have others, else left as they are.)
            let same = buttons(change.from, Lang::En) == buttons(step, Lang::En);
            Some(if ms < CONTENT_OUT {
                let out = eased(ms, 0.0, CONTENT_OUT, ACCELERATE);
                DialogLook { height: from, old: true, slide: -SLIDE * way * out, content: 1.0 - out, buttons: if same { 1.0 } else { 1.0 - out }, ..rest }
            } else {
                let grown = eased(ms, CONTENT_OUT, GROWS, MOVE);
                let come = eased(ms, CONTENT_OUT + CONTENT_IN_AFTER, CONTENT_IN, DECELERATE);
                DialogLook { height: from + (to - from) * grown, slide: SLIDE * way * (1.0 - come), content: come, buttons: if same { 1.0 } else { come }, ..rest }
            })
        }
    }
}

/// Whether the tip shows in its own window: at rest on the layout's step (as it changes, it is the dialog's to draw).
fn tip_rests(change: Option<&Change>, step: Step) -> bool {
    change.is_none() && step == Step::Layout
}

/// Whether the window under the guide is left as it was last drawn (see `Ui::guide_holds_window`): through a change
/// once it has been drawn for it; at rest under the dialog, unless something it shows has come in since it was drawn
/// (`taken_unseen`).
fn holds(change: Option<&Change>, step: Step, taken_unseen: bool) -> bool {
    match change {
        Some(change) => change.window_drawn,
        None => step != Step::Layout && !taken_unseen,
    }
}

/// The change under way, its first frame drawn: its clock started, now.
fn start_clock(guide: &mut Guide) {
    if let Some(change) = guide.change.as_mut() {
        change.began.get_or_insert_with(Instant::now);
    }
}

/// How covered the window is `ms` into `change` (none: at rest), on `step` (0, not at all, to 1, at its darkest): as
/// long as the dialog is over it. Evenly darkened and lightened: all the window darkening at once, any steeper reads as
/// a jump.
fn cover_look(change: Option<(&Change, f32)>, step: Step) -> f32 {
    let Some((change, ms)) = change else { return if step == Step::Layout { 0.0 } else { 1.0 } };
    match change.kind {
        Kind::Opening => eased(ms, change.lead, COVER_IN, LINEAR),
        Kind::Ending => 1.0 - eased(ms, 0.0, COVER_OUT, LINEAR),
        Kind::Swap => 1.0,
        // (Going as the plate is carried to the tip, coming as it is carried back.)
        Kind::ToTip => 1.0 - eased(ms, 0.0, CARRIED, LINEAR),
        Kind::FromTip => eased(ms, 0.0, CARRIED, LINEAR),
    }
}

/// `hwnd` shown or hidden as Windows shows and hides its windows, its animation of that played.
fn shown_by_system(hwnd: HWND, show: bool) {
    let off = BOOL::from(false);
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const _ as *const _, 4);
        let _ = ShowWindow(hwnd, if show { SW_SHOW } else { SW_HIDE });
    }
}

/// `hwnd` placed at `at` (physical px), where it is not there already.
fn place(hwnd: HWND, at: RECT) {
    let mut was = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut was);
        if was != at {
            let _ = SetWindowPos(hwnd, None, at.left, at.top, at.right - at.left, at.bottom - at.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

/// The least rectangle holding both.
fn union(a: RECT, b: RECT) -> RECT {
    RECT { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) }
}

/// `r` (physical px) from `corner` in DIPs at `scale`.
fn dips(r: RECT, corner: (i32, i32), scale: f32) -> Rect {
    Rect { x: (r.left - corner.0) as f32 / scale, y: (r.top - corner.1) as f32 / scale, w: (r.right - r.left) as f32 / scale, h: (r.bottom - r.top) as f32 / scale }
}

/// `a` carried `t` of the way (0 to 1) to `b`.
fn carried(a: Rect, b: Rect, t: f32) -> Rect {
    let at = |a: f32, b: f32| a + (b - a) * t;
    Rect { x: at(a.x, b.x), y: at(a.y, b.y), w: at(a.w, b.w), h: at(a.h, b.h) }
}

/// The colours of its windows besides the settings window's own: a dialog's ground and its footer's, a teaching tip's,
/// a button's (its ground, under the pointer, its outline), a tick's green, and the shadow round the tip, as Windows 11
/// has them.
struct Grounds {
    dialog: Color,
    footer: Color,
    flyout: Color,
    button: Color,
    button_hover: Color,
    button_stroke: Color,
    done: Color,
    shadow: Color,
}

impl Grounds {
    fn new(dark: bool) -> Self {
        let white = |a| Color::hex(0xFFFFFF, a);
        let black = |a| Color::hex(0x000000, a);
        if dark {
            Grounds { dialog: Color::hex(0x2B2B2B, 1.0), footer: Color::hex(0x202020, 1.0), flyout: Color::hex(0x2C2C2C, 1.0), button: white(0.06), button_hover: white(0.09), button_stroke: white(0.07), done: Color::hex(0x6CCB5F, 1.0), shadow: black(0.4) }
        } else {
            Grounds { dialog: white(1.0), footer: Color::hex(0xF3F3F3, 1.0), flyout: Color::hex(0xF9F9F9, 1.0), button: white(1.0), button_hover: Color::hex(0xF6F6F6, 1.0), button_stroke: black(0.1), done: Color::hex(0x0F7B0F, 1.0), shadow: black(0.16) }
        }
    }
}

/// What the guide says of each step: its title, what it asks, what it says till that is done and once it is.
struct Words {
    title: &'static str,
    body: String,
    wait: String,
    done: &'static str,
}

fn words(step: Step, edge: Edge, lang: Lang) -> Words {
    let p = |zh: &'static str, en: &'static str| pick(lang, zh, en);
    match step {
        Step::Push => {
            let side = match edge {
                Edge::Right => p("右", "right"),
                Edge::Left => p("左", "left"),
                Edge::Top => p("上", "top"),
            };
            let (body, wait) = match lang {
                Lang::Zh => (format!("将鼠标移到屏幕{side}边缘，再向外推动一下。"), format!("现在请在屏幕{side}边缘试一次")),
                Lang::En => (format!("Move the pointer to the {side} edge of the screen, then push a little further."), format!("Now try it at the {side} edge")),
            };
            Words { title: p("从屏幕边缘呼出面板", "Open a panel from the screen's edge"), body, wait, done: p("面板已呼出，鼠标移开后自动收起", "The panel is open. It closes as the pointer leaves it") }
        }
        Step::Size => Words {
            title: p("调整面板大小", "Resize the panel"),
            body: p(
                "拖动面板的任意边角即可放大或缩小，双击边角恢复默认大小。",
                "Drag any edge or corner of the panel to make it larger or smaller. Double-click an edge or corner to restore its default size.",
            )
            .into(),
            wait: p("现在请呼出面板，拖动边角试一次", "Now open a panel and drag an edge or corner").into(),
            done: p("已调整大小", "The panel is resized"),
        },
        Step::Tear => Words {
            title: p("拖出成为小组件", "Drag out a widget"),
            body: p(
                "按住某一块的标题拖出，这一块单独成为小组件。按住面板其他地方拖出，整个面板成为小组件。",
                "Hold a block by its title and drag it out to make that block a widget of its own. Hold the panel anywhere else and drag it out to make the whole panel a widget.",
            )
            .into(),
            wait: p("现在请呼出面板，拖出一个小组件试一次", "Now open a panel and drag out a widget").into(),
            done: p("小组件已拖出", "A widget is dragged out"),
        },
        Step::Layout => Words {
            title: p("将组件放到屏幕边缘", "Put components on the screen's edges"),
            body: p(
                "将左侧的组件拖到右侧屏幕的任意一条边缘，推动该边缘即可呼出它。边缘上的组件也可拖到另一条边缘，两者互换位置。",
                "Drag a component on the left onto any edge of the screen on the right, then push that edge to open it. A component on an edge can be dragged onto another, and the two change places.",
            )
            .into(),
            wait: p("现在请拖动一个组件", "Now drag a component").into(),
            done: p("已调整，推动该边缘即可呼出", "Done. Push that edge to open it"),
        },
        Step::Keys => Words { title: p("两个快捷键", "Two shortcuts"), body: String::new(), wait: p("请点击按键各试一次", "Click each to try it once").into(), done: p("两个快捷键均已试过", "Both shortcuts are tried") },
        Step::Ready => Words { title: p("准备就绪", "All set"), body: p("此引导可在通用页中再次查看。", "You can see this guide again on the General page.").into(), wait: String::new(), done: "" },
    }
}

/// A shortcut's keys, each its own cap (none set: one saying so).
fn caps(shortcut: Option<crate::settings::Shortcut>, lang: Lang) -> Vec<String> {
    match shortcut {
        Some(shortcut) => crate::tray::shortcut_name(shortcut).split('+').map(str::to_string).collect(),
        None => vec![pick(lang, "未设置", "None").to_string()],
    }
}

/// Its windows' class (the dialog's and the tip's).
const CLASS: PCWSTR = w!("GlanceGuide");

fn register() {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let icon = |size| LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_SHARED).ok().map(|h| HICON(h.0));
        // (Registered once; again is refused, which is as well.)
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon(GetSystemMetrics(SM_CXICON)).unwrap_or_default(),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)).unwrap_or_default(),
            ..Default::default()
        });
    }
}

/// The work area of the screen `hwnd` is on (physical px).
fn work_of(hwnd: HWND) -> RECT {
    let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut screen);
    }
    screen.rcWork
}

fn point(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // All of the dialog is its own to draw: no frame (Windows keeps its shadow and round corners).
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        // The dialog moved by any of it that takes no press.
        WM_NCHITTEST => {
            let (x, y) = point(lparam);
            let mut at = POINT { x: x as i32, y: y as i32 };
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at);
            }
            let caption = with_ui(|ui| ui.guide_caption(hwnd, at.x as f32, at.y as f32)).unwrap_or(false);
            LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize)
        }
        // The tip takes presses without taking the front from the window it is beside.
        WM_MOUSEACTIVATE if with_ui(|ui| ui.guide.as_ref().and_then(|guide| guide.tip.as_ref()).is_some_and(|tip| tip.hwnd == hwnd)).unwrap_or(false) => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            with_ui(|ui| ui.guide_frame = Instant::now());
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with_ui(|ui| {
                let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = unsafe { TrackMouseEvent(&mut track) };
                ui.guide_pointer(hwnd, Some((x, y)), None);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| ui.guide_pointer(hwnd, None, None));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            unsafe { SetCapture(hwnd) };
            with_ui(|ui| ui.guide_pointer(hwnd, Some((x, y)), Some(true)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            crate::ui::window::let_go_of_capture(hwnd);
            with_ui(|ui| ui.guide_pointer(hwnd, Some((x, y)), Some(false)));
            LRESULT(0)
        }
        // Enter goes on, Esc leaves it, as a dialog's keys do.
        WM_KEYDOWN => {
            match VIRTUAL_KEY(wparam.0 as u16) {
                VK_RETURN => with_ui(|ui| ui.guide_hit(Hit::Next)),
                VK_ESCAPE => with_ui(|ui| ui.guide_hit(Hit::Skip)),
                _ => None,
            };
            LRESULT(0)
        }
        WM_CLOSE => {
            with_ui(|ui| ui.guide_hit(Hit::Skip));
            LRESULT(0)
        }
        // The dialog moved by hand: the window with it.
        WM_MOVING => {
            let to = unsafe { *(lparam.0 as *const RECT) };
            with_ui(|ui| ui.guide_moved(hwnd, to));
            LRESULT(1)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            with_ui(|ui| ui.guide_scaled(hwnd, (wparam.0 & 0xFFFF) as f32 / 96.0));
            unsafe {
                let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// What search and the look show in the small desktop of the last step, as large as on the screen (DIPs): search's
/// sheet, and the look beside it.
const SEARCH: (f32, f32) = (640.0, 236.0);
const LOOK: (f32, f32) = (380.0, 470.0);

/// What the last step's search finds, and the look shows of the first: the words typed, the files (their name, their
/// folder, their kind and its colour), the look's heading and its lines.
struct Example {
    typed: &'static str,
    group: &'static str,
    files: [(&'static str, &'static str, &'static str, u32); 2],
    size: &'static str,
    heading: &'static str,
    lines: [(&'static str, &'static str); 5],
}

fn example(lang: Lang) -> Example {
    match lang {
        Lang::Zh => Example {
            typed: "公式",
            group: "文件",
            files: [("期中公式纸.pdf", r"D:\Documents\课程\力学", "PDF", 0xD6282E), ("公式推导笔记.docx", r"D:\Documents\课程\力学", "DOCX", 0x2B62C4)],
            size: "741 KB",
            heading: "力学 期中公式",
            lines: [("牛顿第二定律", "F = m a"), ("动能", "E = ½ m v²"), ("胡克定律", "F = −k x"), ("角动量", "L = I ω"), ("万有引力", "F = G m₁m₂ / r²")],
        },
        Lang::En => Example {
            typed: "formula",
            group: "Files",
            files: [("Midterm formula sheet.pdf", r"D:\Documents\Courses\Mechanics", "PDF", 0xD6282E), ("Formula notes.docx", r"D:\Documents\Courses\Mechanics", "DOCX", 0x2B62C4)],
            size: "741 KB",
            heading: "Mechanics midterm formulas",
            lines: [("Newton's second law", "F = m a"), ("Kinetic energy", "E = ½ m v²"), ("Hooke's law", "F = −k x"), ("Angular momentum", "L = I ω"), ("Gravitation", "F = G m₁m₂ / r²")],
        },
    }
}

/// The pointer the plays move, its tip at `at`, `size` across, as Windows draws its own: white, outlined, a little
/// shadow under it.
fn paint_pointer(frame: &Frame, at: (f32, f32), size: f32) {
    // (Its icon's tip is near the corner of its box: the box's middle is that far from it.)
    let off = (12.0 - 4.2) * size / 24.0;
    let centre = Point { x: at.0 + off, y: at.1 + off };
    frame.icon_solid(Icon::Pointer, Point { x: centre.x, y: centre.y + 1.0 }, size, Color::hex(0x000000, 0.3));
    frame.icon_solid(Icon::Pointer, centre, size, Color::hex(0xFFFFFF, 1.0));
    frame.icon(Icon::Pointer, centre, size, Color::hex(0x111111, 1.0));
}

/// Where the second and third steps' pointer rests on the panel at `r`: within it, clear of its edges and its buttons.
fn inside(r: Rect) -> (f32, f32) {
    (r.x + r.w * 0.42, r.y + r.h * 0.62)
}

/// The panel's free corner at `r`, on `edge`: the one at its foot away from the edge, a DIP inside it.
fn free_corner(edge: Edge, r: Rect) -> (f32, f32) {
    match edge {
        Edge::Right => (r.x + 1.0, r.y + r.h - 1.0),
        Edge::Left | Edge::Top => (r.x + r.w - 1.0, r.y + r.h - 1.0),
    }
}

/// On the panel at `r`, on `edge`, a little in from its free corner (see `free_corner`): off the corner, where the
/// pointer is a plain one.
fn aside(edge: Edge, r: Rect) -> (f32, f32) {
    let (corner, inside) = (free_corner(edge, r), inside(r));
    (corner.0 + (inside.0 - corner.0) * 0.25, corner.1 + (inside.1 - corner.1) * 0.25)
}

/// The pointer that sizes (`icon`, along the diagonal it sizes), its middle at `at`, `size` across, as Windows draws
/// it: dark, ringed in white, a little shadow under it.
fn paint_sizer(frame: &Frame, at: (f32, f32), size: f32, icon: Icon) {
    let centre = |dx: f32, dy: f32| Point { x: at.0 + dx, y: at.1 + dy };
    frame.icon(icon, centre(0.0, 1.0), size, Color::hex(0x000000, 0.3));
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-0.7, -0.7), (0.7, -0.7), (-0.7, 0.7), (0.7, 0.7)] {
        frame.icon(icon, centre(dx * 1.5, dy * 1.5), size, Color::hex(0xFFFFFF, 1.0));
    }
    frame.icon(icon, centre(0.0, 0.0), size, Color::hex(0x111111, 1.0));
}

/// The ring a click leaves round `at`, `along` its life (0 to 1): growing as it fades.
fn paint_ring(frame: &Frame, at: (f32, f32), along: f32) {
    let r = 11.0 * (0.5 + along);
    let opacity = 1.0 - along;
    let ring = |color: Color, out: f32, line: f32| {
        Canvas::stroke_rounded(frame, crate::ui::canvas::Fill::Solid(color), at.0 - r - out, at.1 - r - out, 2.0 * (r + out), 2.0 * (r + out), r + out, line);
    };
    ring(Color::hex(0x000000, 0.35 * opacity), 1.5, 1.0);
    ring(Color::hex(0xFFFFFF, 0.85 * opacity), 0.0, 2.0);
}

/// `paint` drawn `opacity` opaque, within `bounds`.
fn faded(frame: &Frame, bounds: Rect, opacity: f32, paint: impl FnOnce()) {
    let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: rect(bounds.x, bounds.y, bounds.w, bounds.h), maskTransform: Matrix3x2::identity(), opacity, ..Default::default() };
    unsafe { frame.dc.PushLayer(&layer, None) };
    paint();
    unsafe { frame.dc.PopLayer() };
}

/// One line of `text` in the middle of `r`.
fn centred(frame: &Frame, text: &str, font: Font, color: Color, r: Rect) {
    let wide = frame.gfx.measure(text, font).min(r.w);
    text_centred(frame, text, font, color, r.x + (r.w - wide) / 2.0, r.y + r.h / 2.0, wide + 1.0, Align::Start);
}

/// A button of its windows in `r`, as Windows 11 draws one: outlined, or the one that goes on in the accent; lit under
/// the pointer, darker held down.
#[allow(clippy::too_many_arguments)]
fn button(frame: &Frame, palette: &Palette, grounds: &Grounds, popup: &mut Popup, r: Rect, text: &str, hit: Hit, accent: bool) {
    let hover = popup.hovered() == Some(hit);
    let down = hover && popup.pressed == Some(hit);
    if accent {
        fill(frame, palette.switch_on.alpha(if down { 0.8 } else if hover { 0.9 } else { 1.0 }), r.x, r.y, r.w, r.h, 4.0);
        centred(frame, text, BODY, Color::hex(0xFFFFFF, 1.0), r);
    } else {
        fill(frame, if hover { grounds.button_hover } else { grounds.button }, r.x, r.y, r.w, r.h, 4.0);
        stroke_inside(frame, r, 4.0, grounds.button_stroke);
        centred(frame, text, BODY, if down { palette.text2 } else { palette.text }, r);
    }
    popup.hits.push((r, hit));
}

/// The buttons a step has, from the left: leaving it (but at its end), going back (but at its first step), going on
/// (the last step's and its end's say it is done).
fn buttons(step: Step, lang: Lang) -> Vec<(&'static str, Hit, bool)> {
    let p = |zh: &'static str, en: &'static str| pick(lang, zh, en);
    let mut buttons = Vec::new();
    if step != Step::Ready {
        buttons.push((p("跳过引导", "Skip guide"), Hit::Skip, false));
    }
    if step.before().is_some() {
        buttons.push((p("上一步", "Back"), Hit::Back, false));
    }
    let next = if matches!(step, Step::Keys | Step::Ready) { p("完成", "Done") } else { p("下一步", "Next") };
    buttons.push((next, Hit::Next, true));
    buttons
}

/// `buttons` side by side, the last at `right`, their middles on `cy`.
fn paint_buttons(frame: &Frame, palette: &Palette, grounds: &Grounds, popup: &mut Popup, buttons: &[(&str, Hit, bool)], right: f32, cy: f32, least: f32) {
    let mut x = right;
    for &(text, hit, accent) in buttons.iter().rev() {
        let w = (frame.gfx.measure(text, BODY) + 40.0).max(least);
        x -= w;
        button(frame, palette, grounds, popup, Rect { x, y: cy - BUTTON / 2.0, w, h: BUTTON }, text, hit, accent);
        x -= 8.0;
    }
}

/// Whether a step is done, in a line: a tick and what it says done, else what it asks.
fn paint_state(frame: &Frame, palette: &Palette, grounds: &Grounds, words: &Words, done: bool, x: f32, cy: f32, width: f32) {
    if done {
        frame.icon(Icon::Check, Point { x: x + 8.0, y: cy }, 16.0, grounds.done);
        text_centred(frame, words.done, STATE, grounds.done, x + 22.0, cy, width - 22.0, Align::Start);
    } else {
        text_centred(frame, &words.wait, STATE, palette.text2, x, cy, width, Align::Start);
    }
}

impl Ui {
    /// The guide begun, at its first step; up already, brought to the front.
    pub(super) fn begin_guide(&mut self) {
        if let Some(guide) = &self.guide {
            let shown = [guide.dialog.as_ref(), guide.tip.as_ref()].into_iter().flatten().any(|popup| unsafe { IsWindowVisible(popup.hwnd) }.as_bool());
            match guide.dialog.as_ref().filter(|_| shown && guide.step != Step::Layout) {
                Some(dialog) => unsafe {
                    let _ = SetForegroundWindow(dialog.hwnd);
                },
                None => self.show_for_guide(),
            }
            return;
        }
        register();
        let was_up = unsafe { IsWindowVisible(self.hwnd) }.as_bool();
        self.guide = Some(Guide {
            step: Step::Push,
            done: [false; STEPS],
            tried: [false; 2],
            was_up,
            since: Instant::now(),
            edge: (Edge::Right, None),
            dialog: None,
            tip: None,
            change: None,
            drag: None,
            grounds: [render::GroundLayers::default(), render::GroundLayers::default()],
            _showing: crate::ui::life::Showing::of(&self.gfx),
        });
        TOLD.store(0, Ordering::Release);
        UP.store(true, Ordering::Release);
        // Shown: not again unasked.
        if !self.settings.guided {
            self.settings.guided = true;
            self.save();
        }
        self.guide_to(Step::Push);
        // (Come up, not changed to: over the window, opened first where it was not up, as it is next drawn.)
        let now = Instant::now();
        if let Some(guide) = self.guide.as_mut() {
            let mut opening = Change::new(Kind::Opening, Step::Push, Step::Push, now, (0.0, 0.0));
            opening.lead = if was_up { 0.0 } else { WINDOW_LEAD };
            guide.change = Some(opening);
        }
    }

    /// The guide turned to `step`, changing to it as `Change` has it: the layout's on the window's own page with the tip
    /// beside it, the others in the dialog over the window.
    fn guide_to(&mut self, step: Step) {
        self.stop_drag();
        let now = Instant::now();
        // (Its windows made as they are first wanted.)
        let (want_tip, want_dialog) = match &self.guide {
            Some(guide) => (step == Step::Layout && guide.tip.is_none(), step != Step::Layout && guide.dialog.is_none()),
            None => return,
        };
        let tip = if want_tip { self.make_tip() } else { None };
        let dialog = if want_dialog { self.make_dialog() } else { None };
        let Some((from, from_since, was_on)) = self.guide.as_ref().map(|guide| (guide.step, guide.since, guide.edge)) else { return };
        let screen = self.stage.screen.clone().unwrap_or_default();
        let places = self.settings.places();
        let on = match step {
            // The edge the panel is shown opened from: the right, where it opens anything, else the first that does.
            Step::Push => {
                let edge = if places.part(&screen, Edge::Right).is_some() { Edge::Right } else { places.lit(&screen).each().next().unwrap_or(Edge::Right) };
                (edge, places.part(&screen, edge))
            }
            // The readings, which tear off by their lanes' heads, on their own edge, else on the right.
            Step::Size | Step::Tear => (EDGES.into_iter().find(|&edge| places.part(&screen, edge) == Some(Part::Monitor)).unwrap_or(Edge::Right), Some(Part::Monitor)),
            Step::Layout | Step::Keys | Step::Ready => was_on,
        };
        let heights = (self.dialog_height(from, was_on.0), self.dialog_height(step, on.0));
        let Some(guide) = self.guide.as_mut() else { return };
        guide.edge = on;
        if tip.is_some() {
            guide.tip = tip;
        }
        if dialog.is_some() {
            guide.dialog = dialog;
        }
        guide.change = Some(Change::between(from, step, from_since, heights));
        guide.step = step;
        guide.since = now;
        // (What was done before the step is shown is not its doing.)
        TOLD.store(0, Ordering::Release);
        // (The windows shown and hidden as the change goes: see `draw_guide`.)
        if step == Step::Layout {
            self.turn_to(Page::Layout);
            self.scroll = 0.0;
            self.scroll_target = 0.0;
            // One screen large: its desktop, and the real panel slid out as a component is put on its edge.
            if self.screens.len() > 1 {
                self.view = None;
                self.view = self.stage_screen();
            }
            self.shown_edge = None;
            let screen = self.editor_screen().or_else(|| self.stage.screen.clone()).unwrap_or_default();
            let before = self.settings.places().clone();
            let held = before.part(&screen, Edge::Left);
            let moves = script(played_part(held), held);
            if let Some(guide) = self.guide.as_mut() {
                guide.drag = Some(Drag { before, screen, moves, at: 0, since: now, from: None, pointer: None });
            }
        }
        self.guide_frame = now;
    }

    /// The window shown and in front, for the guide (as Windows shows a window).
    fn show_for_guide(&self) {
        shown_by_system(self.hwnd, true);
        unsafe {
            // (Glance started hidden has the system take the first window it shows as hidden: shown again.)
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// The guide leaving: the dialog going, the guide ended once it has (see `draw_guide`); the window turned meanwhile,
    /// under the dialog and its cover, to the general page it was begun from.
    fn leave_guide(&mut self) {
        self.stop_drag();
        let now = Instant::now();
        let Some(guide) = self.guide.as_mut() else { return };
        guide.change = Some(Change::new(Kind::Ending, guide.step, guide.step, guide.since, (0.0, 0.0)));
        if guide.was_up {
            self.turn_to(Page::General);
        }
        self.guide_frame = now;
    }

    /// The window made to take nothing (`covered`, the dialog over it) or to take input again, as a window under a
    /// dialog of its own is: pressed, it brings the dialog to the front.
    fn guide_covers(&self, covered: bool) {
        unsafe {
            if IsWindowEnabled(self.hwnd).as_bool() == covered {
                let _ = EnableWindow(self.hwnd, !covered);
            }
        }
    }

    /// The window drawn: as the change under way has it, from here on (see `guide_holds_window`).
    pub(super) fn guide_saw_window_drawn(&mut self) {
        if let Some(change) = self.guide.as_mut().and_then(|guide| guide.change.as_mut()) {
            change.window_drawn = true;
        }
    }

    /// Whether the window is left as it was last drawn, so that only the guide is drawn as it plays: under the dialog
    /// at rest on its step, where nothing of it can be seen clearly or touched; and through a change of the guide once
    /// it has been drawn for it (what a page first drawn fetches, its desktops and pictures, each drawing it whole as
    /// it comes, taken in as the change is over). Not where it is another size than it was drawn at (moved to a screen
    /// of another scale under the dialog). At rest, not where something it shows has come in since (see `take_in`):
    /// drawn once with it.
    pub(super) fn guide_holds_window(&self) -> bool {
        let under = self.guide.as_ref().is_some_and(|guide| holds(guide.change.as_ref(), guide.step, self.taken_unseen));
        let mut inside = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut inside);
        }
        under && self.drawn_size == Some((inside.right.max(1) as u32, inside.bottom.max(1) as u32))
    }

    /// The dialog `hwnd` moved by hand to `to` (physical px): the window moved with it, as far, the dialog kept in its
    /// middle.
    fn guide_moved(&self, hwnd: HWND, to: RECT) {
        if !self.guide.as_ref().and_then(|guide| guide.dialog.as_ref()).is_some_and(|dialog| dialog.hwnd == hwnd) {
            return;
        }
        let (mut was, mut window) = (RECT::default(), RECT::default());
        unsafe {
            let _ = GetWindowRect(hwnd, &mut was);
            let _ = GetWindowRect(self.hwnd, &mut window);
            let _ = SetWindowPos(self.hwnd, None, window.left + to.left - was.left, window.top + to.top - was.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// The guide ended (left, or done): the window taking input again before its windows go (so that it is what comes
    /// to the front), then left up if it was up as it began, else closed.
    fn end_guide(&mut self) {
        self.stop_drag();
        self.guide_covers(false);
        let Some(guide) = self.guide.take() else { return };
        let was_up = guide.was_up;
        drop(guide);
        if was_up {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
            }
        } else {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// The dialog's window, made hidden (placed in the middle of the window as it is drawn: see `fit`), the window's
    /// own as a dialog is: kept over it, and in front where it is brought there. All of it its own to draw, its plate's
    /// shadow and round corners too (see `paint_dialog`).
    fn make_dialog(&self) -> Option<Popup> {
        let size = (WIDE + 2.0 * ROOM, DESK + 2.0 * ROOM);
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let title = HSTRING::from(pick(self.lang, "Glance 使用引导", "Glance guide"));
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, CLASS, &title, WS_POPUP | WS_SYSMENU, 0, 0, 1, 1, Some(self.hwnd), None, Some(module.into()), None).ok()?;
            let square = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as *const _, size_of_val(&square) as u32);
            let surface = Surface::new(&self.gfx, hwnd).ok();
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let plate = Rect { x: ROOM, y: ROOM, w: WIDE, h: DESK };
            Some(Popup { hwnd, surface, scale, size, hits: Vec::new(), hits_from: (ROOM, ROOM), pointer: None, pressed: None, plate, beak: None })
        }
    }

    /// The tip's window, made hidden (placed beside the window as it is drawn: see `place_tip`), kept over the window.
    fn make_tip(&self) -> Option<Popup> {
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW, CLASS, w!(""), WS_POPUP, 0, 0, 1, 1, Some(self.hwnd), None, Some(module.into()), None).ok()?;
            let surface = Surface::new(&self.gfx, hwnd).ok();
            Some(Popup { hwnd, surface, scale: self.scale, size: (0.0, 0.0), hits: Vec::new(), hits_from: (0.0, 0.0), pointer: None, pressed: None, plate: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, beak: None })
        }
    }

    /// How tall the dialog is for `step` (DIPs), the first step's panel opened from `edge`.
    fn dialog_height(&self, step: Step, edge: Edge) -> f32 {
        let words = words(step, edge, self.lang);
        let inner = WIDE - 2.0 * PAD;
        let desk = DESK + 20.0;
        let body = if words.body.is_empty() { 0.0 } else { 6.0 + self.gfx.wrapped_height(&words.body, BODY, inner) };
        let keys = if step == Step::Keys { 12.0 + 2.0 * KEY_ROW + 8.0 } else { 0.0 };
        let state = if step == Step::Ready { 0.0 } else { 12.0 + 20.0 };
        PAD + desk + 28.0 + body + keys + state + PAD + FOOTER
    }

    /// How tall the tip's plate is (DIPs).
    fn tip_height(&self) -> f32 {
        let words = words(Step::Layout, Edge::Left, self.lang);
        TIP_PAD + 24.0 + self.gfx.wrapped_height(&words.body, BODY, TIP_WIDE - 2.0 * TIP_PAD) + 10.0 + 20.0 + 14.0 + BUTTON + TIP_PAD
    }

    /// Where the tip goes, `plate_h` DIPs tall at `scale`: beside the window by its editor, its beak at it, on the
    /// window's right where there is room, else on its left; else over the window at the editor's foot, pointing at
    /// nothing. The side its beak is on and how far down it (DIPs), and its window, the room for its shadow round its
    /// plate (physical px).
    fn place_tip(&self, scale: f32, plate_h: f32) -> (Option<(Edge, f32)>, RECT) {
        let mut window = RECT::default();
        let mut corner = POINT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut window);
            let _ = ClientToScreen(self.hwnd, &mut corner);
        }
        let work = work_of(self.hwnd);
        let px = |dips: f32| (dips * scale).round() as i32;
        // The editor on the screen (the window's DIPs are of its own scale).
        let editor = self.editor_at.unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        let on_screen = |x: f32, y: f32| (corner.x + (x * self.scale).round() as i32, corner.y + (y * self.scale).round() as i32);
        let (_, middle) = on_screen(0.0, editor.y + editor.h / 2.0);
        let (right, foot) = on_screen(editor.x + editor.w, editor.y + editor.h);
        let (w, h) = (px(TIP_WIDE), px(plate_h));
        let room = px(BEAK + TIP_OFF);
        let beside = (middle - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
        // With no room beside the window: in it, left of the editor and under the components (what is dragged and
        // where it goes both left in sight), where that holds it.
        let (editor_left, _) = on_screen(editor.x, 0.0);
        let (_, under_library) = self.library_at.map_or((0, corner.y), |r| on_screen(0.0, r.y + r.h + 12.0));
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let client_bottom = corner.y + client.bottom;
        let inside = (editor_left - room - w, (middle - h / 2).clamp(under_library, (client_bottom - h - px(12.0)).max(under_library)));
        let ((left, top), side) = if window.right + room + w <= work.right {
            ((window.right + room, beside), Some(Edge::Left))
        } else if window.left - room - w >= work.left {
            ((window.left - room - w, beside), Some(Edge::Right))
        } else if inside.0 >= corner.x && inside.1 + h <= client_bottom {
            (inside, Some(Edge::Right))
        } else {
            (((right - w - px(16.0)).clamp(work.left, (work.right - w).max(work.left)), (foot - h - px(16.0)).clamp(work.top, (work.bottom - h).max(work.top))), None)
        };
        let beak = side.map(|side| (side, ((middle - top) as f32 / scale).clamp(20.0, plate_h - 20.0)));
        let pad = px(SHADOW);
        (beak, RECT { left: left - pad, top: top - pad, right: left + w + pad, bottom: top + h + pad })
    }

    /// The guide's windows drawn, what was done elsewhere taken in first: the dialog, or the tip beside the window,
    /// placed by it as it is now.
    pub(super) fn draw_guide(&mut self, now: Instant) {
        let Some(mut guide) = self.guide.take() else {
            self.guide_frame = now + IDLE;
            return;
        };
        let _timed = crate::perf::Timed::start("guide frame", || match &guide.change {
            Some(change) => format!("{:?} {:?} {:.1} ms in", guide.step, change.kind, change.ms(now)),
            None => format!("{:?}", guide.step),
        });
        let told = TOLD.swap(0, Ordering::AcqRel);
        match guide.step {
            Step::Push if told & Saw::Pushed.bit() != 0 => guide.done[Step::Push as usize] = true,
            Step::Size if told & Saw::Sized.bit() != 0 => guide.done[Step::Size as usize] = true,
            Step::Tear if told & Saw::Torn.bit() != 0 => guide.done[Step::Tear as usize] = true,
            Step::Keys => {
                for (at, saw) in [Saw::Searched, Saw::Looked].into_iter().enumerate() {
                    if told & saw.bit() != 0 {
                        guide.tried[at] = true;
                    }
                }
                if guide.tried == [true, true] {
                    guide.done[Step::Keys as usize] = true;
                }
            }
            _ => {}
        }
        // The change under way moved on: the window opened as the guide comes up where it was not up, and the change
        // over once it is (the guide ended, where it was leaving).
        let mut ended = false;
        if let Some(change) = guide.change.as_mut() {
            let ms = change.ms(now);
            if change.kind == Kind::Opening && !guide.was_up && !unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
                self.show_for_guide();
            }
            if ms >= change.length() {
                ended = change.kind == Kind::Ending;
                guide.change = None;
            }
        }
        if ended {
            self.guide = Some(guide);
            return self.end_guide();
        }
        let height = self.dialog_height(guide.step, guide.edge.0);
        let (look, cover, step_shown, since, rests, between) = {
            let change = guide.change.as_ref().map(|change| (change, change.ms(now)));
            let look = dialog_look(change, guide.step, height);
            let (step, since) = match (look, change) {
                (Some(look), Some((change, _))) if look.old => (change.from, change.from_since),
                _ => (guide.step, guide.since),
            };
            let between = change.is_some_and(|(change, _)| matches!(change.kind, Kind::ToTip | Kind::FromTip));
            (look, cover_look(change, guide.step), step, since, tip_rests(change.map(|(change, _)| change), guide.step), between)
        };
        // The window taking nothing while the dialog is over it (before the dialog goes, so that the window is what
        // comes to the front as it does).
        self.guide_covers(look.is_some());
        // The tip beside the window as it is now (moved, sized, on another screen): where it rests, and where the
        // dialog's plate is carried to or from.
        let tip_at = match guide.tip.as_mut().filter(|_| rests || between) {
            Some(tip) => {
                tip.scale = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32 / 96.0;
                let plate_h = self.tip_height();
                let (beak, at) = self.place_tip(tip.scale, plate_h);
                tip.size = ((at.right - at.left) as f32 / tip.scale, (at.bottom - at.top) as f32 / tip.scale);
                tip.plate = Rect { x: SHADOW, y: SHADOW, w: TIP_WIDE, h: plate_h };
                tip.beak = beak;
                Some(at)
            }
            None => None,
        };
        let gfx = self.gfx.clone();
        // (What comes shown before what goes is hidden: never a frame with neither.)
        let carried_by = tip_at.filter(|_| between);
        if rests {
            self.show_tip(&gfx, &mut guide, tip_at);
            self.show_dialog(&gfx, &mut guide, look, cover, step_shown, since, carried_by, now);
        } else {
            self.show_dialog(&gfx, &mut guide, look, cover, step_shown, since, carried_by, now);
            self.show_tip(&gfx, &mut guide, None);
        }
        start_clock(&mut guide);
        // Drawn again as anything of it plays; the window too while its own page plays the layout's step.
        let plays = guide.drag.is_some() && rests;
        self.guide = Some(guide);
        self.guide_frame = now + FRAME;
        if plays {
            self.next_frame = self.next_frame.min(now + FRAME);
        }
    }

    /// The dialog's window over the window's inside (and over where the tip stands, `tip_at`, as the dialog's plate is
    /// carried to or from it), drawn as `look` has it, the window covered under it as dark as `cover`; hidden where
    /// neither shows. Its plate in the middle of the window, kept on its screen.
    #[allow(clippy::too_many_arguments)]
    fn show_dialog(&mut self, gfx: &Rc<Gfx>, guide: &mut Guide, look: Option<DialogLook>, cover: f32, step: Step, since: Instant, tip_at: Option<RECT>, now: Instant) {
        let Some(mut dialog) = guide.dialog.take() else { return };
        if look.is_none() && cover <= 0.0 {
            unsafe {
                if IsWindowVisible(dialog.hwnd).as_bool() {
                    let _ = ShowWindow(dialog.hwnd, SW_HIDE);
                }
            }
            guide.dialog = Some(dialog);
            return;
        }
        dialog.scale = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32 / 96.0;
        let scale = dialog.scale;
        let px = |dips: f32| (dips * scale).round() as i32;
        let mut inside = RECT::default();
        let mut corner = POINT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut inside);
            let _ = ClientToScreen(self.hwnd, &mut corner);
        }
        let inside = RECT { left: corner.x, top: corner.y, right: corner.x + inside.right, bottom: corner.y + inside.bottom };
        let work = work_of(self.hwnd);
        let (wide, high) = (px(WIDE), px(look.map_or(0.0, |look| look.height)));
        let x = ((inside.left + inside.right - wide) / 2).clamp(work.left, (work.right - wide).max(work.left));
        let y = ((inside.top + inside.bottom - high) / 2).clamp(work.top, (work.bottom - high).max(work.top));
        let plate = RECT { left: x, top: y, right: x + wide, bottom: y + high };
        let room = px(ROOM);
        let mut at = union(inside, RECT { left: plate.left - room, top: plate.top - room, right: plate.right + room, bottom: plate.bottom + room });
        if let Some(tip) = tip_at {
            at = union(at, tip);
        }
        place(dialog.hwnd, at);
        let from = (at.left, at.top);
        dialog.size = ((at.right - at.left) as f32 / scale, (at.bottom - at.top) as f32 / scale);
        dialog.plate = dips(plate, from, scale);
        dialog.hits_from = (dialog.plate.x, dialog.plate.y);
        let covered = dips(inside, from, scale);
        // (The tip, drawn on the plate as it is carried, from where its window stands.)
        let mut tip = match (tip_at, guide.tip.take()) {
            (Some(tip_at), Some(tip)) => Some((tip, ((tip_at.left - from.0) as f32 / scale, (tip_at.top - from.1) as f32 / scale))),
            (None, kept) => {
                guide.tip = kept;
                None
            }
            (Some(_), None) => None,
        };
        self.draw_popup(gfx, &mut dialog, |ui, frame, popup| {
            if cover > 0.0 {
                frame.fill(Color::hex(0x000000, COVER * cover), covered.x, covered.y, covered.w, covered.h);
            }
            if let Some(look) = look {
                ui.paint_dialog(frame, guide, popup, &look, step, since, tip.as_mut().map(|(tip, corner)| (tip, *corner)), now);
            }
        });
        if let Some((tip, _)) = tip {
            guide.tip = Some(tip);
        }
        unsafe {
            if !IsWindowVisible(dialog.hwnd).as_bool() {
                // (Glance started hidden has the system take the first window it shows as hidden: shown again.)
                let _ = ShowWindow(dialog.hwnd, SW_SHOW);
                if !IsWindowVisible(dialog.hwnd).as_bool() {
                    let _ = ShowWindow(dialog.hwnd, SW_SHOW);
                }
                let _ = SetForegroundWindow(dialog.hwnd);
            }
        }
        guide.dialog = Some(dialog);
    }

    /// The tip's window at `at` (physical px: see `place_tip`), drawn; hidden where it is none (not at rest).
    fn show_tip(&mut self, gfx: &Rc<Gfx>, guide: &mut Guide, at: Option<RECT>) {
        let Some(mut tip) = guide.tip.take() else { return };
        match at {
            Some(at) => {
                place(tip.hwnd, at);
                self.draw_popup(gfx, &mut tip, |ui, frame, popup| ui.paint_tip(frame, guide, popup));
                unsafe {
                    if !IsWindowVisible(tip.hwnd).as_bool() {
                        let _ = ShowWindow(tip.hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
            None => unsafe {
                if IsWindowVisible(tip.hwnd).as_bool() {
                    let _ = ShowWindow(tip.hwnd, SW_HIDE);
                }
            },
        }
        guide.tip = Some(tip);
    }

    /// `popup` drawn by `paint` (at its own scale: what is drawn as the window's is drawn at the popup's meanwhile).
    fn draw_popup(&mut self, gfx: &Rc<Gfx>, popup: &mut Popup, paint: impl FnOnce(&mut Self, &Frame, &mut Popup)) {
        // (Its surface let go of with the window's, the window on another device: made again on that one.)
        let Some(mut surface) = popup.surface.take().or_else(|| Surface::new(gfx, popup.hwnd).ok()) else { return };
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(popup.hwnd, &mut client);
        }
        let size = ((client.right - client.left).max(1) as u32, (client.bottom - client.top).max(1) as u32);
        let scale = popup.scale;
        let window_scale = std::mem::replace(&mut self.scale, scale);
        let drawn = surface.draw(gfx, size, scale, |frame| {
            frame.origin(0.0, 0.0);
            paint(self, frame, popup);
        });
        self.scale = window_scale;
        if let Err(error) = &drawn {
            gfx::failed("guide", gfx, Some(error));
        }
        popup.surface = Some(surface);
    }

    /// The dialog as `look` has it: its plate, its shadow round it; on it what `step` shows (played from `since`), and
    /// the step's buttons in the footer. Between it and the tip (`tip`, and its window's corner in the dialog's), the
    /// plate carried to the tip's place and made its plate, the content of each on it as it goes.
    #[allow(clippy::too_many_arguments)]
    fn paint_dialog(&mut self, frame: &Frame, guide: &mut Guide, dialog: &mut Popup, look: &DialogLook, step: Step, since: Instant, tip: Option<(&mut Popup, (f32, f32))>, now: Instant) {
        let palette = self.palette.clone();
        let grounds = Grounds::new(self.dark);
        dialog.hits.clear();
        let own = dialog.plate;
        let p = match &tip {
            Some((tip, corner)) => carried(own, Rect { x: corner.0 + tip.plate.x, y: corner.1 + tip.plate.y, w: tip.plate.w, h: tip.plate.h }, look.carried),
            None => own,
        };
        let whole = Rect { x: 0.0, y: 0.0, w: dialog.size.0, h: dialog.size.1 };
        let (cx, cy) = (p.x + p.w / 2.0, p.y + p.h / 2.0);
        let about = Matrix3x2::translation(-cx, -cy) * Matrix3x2::scale(look.scale, look.scale) * Matrix3x2::translation(cx, cy);
        let footer = p.y + p.h - FOOTER;
        let lang = self.lang;
        faded(frame, whole, look.opacity, || {
            frame.within(about, |frame| {
                frame.shadow(grounds.shadow, p.x, p.y, p.w, p.h, 8.0, 14.0, 8.0);
                fill(frame, mix(grounds.dialog, grounds.flyout, look.carried), p.x, p.y, p.w, p.h, 8.0);
                // (The footer the plate's foot, in its own ground.)
                faded(frame, p, look.footer, || {
                    unsafe { frame.dc.PushAxisAlignedClip(&rect(p.x, footer, p.w, FOOTER), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    fill(frame, grounds.footer, p.x, p.y, p.w, p.h, 8.0);
                    unsafe { frame.dc.PopAxisAlignedClip() };
                    frame.fill(palette.rule, p.x, footer, p.w, 1.0);
                });
                stroke_inside(frame, p, 8.0, palette.rule);
                // What the step shows, slid as it goes or comes, within the plate above the footer.
                if look.content > 0.0 {
                    unsafe { frame.dc.PushAxisAlignedClip(&rect(p.x, p.y, p.w, p.h - FOOTER), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    faded(frame, p, look.content, || {
                        frame.within(Matrix3x2::translation(p.x + look.slide, p.y), |frame| self.paint_step(frame, &palette, &grounds, guide, dialog, step, since, now));
                    });
                    unsafe { frame.dc.PopAxisAlignedClip() };
                }
                if look.buttons > 0.0 {
                    faded(frame, p, look.buttons, || {
                        frame.within(Matrix3x2::translation(p.x, p.y), |frame| {
                            paint_buttons(frame, &palette, &grounds, dialog, &buttons(step, lang), WIDE - PAD, p.h - FOOTER / 2.0, BUTTON_LEAST);
                        });
                    });
                }
                // (Where the plate is now, as where it ends: the tip's window's corner, at the tip's plate.)
                if let Some((tip, _)) = tip.filter(|_| look.tip > 0.0) {
                    let (x, y) = (p.x - tip.plate.x, p.y - tip.plate.y);
                    faded(frame, whole, look.tip, || frame.within(Matrix3x2::translation(x, y), |frame| self.paint_tip_content(frame, guide, tip)));
                }
            });
        });
    }

    /// What `step` shows on the dialog, from its plate's corner: the small desktop with the step's play at its top
    /// (played from `since`), its place among the steps, its title and what it asks (the shortcuts' rows, on the last
    /// step), whether it is done.
    #[allow(clippy::too_many_arguments)]
    fn paint_step(&mut self, frame: &Frame, palette: &Palette, grounds: &Grounds, guide: &mut Guide, dialog: &mut Popup, step: Step, since: Instant, now: Instant) {
        let lang = self.lang;
        let inner = WIDE - 2.0 * PAD;
        let words = words(step, guide.edge.0, lang);
        let mut y = PAD;
        let mut down = [0, 0];
        let area = Rect { x: PAD, y, w: inner, h: DESK };
        fill(frame, palette.card, area.x, area.y, area.w, area.h, 8.0);
        match step {
            Step::Push => self.play_push(frame, guide, area, since, now),
            Step::Size => self.play_size(frame, guide, area, since, now),
            Step::Tear => self.play_tear(frame, guide, area, since, now),
            Step::Keys => down = self.play_keys(frame, guide, area, since, now),
            // (At the end, the desktop as it is, nothing played on it.)
            _ => {
                self.panel_on_stage(frame, area, None, true, 1.0);
            }
        }
        stroke_inside(frame, area, 8.0, palette.card_stroke);
        y += DESK + 20.0;
        if let Some(index) = step.index() {
            let count = format!("{} / {STEPS}", index + 1);
            frame.text(&count, SMALL, palette.text3, PAD, y + 6.0, inner, Align::End);
        }
        frame.text(words.title, TITLE, palette.text, PAD, y, inner - 60.0, Align::Start);
        y += 28.0;
        if !words.body.is_empty() {
            y += 6.0;
            frame.text_wrapped(&words.body, BODY, palette.text2, PAD, y, inner);
            y += frame.gfx.wrapped_height(&words.body, BODY, inner);
        }
        if step == Step::Keys {
            y += 12.0;
            self.paint_keys(frame, palette, grounds, guide, dialog, Rect { x: PAD, y, w: inner, h: 2.0 * KEY_ROW + 8.0 }, down);
            y += 2.0 * KEY_ROW + 8.0;
        }
        if let Some(index) = step.index() {
            paint_state(frame, palette, grounds, &words, guide.done[index], PAD, y + 12.0 + 10.0, inner);
        }
    }

    /// The shortcuts' rows of the last step in `r`: what each calls up, its keys (as many down as `down` says, as the
    /// play presses them), and a tick once it was tried. Each pressed calls up what it does.
    #[allow(clippy::too_many_arguments)]
    fn paint_keys(&self, frame: &Frame, palette: &Palette, grounds: &Grounds, guide: &Guide, popup: &mut Popup, r: Rect, down: [usize; 2]) {
        let p = |zh: &'static str, en: &'static str| pick(self.lang, zh, en);
        let rows = [
            (Hit::Search, p("搜索", "Search"), p("应用、文件与便签", "Apps, files and notes"), self.settings.search_shortcut),
            (Hit::Look, p("预览", "Preview"), p("选中的文件或卡片", "The file or card picked"), self.settings.preview_shortcut),
        ];
        let hovered = popup.hovered();
        for (at, (hit, name, about, shortcut)) in rows.into_iter().enumerate() {
            let row = Rect { x: r.x, y: r.y + at as f32 * (KEY_ROW + 8.0), w: r.w, h: KEY_ROW };
            fill(frame, if hovered == Some(hit) { palette.control_on } else { palette.control }, row.x, row.y, row.w, row.h, 4.0);
            stroke_inside(frame, row, 4.0, palette.card_stroke);
            frame.text(name, BODY, palette.text, row.x + 14.0, row.y + 8.0, row.w / 2.0, Align::Start);
            frame.text(about, SMALL, palette.text2, row.x + 14.0, row.y + 28.0, row.w / 2.0, Align::Start);
            // The keys against the row's end, as far in as its words are; tried, the tick before them (nothing moved).
            let cy = row.y + row.h / 2.0;
            let keys = caps(shortcut, self.lang);
            let widths: Vec<f32> = keys.iter().map(|key| (frame.gfx.measure(key, SMALL) + 16.0).max(30.0)).collect();
            let mut x = row.x + row.w - 14.0 - widths.iter().sum::<f32>() - 6.0 * (keys.len() - 1) as f32;
            if guide.tried[at] {
                frame.icon(Icon::Check, Point { x: x - 20.0, y: cy }, 16.0, grounds.done);
            }
            for (index, (key, w)) in keys.iter().zip(widths).enumerate() {
                let pressed = index < down[at];
                let cap = Rect { x, y: cy - 12.0 + if pressed { 1.0 } else { 0.0 }, w, h: 24.0 };
                if pressed {
                    fill(frame, palette.switch_on, cap.x, cap.y, cap.w, cap.h, 4.0);
                    centred(frame, key, SMALL, Color::hex(0xFFFFFF, 1.0), cap);
                } else {
                    // (A key's foot: its edge heavier below.)
                    fill(frame, palette.rule.alpha(palette.rule.a * 2.0), cap.x, cap.y + 2.0, cap.w, cap.h, 4.0);
                    fill(frame, grounds.button, cap.x, cap.y, cap.w, cap.h, 4.0);
                    stroke_inside(frame, cap, 4.0, grounds.button_stroke);
                    centred(frame, key, SMALL, palette.text, cap);
                }
                x += cap.w + 6.0;
            }
            popup.hits.push((row, hit));
        }
    }

    /// The first step's play in `area`: the desktop, the pointer gliding to the edge and pushing it, the real panel
    /// sliding out, the pointer back and the panel put away, round and round.
    fn play_push(&mut self, frame: &Frame, guide: &Guide, area: Rect, since: Instant, now: Instant) {
        let (along, out) = pushing(now.saturating_duration_since(since));
        let (edge, part) = guide.edge;
        let screen = match part {
            Some(Part::Music) => self.music_on_stage(frame, area, edge, out).1,
            Some(part) => {
                let prefs = self.prefs.clone().holding(part);
                self.stage_panel(frame, area, Some((edge, &prefs, part)), Staging { out, shown: Shown::Guide, ..Staging::default() }).screen
            }
            None => self.panel_on_stage(frame, area, None, true, 1.0).1,
        };
        // (The screen as much as shows of it.)
        let (left, top) = (screen.x.max(area.x), screen.y.max(area.y));
        let (right, bottom) = ((screen.x + screen.w).min(area.x + area.w), (screen.y + screen.h).min(area.y + area.h));
        let rest = (area.x + area.w * 0.42, area.y + area.h * 0.62);
        let pushed = match edge {
            Edge::Right => (right - 3.0, (top + bottom) / 2.0),
            Edge::Left => (left + 3.0, (top + bottom) / 2.0),
            Edge::Top => ((left + right) / 2.0, top + 3.0),
        };
        paint_pointer(frame, (rest.0 + (pushed.0 - rest.0) * along, rest.1 + (pushed.1 - rest.1) * along), 20.0);
    }

    /// The second step's play in `area`: the whole screen, the readings' panel on its edge made larger and smaller by
    /// its free corner and double-clicked back to its own size, round and round (see `sizing`).
    fn play_size(&mut self, frame: &Frame, guide: &Guide, area: Rect, since: Instant, now: Instant) {
        let play = sizing(now.saturating_duration_since(since));
        let edge = guide.edge.0;
        let prefs = self.prefs.clone().holding(Part::Monitor);
        let panel = Some((edge, &prefs, Part::Monitor));
        self.stage_panel(frame, area, panel, Staging { whole: true, size: Some(play.size), shown: Shown::Guide, ..Staging::default() });
        let place = |ui: &mut Self, mark: SizeMark| {
            let size = match mark {
                SizeMark::Inside => 1.0,
                SizeMark::Corner(size) | SizeMark::Aside(size) => size,
            };
            let r = ui.stage_panel(frame, area, panel, Staging { whole: true, size: Some(size), draw: false, ..Staging::default() }).panel?;
            Some(match mark {
                SizeMark::Inside => inside(r),
                SizeMark::Corner(_) => free_corner(edge, r),
                SizeMark::Aside(_) => aside(edge, r),
            })
        };
        // (No panel yet, nothing read to show in it: no pointer either.)
        let (Some(from), Some(to)) = (place(self, play.hand.from), place(self, play.hand.to)) else { return };
        let at = Hand::gliding(from, to, play.hand.along).point(|point| point);
        let size = if play.held { 20.0 * 0.92 } else { 20.0 };
        if play.sizes {
            paint_sizer(frame, at, size, if edge == Edge::Right { Icon::MoveDiagonal } else { Icon::MoveDiagonal2 });
        } else {
            paint_pointer(frame, at, size);
        }
        if let Some(ring) = play.ring {
            paint_ring(frame, at, ring);
        }
    }

    /// The third step's play in `area`: the whole screen, the readings' panel on its edge, its first lane torn off by
    /// its head and the whole panel by the rest of it, each a widget put down on the screen, round and round (see
    /// `tearing`).
    fn play_tear(&mut self, frame: &Frame, guide: &Guide, area: Rect, since: Instant, now: Instant) {
        let play = tearing(now.saturating_duration_since(since));
        let edge = guide.edge.0;
        let prefs = self.prefs.clone().holding(Part::Monitor);
        let panel = Some((edge, &prefs, Part::Monitor));
        let rest = self.stage_panel(frame, area, panel, Staging { whole: true, draw: false, ..Staging::default() });
        self.stage_panel(frame, area, panel, Staging { whole: true, out: play.out, shown: Shown::Guide, ..Staging::default() });
        let (Some(body), Some(&lane), Some(bar)) = (rest.panel, rest.lanes.first(), rest.bar) else { return };
        let screen = rest.screen;
        // Taken by its head, the lane comes off alone where the panel has others, else the panel whole (see
        // `Panel::head_at`); taken by its foot, left of its buttons (where nothing takes a press of its own), the
        // panel whole.
        let only = (rest.lanes.len() > 1).then_some(0);
        let torn = if only.is_some() { lane } else { body };
        let head = (lane.x + lane.w * 0.3, lane.y + rest.head / 2.0);
        let foot = (bar.x + bar.w * 0.25, bar.y + bar.h / 2.0);
        // Where each is put down: the lane by the screen's far corner, the panel further in, clear of it (on the
        // other side, where the panel is on the left).
        let across = |x: f32, w: f32| {
            let x = if edge == Edge::Left { 2.0 * screen.x + screen.w - x - w } else { x };
            x.clamp(screen.x, (screen.x + screen.w - w).max(screen.x))
        };
        let down = |y: f32, h: f32| y.clamp(screen.y, (screen.y + screen.h - h).max(screen.y));
        let lane_to = (across(screen.x + screen.w * 0.05, torn.w), down(screen.y + screen.h * if edge == Edge::Top { 0.3 } else { 0.08 }, torn.h));
        let whole_to = (across(screen.x + screen.w * 0.3, body.w), down(if edge == Edge::Top { screen.y + screen.h * 0.25 } else { body.y + screen.h * 0.04 }, body.h));
        let place = |mark: TearMark| match mark {
            TearMark::Inside => inside(body),
            TearMark::Head => head,
            TearMark::Foot => foot,
            TearMark::Edge => match edge {
                Edge::Right => (screen.x + screen.w - 3.0, screen.y + screen.h / 2.0),
                Edge::Left => (screen.x + 3.0, screen.y + screen.h / 2.0),
                Edge::Top => (screen.x + screen.w / 2.0, screen.y + 3.0),
            },
            TearMark::LaneDrop => (lane_to.0 + head.0 - torn.x, lane_to.1 + head.1 - torn.y),
            TearMark::WholeDrop => (whole_to.0 + foot.0 - body.x, whole_to.1 + foot.1 - body.y),
        };
        // Each widget where the hand holds it by where it was taken (the stage's DIPs: see `Staging::at`).
        let k = screen.w / self.stage.size.0;
        let on_stage = |x: f32, y: f32| ((x - screen.x) / k, (y - screen.y) / k);
        if let Some((hand, opacity)) = play.lane {
            let at = hand.point(place);
            let corner = on_stage(at.0 - (head.0 - torn.x), at.1 - (head.1 - torn.y));
            self.stage_panel(frame, area, panel, Staging { whole: true, at: Some(corner), lane: only, opacity, desktop: false, shown: Shown::Torn(0), ..Staging::default() });
        }
        if let Some((hand, opacity)) = play.whole {
            let at = hand.point(place);
            let corner = on_stage(at.0 - (foot.0 - body.x), at.1 - (foot.1 - body.y));
            self.stage_panel(frame, area, panel, Staging { whole: true, at: Some(corner), opacity, desktop: false, shown: Shown::Torn(1), ..Staging::default() });
        }
        paint_pointer(frame, play.hand.point(place), if play.held { 20.0 * 0.92 } else { 20.0 });
    }

    /// The last step's play in `area`: the desktop by the middle of its top, search coming up on it as its shortcut is
    /// pressed, the first file found looked at larger beside it as the preview's is, both gone again, round and round.
    /// How many of each shortcut's keys are down.
    fn play_keys(&mut self, frame: &Frame, guide: &mut Guide, area: Rect, since: Instant, now: Instant) -> [usize; 2] {
        let keys = (caps(self.settings.search_shortcut, self.lang).len(), caps(self.settings.preview_shortcut, self.lang).len());
        let (down, search, look) = Keying::new(keys).at(now.saturating_duration_since(since));
        // Search where it comes up, a little way down the middle of the screen, the look beside it; the part of the
        // screen they are in, and some room round them, shown.
        let (sw, sh) = self.stage.size;
        let s = Rect { x: ((sw - SEARCH.0 - 16.0 - LOOK.0) / 2.0).round(), y: (sh * 0.16).round(), w: SEARCH.0, h: SEARCH.1 };
        let l = Rect { x: s.x + s.w + 16.0, y: s.y, w: LOOK.0, h: LOOK.1 };
        let room = 40.0;
        let shown = Rect { x: s.x - room, y: s.y - room, w: l.x + l.w - s.x + 2.0 * room, h: l.h.max(s.h) + 2.0 * room };
        let k = (area.w / shown.w).min(area.h / shown.h);
        let (ox, oy) = (area.x + area.w / 2.0 - (shown.x + shown.w / 2.0) * k, area.y + area.h / 2.0 - (shown.y + shown.h / 2.0) * k);
        self.stage_picture(frame, area, 8.0, (ox, oy), k, 1.0);
        let skin = Skin::named(&self.settings.skin);
        let example = example(self.lang);
        unsafe { frame.dc.PushAxisAlignedClip(&rect(area.x, area.y, area.w, area.h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        if search > 0.0 {
            // (Coming down a little as it shows.)
            let drop = (1.0 - search) * -8.0;
            let local = Matrix3x2::scale(k, k) * Matrix3x2::translation(ox + s.x * k, oy + (s.y + drop) * k);
            let behind = RECT { left: s.x as i32, top: s.y as i32, right: (s.x + s.w) as i32, bottom: (s.y + s.h) as i32 };
            let (theme, frost) = self.stage.look(skin, self.prefs.theme, behind);
            let scale = self.scale * k;
            faded(frame, area, search, || {
                self.stage.music_ground(frame, &mut guide.grounds[0], &theme, frost, (s.w, s.h), Some(((s.x, s.y), 1.0)), local, scale);
                frame.place(local);
                paint_search(frame, &theme, &example, (s.w, s.h));
                frame.origin(0.0, 0.0);
            });
        }
        if look > 0.0 {
            // (Growing out of the search's first row, beside which it stands.)
            let grow = 0.25 + 0.75 * look;
            let from = l.w * 0.2;
            let local = Matrix3x2::scale(grow * k, grow * k) * Matrix3x2::translation(ox + (l.x + from - from * grow) * k, oy + l.y * k);
            let behind = RECT { left: l.x as i32, top: l.y as i32, right: (l.x + l.w) as i32, bottom: (l.y + l.h) as i32 };
            let (theme, frost) = self.stage.look(skin, self.prefs.theme, behind);
            let scale = self.scale * k * grow;
            faded(frame, area, look.min(1.0), || {
                self.stage.music_ground(frame, &mut guide.grounds[1], &theme, frost, (l.w, l.h), Some(((l.x, l.y), 1.0)), local, scale);
                frame.place(local);
                paint_look(frame, &theme, &example, (l.w, l.h));
                frame.origin(0.0, 0.0);
            });
        }
        unsafe { frame.dc.PopAxisAlignedClip() };
        down
    }

    /// The tip at rest in its own window: its plate, its shadow round it, what it shows on it.
    fn paint_tip(&mut self, frame: &Frame, guide: &Guide, tip: &mut Popup) {
        let palette = self.palette.clone();
        let grounds = Grounds::new(self.dark);
        let p = tip.plate;
        frame.shadow(grounds.shadow, p.x, p.y, p.w, p.h, 8.0, 14.0, 8.0);
        fill(frame, grounds.flyout, p.x, p.y, p.w, p.h, 8.0);
        stroke_inside(frame, p, 8.0, palette.rule);
        self.paint_tip_content(frame, guide, tip);
    }

    /// What the tip shows on its plate: its beak at the editor, its place among the steps, its title and what it asks,
    /// whether it is done, its buttons.
    fn paint_tip_content(&mut self, frame: &Frame, guide: &Guide, tip: &mut Popup) {
        let palette = self.palette.clone();
        let grounds = Grounds::new(self.dark);
        let lang = self.lang;
        tip.hits.clear();
        let p = tip.plate;
        if let Some((side, down)) = tip.beak {
            // Over the plate's edge where it stands out, outlined but at its foot.
            let (foot, point) = match side {
                Edge::Right => (p.x + p.w - 1.0, p.x + p.w + BEAK),
                _ => (p.x + 1.0, p.x - BEAK),
            };
            let (top, bottom) = (p.y + down - BEAK, p.y + down + BEAK);
            frame.fill_shape(&[Point { x: foot, y: top }, Point { x: point, y: p.y + down }, Point { x: foot, y: bottom }], crate::ui::canvas::Fill::Solid(grounds.flyout));
            frame.stroke(&[Point { x: foot, y: top }, Point { x: point, y: p.y + down }, Point { x: foot, y: bottom }], palette.rule, 1.0);
        }
        let words = words(Step::Layout, Edge::Left, lang);
        let (x, inner) = (p.x + TIP_PAD, p.w - 2.0 * TIP_PAD);
        let mut y = p.y + TIP_PAD;
        frame.text(&format!("{} / {STEPS}", Step::Layout as usize + 1), SMALL, palette.text3, x, y + 3.0, inner, Align::End);
        frame.text(words.title, TIP_TITLE, palette.text, x, y, inner - 40.0, Align::Start);
        y += 24.0;
        frame.text_wrapped(&words.body, BODY, palette.text2, x, y, inner);
        y += frame.gfx.wrapped_height(&words.body, BODY, inner) + 10.0;
        paint_state(frame, &palette, &grounds, &words, guide.done[Step::Layout as usize], x, y + 10.0, inner);
        y += 20.0 + 14.0;
        paint_buttons(frame, &palette, &grounds, tip, &buttons(Step::Layout, lang), x + inner, y + BUTTON / 2.0, 72.0);
    }

    /// Whether a press at `x`, `y` (physical px) on the guide's window `hwnd` moves it (and the window with it): on the
    /// dialog, where it takes no press, the window not maximized.
    fn guide_caption(&self, hwnd: HWND, x: f32, y: f32) -> bool {
        if unsafe { IsZoomed(self.hwnd) }.as_bool() {
            return false;
        }
        let Some(dialog) = self.guide.as_ref().and_then(|guide| guide.dialog.as_ref()).filter(|dialog| dialog.hwnd == hwnd) else { return false };
        let (x, y) = (x / dialog.scale - dialog.hits_from.0, y / dialog.scale - dialog.hits_from.1);
        let on = x >= 0.0 && y >= 0.0 && x < dialog.plate.w && y < dialog.plate.h;
        on && !dialog.hits.iter().any(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }

    /// The pointer on the guide's window `hwnd` at `at` (physical px; none: gone from it), pressed (true) or let go
    /// (false) there: let go on what it was pressed on, that done.
    fn guide_pointer(&mut self, hwnd: HWND, at: Option<(f32, f32)>, press: Option<bool>) {
        let Some(guide) = self.guide.as_mut() else { return };
        let Some(popup) = [guide.dialog.as_mut(), guide.tip.as_mut()].into_iter().flatten().find(|popup| popup.hwnd == hwnd) else { return };
        popup.pointer = at.map(|(x, y)| (x / popup.scale - popup.hits_from.0, y / popup.scale - popup.hits_from.1));
        let hit = match press {
            Some(true) => {
                popup.pressed = popup.hovered();
                None
            }
            Some(false) => popup.pressed.take().filter(|&pressed| popup.hovered() == Some(pressed)),
            None => None,
        };
        self.guide_frame = Instant::now();
        if let Some(hit) = hit {
            self.guide_hit(hit);
        }
    }

    /// Does what `hit` does: leaving it goes to its end (and from there, ends it). Nothing while it changes step.
    fn guide_hit(&mut self, hit: Hit) {
        let Some(step) = self.guide.as_ref().filter(|guide| guide.change.is_none()).map(|guide| guide.step) else { return };
        match hit {
            Hit::Skip if step == Step::Ready => self.leave_guide(),
            Hit::Skip => self.guide_to(Step::Ready),
            Hit::Back => {
                if let Some(before) = step.before() {
                    self.guide_to(before);
                }
            }
            Hit::Next => match step.after() {
                Some(after) => self.guide_to(after),
                None => self.leave_guide(),
            },
            Hit::Search => crate::ui::find::toggle(),
            Hit::Look => crate::ui::find::preview(),
        }
    }

    /// The dialog `hwnd` moved to a screen of another scale.
    fn guide_scaled(&mut self, hwnd: HWND, scale: f32) {
        if let Some(dialog) = self.guide.as_mut().and_then(|guide| guide.dialog.as_mut()).filter(|dialog| dialog.hwnd == hwnd) {
            dialog.scale = scale;
        }
    }

    /// A component put on an edge by hand: the layout's step done, where it shows.
    pub(super) fn guide_saw_layout(&mut self) {
        if let Some(guide) = self.guide.as_mut().filter(|guide| guide.step == Step::Layout) {
            guide.done[Step::Layout as usize] = true;
        }
    }

    /// Where the guide's play on the layout page has the pointer, while it plays.
    pub(super) fn played_hand(&self) -> Option<(f32, f32)> {
        self.guide.as_ref()?.drag.as_ref()?.pointer
    }

    /// The pointer of the guide's play on the layout page, over all the window shows.
    pub(super) fn paint_played_pointer(&self, frame: &Frame) {
        if let Some(at) = self.played_hand() {
            paint_pointer(frame, at, 20.0);
        }
    }

    /// Where `spot` is in the window (DIPs), as last drawn: none, not drawn (yet).
    fn spot_at(&self, drag: &Drag, spot: Spot) -> Option<(f32, f32)> {
        let zone = |edge: Edge| self.zones.iter().find(|zone| zone.screen == drag.screen && zone.edge == edge).map(|zone| zone.rect);
        match spot {
            Spot::Rest => self.editor_at.map(|r| (r.x + r.w * 0.6, r.y + r.h - 30.0)),
            Spot::Tile(part) => self.targets.iter().find(|(_, target)| *target == Target::Library(part)).map(|(r, _)| (r.x + r.w / 2.0, r.y + r.h * 0.4)),
            Spot::Edge => zone(Edge::Left).map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0)),
            Spot::Pill => self.targets.iter().find(|(_, target)| matches!(target, Target::Chip(screen, Edge::Left, _) if *screen == drag.screen)).map(|(r, _)| (r.x + r.w * 0.35, r.y + r.h / 2.0)),
            Spot::Middle => {
                let (left, right) = (zone(Edge::Left)?, zone(Edge::Right)?);
                Some(((left.x + right.x + right.w) / 2.0, left.y + left.h / 2.0))
            }
        }
    }

    /// The guide's play on the layout page moved on to `now`: the pointer where its move has it, what the moves passed
    /// do done (a component taken up, or let go on an edge or off it), as the window was last drawn.
    pub(super) fn play_drag(&mut self, now: Instant) {
        let Some(mut drag) = self.guide.as_mut().filter(|guide| guide.change.is_none()).and_then(|guide| guide.drag.take()) else { return };
        // First shown at the foot of the editor, a moment before it plays (once the change to its step is over).
        if drag.pointer.is_none() {
            if let Some(rest) = self.spot_at(&drag, Spot::Rest) {
                drag.pointer = Some(rest);
                drag.from = Some(rest);
                drag.since = now + Duration::from_millis(500);
            }
        }
        while drag.pointer.is_some() {
            let made = drag.moves[drag.at];
            // (Bound for the pill: the panel its edge shows put away, the pill there in its place.)
            if made.to == Spot::Pill {
                self.shown_edge = None;
            }
            let Some(to) = self.spot_at(&drag, made.to) else { break };
            let ms = now.saturating_duration_since(drag.since).as_millis() as f32;
            let from = drag.from.unwrap_or(to);
            let along = if made.glide == 0 { 1.0 } else { GLIDE.at((ms / made.glide as f32).clamp(0.0, 1.0)) };
            drag.pointer = Some((from.0 + (to.0 - from.0) * along, from.1 + (to.1 - from.1) * along));
            if ms < (made.glide + made.wait) as f32 {
                break;
            }
            match made.act {
                Act::Take(part, off_edge) => {
                    self.carry = Some(Carry { part, from: off_edge.then(|| (drag.screen.clone(), Edge::Left)), start: to, moved: true });
                    self.shown_edge = None;
                }
                Act::Put => {
                    if let Some(carry) = self.carry.take() {
                        self.put(carry, Some(to));
                    }
                }
                Act::Wait => {}
            }
            drag.from = Some(to);
            drag.since += Duration::from_millis(made.glide + made.wait);
            drag.at = (drag.at + 1) % drag.moves.len();
        }
        if let Some(guide) = self.guide.as_mut() {
            guide.drag = Some(drag);
        }
    }

    /// The guide's play on the layout page stopped, where it plays: what it carried put away, the edges as they were
    /// before it.
    pub(super) fn stop_drag(&mut self) {
        let Some(drag) = self.guide.as_mut().and_then(|guide| guide.drag.take()) else { return };
        *self.settings.places_mut() = drag.before;
        self.carry = None;
        self.shown_edge = None;
        self.next_frame = Instant::now();
    }
}

/// Search's sheet as the last step's play shows it, `size` large, its corner at the frame's origin: what is typed, and
/// the files found, the first picked.
fn paint_search(frame: &Frame, theme: &Theme, example: &Example, size: (f32, f32)) {
    let family = theme.body.family;
    let typed = Font::new(family, 22.0, 400.0);
    frame.icon(Icon::Search, Point { x: 30.0, y: 34.0 }, 20.0, theme.text2);
    text_centred(frame, example.typed, typed, theme.text, 54.0, 34.0, size.0 - 80.0, Align::Start);
    // (Its caret blinking, as a caret does.)
    if SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() % 1060 < 530 {
        let end = 54.0 + frame.gfx.measure(example.typed, typed) + 2.0;
        frame.fill(theme.text, end, 22.0, 1.5, 24.0);
    }
    frame.fill(theme.rule, 0.0, 66.0, size.0, 1.0);
    frame.text(example.group, Font::new(family, 12.0, 400.0), theme.text2, 22.0, 78.0, size.0 - 44.0, Align::Start);
    for (at, (name, folder, kind, colour)) in example.files.iter().enumerate() {
        let y = 100.0 + at as f32 * 60.0;
        if at == 0 {
            frame.fill_rounded(theme.text.alpha(0.1), 8.0, y, size.0 - 16.0, 56.0, 8.0);
        }
        // The file's icon: a page, its kind on a band of its colour.
        let page = Rect { x: 24.0, y: y + 10.0, w: 28.0, h: 36.0 };
        frame.fill_rounded(Color::hex(0xFFFFFF, 1.0), page.x, page.y, page.w, page.h, 3.0);
        frame.fill(Color::hex(*colour, 1.0), page.x, page.y + 20.0, page.w, 11.0);
        centred(frame, kind, Font::new(family, 7.0, 800.0), Color::hex(0xFFFFFF, 1.0), Rect { x: page.x, y: page.y + 20.0, w: page.w, h: 11.0 });
        frame.text(name, Font::new(family, 14.0, 400.0), theme.text, 66.0, y + 9.0, size.0 - 90.0, Align::Start);
        frame.text(folder, Font::new(family, 12.0, 400.0), theme.text2, 66.0, y + 30.0, size.0 - 90.0, Align::Start);
    }
}

/// The look at the first file found as the last step's play shows it, `size` large, its corner at the frame's origin:
/// its name and size, and the page it shows.
fn paint_look(frame: &Frame, theme: &Theme, example: &Example, size: (f32, f32)) {
    let family = theme.body.family;
    let name = example.files[0].0;
    let bold = Font::new(family, 14.0, 600.0);
    frame.text(name, bold, theme.text, 16.0, 14.0, size.0 - 32.0, Align::Start);
    let after = 16.0 + frame.gfx.measure(name, bold) + 10.0;
    frame.text(example.size, Font::new(family, 12.0, 400.0), theme.text2, after, 16.0, (size.0 - after - 16.0).max(0.0), Align::Start);
    let page = Rect { x: 12.0, y: 44.0, w: size.0 - 24.0, h: size.1 - 56.0 };
    frame.fill_rounded(Color::hex(0xFBFBF9, 1.0), page.x, page.y, page.w, page.h, 6.0);
    let (ink, rule) = (Color::hex(0x22262C, 1.0), Color::hex(0xE5E5E2, 1.0));
    frame.text(example.heading, Font::new(family, 17.0, 700.0), Color::hex(0x173A6B, 1.0), page.x + 22.0, page.y + 22.0, page.w - 44.0, Align::Start);
    let line = Font::new(family, 13.0, 400.0);
    for (at, (what, formula)) in example.lines.iter().enumerate() {
        let y = page.y + 66.0 + at as f32 * 40.0;
        frame.text(what, line, ink, page.x + 22.0, y, page.w / 2.0, Align::Start);
        frame.text(formula, line, ink, page.x + 22.0, y, page.w - 44.0, Align::End);
        frame.fill(rule, page.x + 22.0, y + 28.0, page.w - 44.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first step's play runs on round with nothing jumping: the pointer and the panel move a little from one
    /// frame to the next, across where it begins again too; the pointer reaches the edge, the panel slides all the way
    /// out, and both are back where they began.
    #[test]
    fn the_push_s_play_never_jumps() {
        let (mut along_most, mut out_most) = (0.0f32, 0.0f32);
        let mut was = pushing(Duration::ZERO);
        assert_eq!(was, (0.0, 0.0));
        for ms in (8..=2 * PUSH_LOOP).step_by(8) {
            let now = pushing(Duration::from_millis(ms));
            assert!((now.0 - was.0).abs() < 0.05 && (now.1 - was.1).abs() < 0.12, "{ms} ms: {was:?} to {now:?}");
            (along_most, out_most) = (along_most.max(now.0), out_most.max(now.1));
            was = now;
        }
        assert!(along_most > 0.999 && out_most >= 1.0, "{along_most} {out_most}");
    }

    /// The steps go one after another, each back to the one before, and each but the word that it is done has its
    /// place among them in order.
    #[test]
    fn the_steps_go_in_order() {
        let mut step = Step::Push;
        let mut seen = vec![step];
        while let Some(after) = step.after() {
            assert_eq!(after.before(), Some(step));
            step = after;
            seen.push(step);
        }
        assert_eq!(seen, [Step::Push, Step::Size, Step::Tear, Step::Layout, Step::Keys, Step::Ready]);
        assert_eq!(seen.iter().map(|step| step.index()).collect::<Vec<_>>(), [Some(0), Some(1), Some(2), Some(3), Some(4), None]);
        assert_eq!(STEPS, seen.len() - 1);
    }

    /// A panel at the right of a screen (its fractions), centred on its edge as Glance has it, as large as `size`.
    fn panel_at(size: f32) -> Rect {
        let (w, h) = (0.37 * size, 0.69 * size);
        Rect { x: 0.99 - w, y: 0.5 - h / 2.0, w, h }
    }

    /// The second step's play runs on round with nothing jumping, across where it begins again too: the pointer and
    /// the panel move a little from one frame to the next. The pointer that sizes shows only with the pointer on the
    /// corner, never as it glides there; held, the hand is on the corner of the panel as large as it is drawn, so that
    /// the corner follows it. The panel is made larger and smaller, double-clicked back to its own size, and each
    /// click leaves a ring.
    #[test]
    fn the_size_s_play_never_jumps() {
        let place = |mark: SizeMark| match mark {
            SizeMark::Inside => inside(panel_at(1.0)),
            SizeMark::Corner(size) => free_corner(Edge::Right, panel_at(size)),
            SizeMark::Aside(size) => aside(Edge::Right, panel_at(size)),
        };
        let was = sizing(Duration::ZERO);
        assert_eq!((was.size, was.hand, was.sizes, was.held), (1.0, Hand::at(SizeMark::Inside), false, false));
        let (mut was, mut was_at) = (was, was.hand.point(place));
        let (mut most, mut least, mut rings) = (1.0f32, 1.0f32, 0);
        for ms in (5..=2 * SIZE_LOOP).step_by(5) {
            let now = sizing(Duration::from_millis(ms));
            let at = now.hand.point(place);
            assert!((at.0 - was_at.0).abs() < 0.02 && (at.1 - was_at.1).abs() < 0.02, "{ms} ms: the pointer from {was_at:?} to {at:?}");
            assert!((now.size - was.size).abs() < 0.04, "{ms} ms: the panel from {} to {}", was.size, now.size);
            if now.sizes {
                assert!(now.hand.from == now.hand.to && matches!(now.hand.to, SizeMark::Corner(_)), "{ms} ms: sizing off the corner {:?}", now.hand);
            }
            // (Held but for the clicks, which leave their rings.)
            if now.held && now.ring.is_none() {
                assert_eq!(now.hand, Hand::at(SizeMark::Corner(now.size)), "{ms} ms: the corner left behind");
            }
            if now.ring.is_some() && was.ring.is_none_or(|was| now.ring.unwrap() < was) {
                rings += 1;
            }
            (most, least) = (most.max(now.size), least.min(now.size));
            (was, was_at) = (now, at);
        }
        assert!(most >= GROWN - 1e-4 && least <= SHRUNK + 1e-4, "{most} {least}");
        assert_eq!(rings, 4, "two clicks a round");
        let end = sizing(Duration::from_millis(SIZE_LOOP - 1));
        let (at, rest) = (end.hand.point(place), inside(panel_at(1.0)));
        assert!(end.size == 1.0 && !end.sizes && (at.0 - rest.0).abs() < 1e-5 && (at.1 - rest.1).abs() < 1e-5, "{end:?}");
    }

    /// The second step's play rests between its changes: grown, the panel stays so a moment before it is made
    /// smaller (long enough for a turn into other columns to end); let go, the pointer moves off the corner, a plain
    /// pointer, and rests there a while before it comes back to double-click it as a plain pointer; back to its own
    /// size, nothing moves a while before the pointer goes back onto the panel.
    #[test]
    fn the_size_s_play_rests_before_each_change() {
        // How long the play is still before `at` (ms): the panel as large, the pointer where it is, no click.
        let still_before = |at: f32| {
            let then = sizing(Duration::from_millis(at as u64 - 1));
            let mut ms = at as u64 - 1;
            while ms > 0 {
                let was = sizing(Duration::from_millis(ms - 1));
                if was.size != then.size || was.hand != then.hand || was.held != then.held {
                    break;
                }
                ms -= 1;
            }
            at - ms as f32
        };
        let grown = sizing(Duration::from_millis(SHRINK_AT as u64 - 1));
        assert!(grown.size == GROWN && grown.held, "{grown:?}");
        assert!(still_before(SHRINK_AT) >= crate::widget::MORPH.as_secs_f32() * 1000.0, "{} ms grown", still_before(SHRINK_AT));
        let aside = sizing(Duration::from_millis(RETURN_AT as u64 - 1));
        assert!(aside.size == SHRUNK && !aside.held && !aside.sizes && aside.hand == Hand::at(SizeMark::Aside(SHRUNK)), "{aside:?}");
        assert!(still_before(RETURN_AT) >= SIZE_REST, "{} ms off the corner", still_before(RETURN_AT));
        for click in CLICKS {
            let down = sizing(Duration::from_millis(click as u64 + 1));
            assert!(down.held && !down.sizes && down.hand == Hand::at(SizeMark::Corner(SHRUNK)), "{down:?}");
        }
        assert!(still_before(BACK_AT) >= SIZE_REST, "{} ms before going back", still_before(BACK_AT));
    }

    /// The third step's play runs on round with nothing jumping, across where it begins again too: the pointer and
    /// each widget move a little from one frame to the next, a widget comes off under the hand where it was taken and
    /// fades out from where it was put down; the panel goes at once only while a widget torn off it is in hand.
    #[test]
    fn the_tear_s_play_never_jumps() {
        let body = panel_at(1.0);
        let place = |mark: TearMark| match mark {
            TearMark::Inside => inside(body),
            TearMark::Head => (body.x + 0.05, body.y + 0.03),
            TearMark::Foot => (body.x + 0.1, body.y + 0.2),
            TearMark::Edge => (0.997, 0.5),
            TearMark::LaneDrop => (0.1, 0.12),
            TearMark::WholeDrop => (0.4, 0.4),
        };
        let start = tearing(Duration::ZERO);
        assert_eq!((start.out, start.hand, start.held, start.lane, start.whole), (1.0, Hand::at(TearMark::Inside), false, None, None));
        let mut was = start;
        let (mut lanes, mut wholes) = (0, 0);
        for ms in (5..=2 * TEAR_LOOP).step_by(5) {
            let now = tearing(Duration::from_millis(ms));
            let (a, b) = (was.hand.point(place), now.hand.point(place));
            assert!((a.0 - b.0).abs() < 0.03 && (a.1 - b.1).abs() < 0.03, "{ms} ms: the pointer from {a:?} to {b:?}");
            for (name, was_widget, widget, taken) in [("lane", was.lane, now.lane, TearMark::Head), ("whole", was.whole, now.whole, TearMark::Foot)] {
                match (was_widget, widget) {
                    (Some((a, a_shown)), Some((b, b_shown))) => {
                        let (a, b) = (a.point(place), b.point(place));
                        assert!((a.0 - b.0).abs() < 0.03 && (a.1 - b.1).abs() < 0.03, "{ms} ms: the {name} from {a:?} to {b:?}");
                        assert!((a_shown - b_shown).abs() < 0.02, "{ms} ms: the {name} from {a_shown} to {b_shown} opaque");
                    }
                    (None, Some((hand, _))) => {
                        let (at, from) = (hand.point(place), place(taken));
                        assert!(now.held && (at.0 - from.0).abs() < 0.01 && (at.1 - from.1).abs() < 0.01, "{ms} ms: the {name} came at {at:?}");
                        if name == "lane" { lanes += 1 } else { wholes += 1 }
                    }
                    (Some((_, shown)), None) => assert!(shown < 0.02, "{ms} ms: the {name} went at {shown}"),
                    (None, None) => {}
                }
            }
            let in_hand = |play: &Tearing| play.held && (play.lane.is_some_and(|(hand, _)| hand.from != hand.to) || play.whole.is_some_and(|(hand, _)| hand.from != hand.to));
            assert!((now.out - was.out).abs() < 0.12 || (in_hand(&was) && in_hand(&now) && now.out == 0.0), "{ms} ms: the panel from {} to {}", was.out, now.out);
            was = now;
        }
        assert_eq!((lanes, wholes), (2, 2), "each torn off once a round");
        let end = tearing(Duration::from_millis(TEAR_LOOP - 1));
        let (at, rest) = (end.hand.point(place), inside(body));
        assert!((at.0 - rest.0).abs() < 1e-5 && (at.1 - rest.1).abs() < 1e-5 && end.lane.is_none() && end.whole.is_none() && !end.held, "{end:?}");
        assert!((end.out - 1.0).abs() < 1e-4, "{}", end.out);
    }

    /// The last step's play presses each shortcut's keys one after another and lets them go, search shows as the first
    /// is pressed and the look only while search does; it ends as it begins, nothing down and nothing up.
    #[test]
    fn the_keys_play_presses_each_shortcut_then_lets_go() {
        for keys in [(3, 2), (2, 1), (1, 3)] {
            let keying = Keying::new(keys);
            let (mut most, mut searched, mut looked) = ([0, 0], false, false);
            for ms in (0..keying.end as u64 + 200).step_by(5) {
                let (down, search, look) = keying.at(Duration::from_millis(ms));
                most = [most[0].max(down[0]), most[1].max(down[1])];
                assert!(down[0] <= keys.0 && down[1] <= keys.1);
                assert!(look <= 0.0 || search >= 1.0, "{ms} ms: the look without search ({search})");
                searched |= search >= 1.0;
                looked |= look >= 1.0;
            }
            assert_eq!(most, [keys.0, keys.1]);
            assert!(searched && looked);
            assert_eq!(keying.at(Duration::ZERO), ([0, 0], 0.0, 0.0));
            assert_eq!(keying.at(Duration::from_millis(keying.end as u64 - 1)), ([0, 0], 0.0, 0.0));
        }
    }

    /// The window under the dialog, held at rest on a step, is drawn once with what it shows as that comes in (a
    /// desktop read after it was first drawn, which the guide's small desktop shows too); through a change drawn for,
    /// it is held, and drawn with it after.
    #[test]
    fn what_comes_in_under_the_dialog_is_drawn_once_at_rest() {
        let now = Instant::now();
        for step in [Step::Push, Step::Size, Step::Tear, Step::Keys, Step::Ready] {
            assert!(holds(None, step, false), "{step:?}: held at rest, nothing new");
            assert!(!holds(None, step, true), "{step:?}: drawn once what it shows has come in");
        }
        assert!(!holds(None, Step::Layout, false), "the layout's step: never held at rest");
        let mut change = Change { kind: Kind::Swap, from: Step::Keys, began: Some(now), back: false, from_since: now, heights: (560.0, 470.0), lead: 0.0, window_drawn: false, quick: false };
        assert!(!holds(Some(&change), Step::Ready, false), "a change not drawn for yet: drawn");
        change.window_drawn = true;
        assert!(holds(Some(&change), Step::Ready, true), "through a change drawn for: held, what came in taken in after it");
    }

    /// Every change of the guide goes on with nothing jumping: the dialog and the cover over the window grow, fade and
    /// slide a little from one frame to the next; what comes shows from nothing, what goes ends in nothing; the content
    /// changes for the next only while none of it shows; between the dialog and the tip, the plate is carried evenly,
    /// never with what both show on it at once, what comes showing as it goes (half in before it is nearly there); the cover darkens and lightens evenly; and each ends as it is at rest
    /// (the tip's in its own window).
    #[test]
    fn a_change_of_step_never_jumps() {
        let now = Instant::now();
        let heights = (560.0, 470.0);
        let changes = [
            (Kind::Opening, Step::Push, Step::Push, 0.0),
            (Kind::Opening, Step::Push, Step::Push, WINDOW_LEAD),
            (Kind::Ending, Step::Ready, Step::Ready, 0.0),
            (Kind::Swap, Step::Keys, Step::Ready, 0.0),
            (Kind::Swap, Step::Ready, Step::Keys, 0.0),
            (Kind::ToTip, Step::Push, Step::Layout, 0.0),
            (Kind::FromTip, Step::Layout, Step::Keys, 0.0),
        ];
        let steepest_cover = 1.0 / COVER_IN.min(COVER_OUT).min(CARRIED) + 1e-4;
        for (kind, from, to, lead) in changes {
            let change = Change { kind, from, began: Some(now), back: from.before() == Some(to), from_since: now, heights, lead, window_drawn: false, quick: false };
            let at_rest = if kind == Kind::Swap { heights.1 } else { heights.0 };
            let look = |ms: f32| dialog_look(Some((&change, ms)), to, at_rest);
            let mut was = look(0.0);
            let mut was_cover = cover_look(Some((&change, 0.0)), to);
            // (Between the dialog and the tip: when what comes is half in, and when the plate is nearly where it goes.)
            let (mut half_in, mut nearly_there) = (None, None);
            // (Shown from the start at rest, else coming from nothing.)
            if let Some(dialog) = was {
                assert!(dialog.opacity >= 1.0 || dialog.opacity <= 0.0, "{kind:?} begins at {dialog:?}");
            }
            assert!(was_cover >= 1.0 || was_cover <= 0.0, "{kind:?} begins covered {was_cover}");
            for ms in 1..=(change.length() as u32 + 20) {
                let dialog = look(ms as f32);
                let cover = cover_look(Some((&change, ms as f32)), to);
                assert!((cover - was_cover).abs() <= steepest_cover, "{kind:?} {ms} ms: covered {was_cover} to {cover}");
                was_cover = cover;
                match (was, dialog) {
                    (Some(a), Some(b)) => {
                        assert!((a.opacity - b.opacity).abs() < 0.08 && (a.scale - b.scale).abs() < 0.005 && (a.height - b.height).abs() < 3.0, "{kind:?} {ms} ms: {a:?} to {b:?}");
                        assert!((a.content - b.content).abs() < 0.08 && (a.footer - b.footer).abs() < 0.08 && (a.buttons - b.buttons).abs() < 0.08 && (a.tip - b.tip).abs() < 0.08, "{kind:?} {ms} ms: {a:?} to {b:?}");
                        assert!((a.carried - b.carried).abs() < 0.01, "{kind:?} {ms} ms: carried {a:?} to {b:?}");
                        assert!(a.old == b.old || a.content.max(b.content).max(a.buttons).max(b.buttons) < 0.08, "{kind:?} {ms} ms: changed while shown");
                        assert!((a.slide - b.slide).abs() < 2.0 || a.content.max(b.content) < 0.08, "{kind:?} {ms} ms: {a:?} to {b:?}");
                        // (Never the dialog's content and the tip's on the plate at once.)
                        assert!(b.content.max(b.footer).max(b.buttons) < 0.08 || b.tip < 0.08, "{kind:?} {ms} ms: both shown {b:?}");
                    }
                    (None, Some(b)) => assert!(b.opacity < 0.08, "{kind:?} {ms} ms: came at {b:?}"),
                    (Some(a), None) => assert!(a.opacity < 0.08, "{kind:?} {ms} ms: went at {a:?}"),
                    (None, None) => {}
                }
                if let Some(b) = dialog {
                    let (coming, carried) = match kind {
                        Kind::ToTip => (b.tip, b.carried),
                        Kind::FromTip => (b.content, 1.0 - b.carried),
                        _ => (0.0, 0.0),
                    };
                    if coming >= 0.5 {
                        half_in.get_or_insert(ms);
                    }
                    if carried >= 0.95 {
                        nearly_there.get_or_insert(ms);
                    }
                }
                was = dialog;
            }
            // What comes shows as the plate goes, not once it is there.
            if matches!(kind, Kind::ToTip | Kind::FromTip) {
                assert!(half_in < nearly_there, "{kind:?}: half in at {half_in:?} ms, the plate nearly there at {nearly_there:?} ms");
            }
            // Over: as at rest (gone, where it ended; where it was carried to the tip, the tip as it rests).
            match kind {
                Kind::Ending => {
                    assert!(was.is_some_and(|dialog| dialog.opacity <= 0.0), "{was:?}");
                    assert!(was_cover <= 0.0, "{kind:?} ends covered {was_cover}");
                }
                Kind::ToTip => {
                    let end = was.expect("the plate at the tip's place");
                    assert!(end.carried >= 1.0 && end.tip >= 1.0 && end.content <= 0.0 && end.footer <= 0.0 && end.buttons <= 0.0 && end.opacity >= 1.0 && end.scale == 1.0, "{end:?}");
                    assert!(dialog_look(None, to, at_rest).is_none() && tip_rests(None, to));
                    assert_eq!(was_cover, cover_look(None, to), "{kind:?}");
                }
                _ => {
                    assert_eq!(was, dialog_look(None, to, at_rest), "{kind:?}");
                    assert!(!tip_rests(None, to));
                    assert_eq!(was_cover, cover_look(None, to), "{kind:?}");
                }
            }
        }
    }

    /// The layout page's play, played round as the window would play it, leaves every edge as it found it, whatever
    /// the edge it plays on held; and each round begins with the pointer gliding on from where the last left it.
    #[test]
    fn the_layout_s_play_leaves_the_edges_as_they_were() {
        let screen = "screen";
        for held in [Some(Part::Workbench), Some(Part::Music), None] {
            let mut places = Places::default();
            match held {
                Some(part) => places.put(screen, Edge::Left, part, None),
                None => places.take(screen, Edge::Left),
            }
            let before: Vec<Option<Part>> = EDGES.iter().map(|&edge| places.part(screen, edge)).collect();
            let put = played_part(held);
            assert_ne!(Some(put), held);
            let moves = script(put, held);
            assert!(moves[0].glide > 0, "a round begins gliding on");
            let mut carried: Option<(Part, bool)> = None;
            let mut put_down = false;
            for made in &moves {
                match made.act {
                    Act::Take(part, off_edge) => carried = Some((part, off_edge)),
                    Act::Put => {
                        let (part, off_edge) = carried.take().expect("carrying");
                        let from = off_edge.then_some((screen, Edge::Left));
                        match made.to {
                            Spot::Edge => places.put(screen, Edge::Left, part, from),
                            _ => places.take(screen, Edge::Left),
                        }
                        put_down |= places.part(screen, Edge::Left) == Some(put);
                    }
                    Act::Wait => {}
                }
            }
            assert!(put_down, "it shows the component put on the edge");
            assert!(carried.is_none());
            assert_eq!(EDGES.iter().map(|&edge| places.part(screen, edge)).collect::<Vec<_>>(), before, "held {held:?}");
        }
    }
}
