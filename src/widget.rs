//! Desktop widgets: the panel's readings torn off onto the desktop, each in
//! a window of its own above the others. Sized by its edges, a widget shows
//! as much as fits (see `arrange::Ladder`), each reading moving from its
//! place in one layout to its place in the next (see `morph`); let go near the edge of its screen, it sticks to it as it
//! is; flung, it slides away and is put away. Pinned, it is neither moved
//! nor sized; letting clicks through, Ctrl held lets it take them again.
//! Moved near an edge, where it would stick shows before it is let go.
//! What a widget shows is its content (see `Content`): the panel's readings,
//! or what plays (see `music`); its shell (moved, sized, stuck, flung, kept
//! above the others or on the desktop, its buttons) is the same for each.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_LAYER_PARAMETERS1};
use windows::Win32::UI::Input::KeyboardAndMouse::SetCapture;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, PostMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WM_APP};
use windows_numerics::{Matrix3x2, Vector2};

use crate::journal::note;

/// How long a kind of note that can come many times a second (a widget
/// being dragged takes the desktop again and again, and may draw slowly
/// all the while) is kept to one: the log is for what went wrong, and a
/// user's report carries only its last lines.
const SPARINGLY: Duration = Duration::from_secs(30);

/// Notes `what` of `kind`, unless one of that kind was noted within
/// `SPARINGLY`: then it is only counted, and the next one noted says how
/// many came between.
pub(crate) fn note_sparingly(kind: &'static str, what: impl FnOnce() -> String) {
    static LAST: std::sync::Mutex<Vec<(&'static str, Instant, u32)>> = std::sync::Mutex::new(Vec::new());
    let mut last = LAST.lock().unwrap();
    let now = Instant::now();
    match last.iter_mut().find(|(of, _, _)| *of == kind) {
        Some((_, at, between)) if now.duration_since(*at) < SPARINGLY => *between += 1,
        Some((_, at, between)) => {
            let more = std::mem::take(between);
            *at = now;
            drop(last);
            note(if more > 0 { format!("{} ({more} more like it since the last)", what()) } else { what() });
        }
        None => {
            last.push((kind, now, 0));
            drop(last);
            note(what());
        }
    }
}
use crate::mouse::{cursor_position, monitor_at, Contact};
use crate::settings::{WidgetAt, WidgetSide, MUSIC_FORM};

/// What a widget showing one lane alone keeps before its module's id and its form (see `form_of`).
const ONLY_FORM: &str = "only:";

/// The form a widget's readings are kept as: `form`, or for one lane alone (`only`), its module's id and its form
/// after `ONLY_FORM` (a form an older Glance does not know: it leaves the widget out, rather than show all the lanes).
fn form_of(only: Option<&str>, form: String) -> String {
    match only {
        Some(id) => format!("{ONLY_FORM}{id}|{form}"),
        None => form,
    }
}

/// A kept form read back (see `form_of`): the one lane it shows, if it shows one, and its form.
fn only_of(kept: &str) -> (Option<String>, String) {
    // (A module's id may hold ':', as a note stuck on the desktop's does; '|' none, nor a file's name.)
    match kept.strip_prefix(ONLY_FORM).and_then(|rest| rest.split_once('|')) {
        Some((id, form)) => (Some(id.to_string()), form.to_string()),
        None => (None, kept.to_string()),
    }
}
use crate::ui::buttons::{self, Button};
use crate::ui::arrange::{self, Choice, Form, Want};
use crate::ui::backdrop::Capture;
use crate::ui::canvas::{Canvas, Color, Fill, Point};
use crate::ui::forms::{self, Small};
use crate::ui::gfx::{self, Gfx, Layer, Surface};
use crate::ui::icons::Icon;
use crate::ui::morph::{self, Part};
use crate::ui::motion::Spring;
use crate::ui::music::{self, Music, Shows};
use crate::ui::prefs::{MusicGround, Prefs};
use crate::ui::life::Lives;
use crate::ui::render::{self, PanelLayers};
use crate::ui::seen::Seen;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Skin, Theme};
use crate::ui::view::{self, Scene};
use crate::ui::window::Window;
use crate::reading::Sample;
use crate::settings::Edge;

/// How near its edge the pointer sizes a widget rather than moving it (DIPs).
const EDGE: f32 = 8.0;
/// How near a corner, along either of its edges, the pointer sizes both
/// (DIPs): a corner easy to take without turning into one edge.
const CORNER: f32 = 20.0;

/// The edges a press at (`x`, `y`) on a widget `w` × `h` sizes: left,
/// right, top, bottom.
pub(crate) fn edges_at(x: f32, y: f32, w: f32, h: f32) -> (bool, bool, bool, bool) {
    // (Past the band round its edge, over its shadow (which its window holds too): none, as a press there takes
    // nothing, nor the pointer's shape says one would.)
    if x < -EDGE || y < -EDGE || x > w + EDGE || y > h + EDGE {
        return (false, false, false, false);
    }
    let (left, right, top, bottom) = (x < EDGE, x > w - EDGE, y < EDGE, y > h - EDGE);
    let (side, end) = (left || right, top || bottom);
    (left || end && x < CORNER, right || end && x > w - CORNER, top || side && y < CORNER, bottom || side && y > h - CORNER)
}
/// The edge a widget let go is stuck to and where along it its middle is (physical px): the edge of its screen's work
/// area it is let go near (`near`), else the one it was stuck to (`stuck`, with where its middle was along it); its
/// middle where it is let go (`rect`), slid along its edge or not. None, near none and stuck to none.
fn stuck_after(near: Option<Side>, stuck: Option<(Side, i32)>, rect: RECT) -> Option<(Side, i32)> {
    let side = near.or(stuck.map(|(side, _)| side))?;
    Some((side, middle_along(rect, side)))
}

/// Where the middle of `rect` is along `side` (physical px).
fn middle_along(rect: RECT, side: Side) -> i32 {
    match side {
        Side::Left | Side::Right => (rect.top + rect.bottom) / 2,
        Side::Top | Side::Bottom => (rect.left + rect.right) / 2,
    }
}

/// The side across `side` of `work` (a work area) that `rect` (physical px) is within `within` DIPs of (`scale`
/// physical px a DIP), the nearer where it is near both: stuck to both, it is stuck in their corner.
fn corner_side(work: RECT, rect: RECT, scale: f32, side: Side, within: f32) -> Option<Side> {
    let across = match side {
        Side::Left | Side::Right => [(Side::Top, rect.top - work.top), (Side::Bottom, work.bottom - rect.bottom)],
        Side::Top | Side::Bottom => [(Side::Left, rect.left - work.left), (Side::Right, work.right - rect.right)],
    };
    across.into_iter().filter(|(_, away)| (*away as f32 / scale) < within).min_by_key(|(_, away)| *away).map(|(side, _)| side)
}

/// Where a surface `size` large (physical px) stuck to `side` of the work area `work`, its middle `along` it, has its
/// corner, all in the work area; along the side where it is now (its corner `at`) for none. Stuck to the side across
/// it `also`, in their corner (where along `side` it was is not asked).
fn stuck_at(work: RECT, at: POINT, side: Side, along: Option<i32>, also: Option<Side>, size: (i32, i32)) -> POINT {
    let flush = |side: Side, (x, y): (i32, i32)| match side {
        Side::Left => (work.left, y),
        Side::Right => (work.right - size.0, y),
        Side::Top => (x, work.top),
        Side::Bottom => (x, work.bottom - size.1),
    };
    let (x, y) = flush(side, (at.x, at.y));
    let x = match (side, along) {
        (Side::Top | Side::Bottom, Some(middle)) => middle - size.0 / 2,
        _ => x,
    };
    let y = match (side, along) {
        (Side::Left | Side::Right, Some(middle)) => middle - size.1 / 2,
        _ => y,
    };
    let (x, y) = also.map_or((x, y), |also| flush(also, (x, y)));
    let x = x.clamp(work.left, (work.right - size.0).max(work.left));
    let y = y.clamp(work.top, (work.bottom - size.1).max(work.top));
    POINT { x, y }
}

/// Whether a widget's window lets a click at `cursor` through to what is under it: everywhere, set to (`setting`),
/// but on the button that ends that (`reached`); and, as the panel's window does, over its shadow: in its `window`, off
/// where it takes the pointer (`taking`), unless it is in hand (`held`).
fn clicks_through(setting: bool, reached: bool, cursor: Option<POINT>, window: RECT, taking: RECT, held: bool) -> bool {
    let inside = |r: RECT, c: POINT| (r.left..r.right).contains(&c.x) && (r.top..r.bottom).contains(&c.y);
    let shadow = !held && cursor.is_some_and(|c| inside(window, c) && !inside(taking, c));
    setting && !reached || shadow
}

/// How far the pointer moves a pressed widget before it moves (DIPs).
const MOVE_FROM: f32 = 4.0;
/// How far past what its form fills a widget's edge follows the hand:
/// half the way, as a band would stretch, loose enough that the way to a
/// layout showing much more is not a long pull.
pub(crate) const GIVE: f32 = 0.5;
/// How long a widget let go after it was sized, or stuck to an edge, takes
/// to settle onto what it shows.
pub(crate) const SETTLE: Duration = Duration::from_millis(260);
/// How fast a widget let go (DIPs a millisecond, over the last moments of
/// the hand) is flung rather than put down.
const FLING: f32 = 1.1;
/// How long the hand's last moments are, for its speed.
const FLING_SPAN: Duration = Duration::from_millis(80);
/// What a flung widget keeps of its speed each millisecond.
const FRICTION: f32 = 0.997;
/// How long a thing on a station's pages is held before it is dragged off them rather than swiped across: a quick
/// stroke across turns its pages, as a phone's do; held a moment first, or drawn away up or down, it is dragged off.
const HOLD: Duration = Duration::from_millis(300);
/// How fast pages let go are flicked on to the next (DIPs a millisecond, over the last moments of the hand).
const FLICK: f32 = 0.3;
/// How near the edge of its screen's work area a widget let go sticks to
/// it (DIPs), and how far from it, pulled, it comes away.
/// The least a workbench widget is made (DIPs): smaller than all its lanes fit in, they take shapes of their own (see
/// `view::Shape`) down to this.
const SHAPE_LEAST: (f32, f32) = (40.0, 40.0);

const STICK: f32 = 36.0;
const UNSTICK: f32 = 52.0;
/// A frame that takes longer than this is noted in the journal, its parts'
/// times with it.
const SLOW_FRAME: Duration = Duration::from_millis(12);
/// How often a widget at rest is drawn again (its charts step with the
/// samples).
const REST_FRAME: Duration = Duration::from_millis(250);
/// How soon one with the pointer over it is drawn again at rest.
const HOVERED_FRAME: Duration = Duration::from_millis(33);
/// Without live refraction, how long after a frame of what is behind is taken another is looked for (see
/// `collect`).
pub const LOOK_AGAIN: Duration = Duration::from_millis(250);
/// How long a frame of what is behind is awaited before its scene is arranged anew (see `collect`).
const ASK_AGAIN: Duration = Duration::from_secs(1);
/// How long the glass takes to frost fully from clear, or to clear (see `draw`).
const FROST_EASE: Duration = Duration::from_millis(400);
/// How often, the time up, a frame being read meanwhile is waited for.
const LOOK_AGAIN_STEP: Duration = Duration::from_millis(16);
/// With live refraction, how often the desktop behind is taken again as it
/// changes, as the panel's is (see `scene::LIVE`).
const LIVE_INTERVAL: Duration = crate::ui::scene::LIVE;
/// Without live refraction, how old the desktop taken behind a widget is
/// before letting go of it takes it again.
const RETAKE_AFTER: Duration = Duration::from_secs(3);
/// Without live refraction, how long the windows on the desktop are left
/// to settle after one changed (moved, came to the front, opened, closed)
/// before the desktop is taken again; and how often at most it is. (With it,
/// taken again as they change: see `WindowWatch`.)
const SETTLED: Duration = Duration::from_millis(300);
const RETAKE_EVERY: Duration = Duration::from_secs(1);

/// When another program's window last changed on the desktop (see
/// `WindowWatch`), as milliseconds since `EPOCH`; 0 for never.
static WINDOWS_CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Where the widgets are on the screens (physical px): a window changing
/// elsewhere changes nothing behind them.
static WIDGET_AREAS: std::sync::Mutex<Vec<RECT>> = std::sync::Mutex::new(Vec::new());
/// The order of the windows on the screens changed: the desktop's own may
/// have moved among the others (Win+D brings it in front of them, after the
/// front has changed), and the widgets kept on it are put back just above it.
static ORDER_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether `r` meets a widget (physical px).
fn meets_a_widget(r: RECT) -> bool {
    WIDGET_AREAS.lock().unwrap().iter().any(|a| r.left < a.right && a.left < r.right && r.top < a.bottom && a.top < r.bottom)
}

/// What is behind the widgets changed, now.
fn behind_changed() {
    if let Some(epoch) = EPOCH.get() {
        WINDOWS_CHANGED.store(epoch.elapsed().as_millis() as u64 + 1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One of Glance's own windows (the panel) came or went at `rect`: the
/// system does not say so (see `WindowWatch`), and a widget that took the
/// desktop with it there shows it still.
pub fn own_window_changed(rect: RECT) {
    if meets_a_widget(rect) {
        behind_changed();
    }
}
static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// How often the other programs' windows are looked at, while a widget is shown: with live refraction, as often as what
/// is behind is taken again (`LIVE_INTERVAL`), a window moved behind one followed as it goes.
const WINDOWS_LOOKED_AT: Duration = Duration::from_millis(250);
/// Whether the widgets are drawn with live refraction (see `Widgets::live`): their windows looked at as often, and
/// their thread woken as one changes behind them (see `WindowWatch`).
static WATCHED_LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The other programs' windows, looked at on a thread of their own every `WINDOWS_LOOKED_AT` for as long as this is
/// kept: one shown that meets a widget moved, sized, come or gone, or put in front of another there (each widget's
/// backdrop may have changed), and the window just above the desktop's own changed (the desktop put in front of the
/// others, Win+D, say: the widgets kept on it follow). Looked at, not heard of: the system tells of such changes only
/// with the rest of its window events, each one from every program a call into this process that takes the system's
/// lock for windows; a program that raises them by the thousand (Task Manager's list, filled), or moves a window
/// hundreds of times a second, makes every program that hears them hold that lock up, and the pointer with it.
struct WindowWatch {
    /// Dropped, it ends the looking.
    _ends: std::sync::mpsc::Sender<()>,
}

/// What the widgets care about of the other programs' windows, at one look.
#[derive(PartialEq, Default)]
struct WindowsSeen {
    /// Those shown (not minimized, nor kept off the screen) that meet a widget, from the front, with where they are.
    meeting: Vec<(isize, RECT)>,
    /// The window just above the desktop's own.
    above_desktop: isize,
}

impl WindowsSeen {
    fn look() -> WindowsSeen {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GW_HWNDPREV};
        unsafe extern "system" fn each(window: HWND, found: LPARAM) -> windows::core::BOOL {
            unsafe { (*(found.0 as *mut Vec<HWND>)).push(window) };
            true.into()
        }
        let mut windows: Vec<HWND> = Vec::new();
        let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut windows as *mut _ as isize)) };
        let own = std::process::id();
        let meeting = windows
            .into_iter()
            .filter(|&window| {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
                let mut cloaked = 0u32;
                let cloaked = unsafe { DwmGetWindowAttribute(window, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, size_of::<u32>() as u32) }.is_ok() && cloaked != 0;
                pid != own && unsafe { IsWindowVisible(window) }.as_bool() && !unsafe { IsIconic(window) }.as_bool() && !cloaked
            })
            .filter_map(|window| {
                let mut r = RECT::default();
                unsafe { GetWindowRect(window, &mut r) }.ok()?;
                meets_a_widget(r).then_some((window.0 as isize, r))
            })
            .collect();
        let above_desktop = crate::ui::window::desktop_window().and_then(|desktop| unsafe { GetWindow(desktop, GW_HWNDPREV) }.ok()).map_or(0, |above| above.0 as isize);
        WindowsSeen { meeting, above_desktop }
    }
}

impl WindowWatch {
    fn start() -> WindowWatch {
        EPOCH.get_or_init(Instant::now);
        let (ends, ended) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new().name("glance: windows watched".into()).spawn(move || {
            let mut seen = WindowsSeen::look();
            let every = || if WATCHED_LIVE.load(std::sync::atomic::Ordering::Relaxed) { LIVE_INTERVAL } else { WINDOWS_LOOKED_AT };
            while ended.recv_timeout(every()) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                let now = WindowsSeen::look();
                if now.meeting != seen.meeting {
                    behind_changed();
                    // (Live, the widgets' thread woken for it: what is behind them taken again at once.)
                    if WATCHED_LIVE.load(std::sync::atomic::Ordering::Relaxed) {
                        crate::app().controller.wake();
                    }
                }
                // (The widgets' thread woken for it: they follow at once.)
                if now.above_desktop != seen.above_desktop && !ORDER_CHANGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    crate::app().controller.wake();
                }
                seen = now;
            }
        }).expect("a thread started");
        WindowWatch { _ends: ends }
    }
}

/// When another program's window last changed, if one has.
fn windows_changed() -> Option<Instant> {
    let ms = WINDOWS_CHANGED.load(std::sync::atomic::Ordering::Relaxed);
    let epoch = EPOCH.get()?;
    (ms > 0).then(|| *epoch + Duration::from_millis(ms - 1))
}
/// How long one layout takes to turn into the next: as long as the frame
/// takes to settle, on the same ease.
pub(crate) const MORPH: Duration = SETTLE;
/// The buttons shown above a widget the pointer is on (under it, where its
/// screen has no room above), clear of what it shows: their size, and how
/// far from it (DIPs).
const BUTTON: f32 = buttons::SIZE;
const BUTTON_INSET: f32 = 6.0;
/// How often a widget that lets clicks through looks for the pointer while
/// it is over it (it hears nothing of it): for its one button to show, and
/// to take the click on it.
const WATCHED: Duration = Duration::from_millis(40);
/// How near the edge of its screen a widget moved shows where it would
/// stick (DIPs).
const HINT_SEEN: f32 = 64.0;

/// The side of its screen a widget is stuck to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn kept(self) -> WidgetSide {
        match self {
            Side::Left => WidgetSide::Left,
            Side::Right => WidgetSide::Right,
            Side::Top => WidgetSide::Top,
            Side::Bottom => WidgetSide::Bottom,
        }
    }

    fn from_kept(side: WidgetSide) -> Self {
        match side {
            WidgetSide::Left => Side::Left,
            WidgetSide::Right => Side::Right,
            WidgetSide::Top => Side::Top,
            WidgetSide::Bottom => Side::Bottom,
        }
    }
}

/// A widget in hand: moved, from where it was taken (the pointer's offset
/// from its corner, physical px), the pointer's last places (for a fling)
/// and whether it was just torn off and has not left the edge yet (put
/// down there, it is neither flung nor stuck back to it); or
/// sized by `edges` (left, right, top, bottom), from the pointer and its
/// room as the drag began.
enum Drag {
    Move { grab: POINT, trail: Vec<(Instant, POINT)>, torn: bool, moved: bool },
    Size { edges: (bool, bool, bool, bool), from: POINT, room: (f32, f32), corner: POINT },
    /// A track slid along (the sound's, or that of what plays): which, where it begins and how wide it is (from the
    /// widget's corner, its own DIPs).
    /// A track pressed: where it begins and how long it is (DIPs, its own; a track standing up begins at its bottom).
    Slide { slider: crate::playing::Slider, from: f32, wide: f32, upright: bool },
    /// Pages swiped across (see `view::Hit::Pages`): which list, how far it turns, a page's width (its own DIPs),
    /// where the hand and the list were as it began, and the hand's last places (for a flick).
    Swipe { list: view::List, most: f32, wide: f32, from: POINT, start: f32, trail: Vec<(Instant, POINT)> },
    /// A list's bar's thumb dragged along it (see `view::Thumb`).
    Scroll(view::Thumb),
}

/// What the widgets are drawn with, as the panel has them now.
pub struct Look<'a> {
    pub prefs: &'a Prefs,
    pub skin: Skin,
    pub lang: Lang,
    /// Live refraction: the desktop behind taken again every so often (see
    /// `LIVE_INTERVAL`).
    pub live: bool,
}

/// What a widget showed: its form at its zoom, its size (its own DIPs), its
/// lanes laid out (or the small layout it showed instead) and how much of
/// each.
#[derive(Clone)]
struct Shown {
    form: Form,
    zoom: f32,
    size: (f32, f32),
    layout: view::Layout,
    small: Option<Small>,
    detail: view::Detail,
}

impl Shown {
    /// Its lanes as they are drawn `size` large (DIPs): as they were laid out; a note pulled off the panel, turning
    /// between its card and a note (`turning`), laid out again for that size, its page growing with its card (laid out
    /// as the note it turns into, its page was as large as that note from the first, its head and foot reaching past
    /// the card).
    fn drawn_at(&self, size: (f32, f32), turning: bool) -> std::borrow::Cow<'_, view::Layout> {
        if turning { std::borrow::Cow::Owned(self.layout.clone().fitted(size)) } else { std::borrow::Cow::Borrowed(&self.layout) }
    }
}

/// What a layout turns from: what was shown, not yet recorded, or what was
/// drawn, recorded.
enum Before {
    Shown(Shown),
    Drawn(Vec<Part>),
}

/// Where a widget moved would stick, let go now: a window of its own,
/// over the widget, shown while it is near an edge.
struct Hint {
    window: Window,
    /// Its surface, on the device its widget drew with as it was made (made anew with the widget's next).
    surface: Option<Surface>,
    gfx: Rc<Gfx>,
    /// Where and how it was last drawn: drawn again only when that changes.
    drawn: Option<(RECT, bool, u32, Option<Ground>)>,
}

/// A widget's own colour, its hint drawn in it (see `paint_hint`): a ground of its own (what plays, its colour and
/// corners), or a colour of its own its skin's hint is drawn in (a note's).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Ground {
    Own([f32; 3]),
    Tint(Color),
}

/// A note pulled off the panel (see `Widgets::stick_note_held`): its file's name, its card's box on the screen and
/// where the hand pressed it (physical px), where the panel is (let go over it, it goes back), and how many physical
/// px a DIP is on the panel.
pub struct NotePulled {
    pub name: String,
    pub card: RECT,
    pub from: POINT,
    pub over: RECT,
    pub px: f32,
}

/// A note pulled off the panel, carried (in hand, put down and settling, or going back to the panel): its number, where
/// the panel is, its card's box there (physical px), where it is held (across, a share of its width; down, how far
/// from its top it was on its card, DIPs), between its card and a note (see `CARD_TO_NOTE`), and going back to the
/// panel, its corner on its way there (physical px). Over the panel it is its card (let go there, it goes back: see
/// `release`); off it, a note, its title drawn up under the pointer (see `grab_at`), and back over it, its card again.
/// Gone back all the way, it has landed: its card drawn on the panel again before it goes (see `land`). Its card is
/// drawn at the panel's px a DIP (`card_px`), as the panel draws it: drawn at the widget's, its words were larger
/// than on the panel (a panel made smaller), and changed as it landed.
struct Pulled {
    number: u64,
    over: RECT,
    card: RECT,
    card_px: f32,
    grab: (f32, f32),
    morph: Spring,
    back: Option<(Spring, Spring)>,
    landed: bool,
}

/// How a note pulled off the panel turns between its card and a note, and how it goes back to the panel (stiffness,
/// damping: see `Spring`; the first a little past, as it comes, about a quarter of a second).
const CARD_TO_NOTE: (f32, f32) = (260.0, 26.0);
pub(crate) const GOING_BACK: (f32, f32) = (220.0, 28.0);
/// How a note is lifted in hand and put down, and how it leans as it is moved across (degrees, the most; and per
/// DIP a millisecond across), pulled off the panel or moved on the desktop.
pub(crate) const LIFTING: (f32, f32) = (500.0, 40.0);
pub(crate) const LEANING: (f32, f32) = (300.0, 22.0);
pub(crate) const LEAN_PULLED: (f32, f32) = (4.0, 6.0);
const LEAN_MOVED: (f32, f32) = (2.0, 3.0);
/// How much larger a note lifted in hand is drawn.
pub(crate) const LIFTED_BY: f32 = 0.04;
/// How fast a note asked for while it is on the desktop already is lifted for a moment (see `Widgets::raise_note`).
const NUDGE: f32 = 36.0;

impl Pulled {
    /// Whether it is carried off the panel (its room there outlined, its card not drawn): in hand (`in_hand`), or going
    /// back, till it lands.
    fn carried(&self, in_hand: bool) -> bool {
        !self.landed && (self.back.is_some() || in_hand)
    }

    /// Gone back all the way: whether it may go now. First it lands, carried no more, and stays a frame of the panel's
    /// more, that panel drawing its card again as the panels are drawn next (before the widgets): gone at once, the room
    /// it left showed empty till then, and it flashed.
    fn land(&mut self) -> bool {
        std::mem::replace(&mut self.landed, true)
    }

    /// Where it is going, the pointer at `cursor`: its card (0) over the panel, a note (1) off it.
    fn wants(&self, cursor: Option<POINT>) -> f32 {
        if cursor.is_some_and(|cursor| crate::panel::contains(&self.over, cursor)) { 0.0 } else { 1.0 }
    }

    /// Where it is held (DIPs from its corner), `size` large: across as far as it was on its card, by share; down, as
    /// far as it was on its card, drawn up into a note's head as it turns into one (no further down than the middle of
    /// it), that the note grows down from under the pointer.
    fn grab_at(&self, size: (f32, f32)) -> (f32, f32) {
        let m = self.morph.value.clamp(0.0, 1.0);
        let down = self.grab.1.min(crate::ui::note::HEAD / 2.0);
        (self.grab.0 * size.0, self.grab.1 + (down - self.grab.1) * m)
    }
}

/// How long back the hand's moves tell how fast a note in hand goes across (see `Widget::carry`).
pub(crate) const LEAN_SPAN: Duration = Duration::from_millis(60);

/// The shadow under a note lifted (see `Widget::lift`): how blurred (a deviation, DIPs), how far it drops, how dark.
pub(crate) struct Lifted {
    pub blur: f32,
    pub drop: f32,
    pub dark: f32,
}

impl Lifted {
    /// How far past the note it reaches (DIPs).
    fn reach(&self) -> f32 {
        3.0 * self.blur + self.drop
    }
}

/// The shadow under a note lifted `lift` of the way: deeper and further down the higher it is.
pub(crate) fn lifted_shadow(lift: f32) -> Lifted {
    let lift = lift.clamp(0.0, 1.2);
    Lifted { blur: 4.0 + 10.0 * lift, drop: 2.0 + 8.0 * lift, dark: 0.28 * lift.min(1.0) }
}

/// What `draw` draws placed by `local`, the frame placed as it was after (what is drawn next is placed from there:
/// the readings' layers place themselves by the frame's place, and `local` again, see `render::PanelLayers::draw_on`).
fn drawn_in(frame: &gfx::Frame, local: Matrix3x2, draw: impl FnOnce()) {
    let mut was = Matrix3x2::default();
    unsafe { frame.dc.GetTransform(&mut was) };
    unsafe { frame.dc.SetTransform(&(local * was)) };
    draw();
    unsafe { frame.dc.SetTransform(&was) };
}

/// What `draw` draws, seen through as much as `opacity` says (1: as it is).
pub(crate) fn faded(frame: &gfx::Frame, opacity: f32, draw: impl FnOnce()) {
    if opacity >= 1.0 {
        draw();
        return;
    }
    let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
    let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
    unsafe { frame.dc.PushLayer(&layer, None) };
    draw();
    unsafe { frame.dc.PopLayer() };
}

/// How far a surface `size` large (DIPs) reaches past its own box, drawn `scale` as large about `about` and turned by
/// `lean` degrees there, its shadow `shadow` further: the room round it its window needs.
fn carried_reach(size: (f32, f32), about: (f32, f32), scale: f32, lean: f32, shadow: f32) -> f32 {
    let (sin, cos) = lean.to_radians().sin_cos();
    let corners = [(0.0, 0.0), (size.0, 0.0), (0.0, size.1), (size.0, size.1)];
    let placed = corners.map(|(x, y)| {
        let (dx, dy) = ((x - about.0) * scale, (y - about.1) * scale);
        (about.0 + dx * cos - dy * sin, about.1 + dx * sin + dy * cos)
    });
    let past = placed.iter().fold(0.0f32, |most, &(x, y)| most.max(-x).max(x - size.0).max(-y).max(y - size.1));
    past.max(0.0) + shadow
}

/// The desktop the panel had taken, lent to a widget torn off it: its
/// capture (without its pixels), as a bitmap at the scale it was made for,
/// and its tone.
pub struct Lent {
    pub capture: Capture,
    pub bitmap: (ID2D1Bitmap1, f32),
    pub tone: (f32, f32),
}

/// The desktop behind a widget, for the skins that see it.
struct Behind {
    capture: Capture,
    /// The capture as a bitmap, at the scale of the screen (physical px a
    /// DIP) it was made for.
    bitmap: Option<(ID2D1Bitmap1, f32)>,
    /// The bitmap the capture before was drawn into: its memory written
    /// again for this one (see `Capture::bitmap_into`).
    former: Option<ID2D1Bitmap1>,
    tone: (f32, f32),
    taken: Instant,
}

impl Behind {
    /// The capture as drawn (made once, at its screen's scale `scale`, its pixels then let go of: a screen's worth of
    /// memory), placed so that the widget's corner, `margin` in from its window's (`window`, `px` physical pixels a
    /// DIP), is where it is on screen; as `render::Picture::backdrop` takes it.
    fn backdrop(&mut self, dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, scale: f32, window: RECT, px: f32, margin: f32) -> Option<(&ID2D1Bitmap1, Vector2, u64, f32)> {
        if self.bitmap.is_none() {
            if let Ok(bitmap) = self.capture.bitmap_into(dc, scale, self.former.take().as_ref()) {
                self.bitmap = Some((bitmap, scale));
                self.capture.forget_pixels();
            }
        }
        let from = self.capture.rect;
        let at = Vector2 { X: (window.left - from.left) as f32 / px + margin, Y: (window.top - from.top) as f32 / px + margin };
        Some((&self.bitmap.as_ref()?.0, at, self.capture.digest, px))
    }
}

impl crate::ui::life::Lives for Widget {
    fn device(&self) -> &Rc<Gfx> {
        &self.gfx
    }

    fn showing(&mut self) -> &mut Option<crate::ui::life::Showing> {
        &mut self.showing
    }

    /// Its surface, what its content kept (see `Content::release`), where it would stick, the desktop behind it; its
    /// scene rests.
    fn let_go_of_shown(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.release(&self.gfx);
        }
        self.content.release();
        self.hide_hint();
        self.behind = None;
        self.pending = None;
        if let Some(scene) = &mut self.backdrop {
            scene.rest();
        }
    }

    /// Its scene of what is behind it, and its surface.
    fn let_go_of_kept(&mut self) {
        self.backdrop = None;
        self.surface = None;
    }
}

impl Widget {
    /// What it keeps of the desktop behind it let go of (its skin does not see it): its scene, what was taken, what
    /// is awaited (made again as a skin that sees it is chosen, as for one that never saw it).
    fn unsee(&mut self) {
        self.backdrop = None;
        self.behind = None;
        self.pending = None;
        self.look_again = None;
    }
}

struct Widget {
    window: Window,
    gfx: Rc<Gfx>,
    /// Counted as shown on its device while it is (see `life`).
    showing: Option<crate::ui::life::Showing>,
    surface: Option<Surface>,
    /// What it shows.
    content: Content,
    /// The screen it is on.
    contact: Contact,
    /// Its surface's corner on the screen (physical px).
    at: POINT,
    /// Shown as where a widget in hand would be merged (beside it, else below), while it would.
    drop_target: Option<bool>,
    /// Merged, pressed on a part after the first: that part (torn out as it is moved), and where it was taken from
    /// its corner (DIPs); torn out, to be made a widget of its own (see `Widgets::tear_part`).
    splitting: Option<(usize, (f32, f32))>,
    split: Option<(usize, (f32, f32))>,
    /// While it is sized: the room its edges were dragged to (DIPs).
    room: Option<(f32, f32)>,
    /// Let go after it was sized or stuck: since when, and from what
    /// surface (DIPs), it settles onto what it shows.
    settle: Option<(Instant, (f32, f32))>,
    /// A note pulled off the panel, carried (see `Pulled`).
    pulled: Option<Pulled>,
    /// A note's: how far it is lifted (0 at rest, 1 in hand) and how it leans (degrees), and where (DIPs from its corner:
    /// where it is held, else its middle).
    lift: Spring,
    lean: Spring,
    about: (f32, f32),
    /// The sides of its surface what it shows keeps to (right, bottom).
    keeps_to: (bool, bool),
    drag: Option<Drag>,
    /// Flung: its speed (DIPs a millisecond), and how far it shows.
    fling: Option<(f32, f32)>,
    opacity: f32,
    stuck: Option<Side>,
    /// Where along it its middle is (physical px; none: where it was,
    /// restored from the settings): where the middle of what was stuck was.
    along: Option<i32>,
    /// The side across it it is stuck to as well: it is stuck in their corner.
    also: Option<Side>,
    /// Its skin's room round it for its shadow (DIPs, its own).
    margin: f32,
    /// Settling after it was sized by its left or top edge: where its right
    /// and bottom edges stay (physical px).
    anchor: Option<POINT>,
    pinned: bool,
    click_through: bool,
    /// Torn off the panel and not clear of where it came from yet: the
    /// edge the panel was at (none: moved away from it), and its screen.
    torn_from: Option<(Option<Side>, RECT)>,
    game_only: bool,
    /// Kept on the desktop, under every other window (one shown only with
    /// a game is above them all still).
    on_desktop: bool,
    /// Sized by hand in a frame since it was last drawn at rest (see `Gfx::settle`).
    was_sized: bool,
    hint: Option<Hint>,
    /// What is behind it, put together live where no one sees it (see
    /// `scene`): while it sees the desktop and shows.
    backdrop: Option<crate::ui::scene::Scene>,
    /// When the windows behind it were last looked at for it, and the screen it was on then.
    arranged: Option<Instant>,
    arranged_on: Option<RECT>,
    /// Looking at what is behind it again is due (let go, the settings
    /// gone from the front).
    retake: bool,
    /// What is behind it arranged anew, its frame awaited.
    pending: Option<Request>,
    /// Without live refraction, a frame taken: till when another is looked for before the scene rests (see
    /// `collect`).
    look_again: Option<Instant>,
    /// How many times what is behind it has changed as a frame of it was taken (see `look_behind`).
    behind_changes: u64,
    /// What is behind it is to be replaced at once, even in hand: the
    /// panel's, lent as it was torn off (`false`: its tone kept), or taken
    /// of its screen as it was (`true`: toned afresh).
    replace: Option<bool>,
    /// Counts the changes of screen: a capture asked for before the last
    /// is of the screen as it was, and let go of.
    generation: u64,
    /// Come to rest where it was not when it was saved (flung, or settling
    /// after it was sized): saved again.
    unsaved: bool,
    /// When the last capture asked for came back with nothing.
    failed: Option<Instant>,
    /// Drawn and shown on the screen at least once.
    shown_once: bool,
    /// How frosted it was, measured where it was (see `draw`).
    frost_at: Option<(RECT, f32)>,
    /// How frosted it is drawn, easing toward how frosted it should be (the second), and when it was last drawn so.
    frost_shown: Option<(f32, f32, Instant)>,
    /// Light or dark is to be measured again where it comes to rest (see
    /// `retone`).
    tone_due: bool,
    /// How long the last frame's painting took, and how long handing it to the compositor then did.
    frame_took: gfx::Took,
    /// The pointer is over it (its buttons show).
    hover: bool,
    /// What a press would do something with under the pointer (see `view::Hit::takes_press`): lit as it is drawn.
    hit_hover: Option<view::Hit>,
    /// The preview's shortcut said on what the pointer rests on (see `view::Hint`).
    preview_hint: view::Hint,
    /// What is under the pointer said under it (see `hover`): taken away as the pointer leaves it, the widget goes, or
    /// what is under the pointer changes without it (pages turned).
    saying: bool,
    /// Its sliders' knobs' dots, easing (see `thumb::Dots`).
    thumbs: crate::ui::thumb::Dots<view::Hit>,
    /// Letting clicks through: since when the pointer has been over it, and
    /// whether it is on the button that ends that (the window then takes
    /// the click).
    lingered: Option<Instant>,
    /// The pointer over its shadow, its window letting clicks there through (see `clicks_through`).
    shadowed: bool,
    /// Where its settings' button was when it was last seen to be the place the settings, open, were asked from (see
    /// `follow_settings`).
    settings_from: Option<RECT>,
    reached: bool,
    /// The button the pointer is on, said under it.
    tip: Option<Button>,
    /// What of what it shows takes a press (what plays, the sound), where each
    /// is from the corner of what it shows (its own DIPs), as last drawn.
    hits: Vec<view::HitBox>,
    hits_from: (f32, f32),
    /// A thing on its station pressed (see `station::select::Press`): dragged off it once moved far enough, else
    /// pressed as it is let go (see `Widgets::hover`, `Widgets::release`).
    shelf_press: Option<crate::station::select::Press>,
    /// A thing on its station dragged off it, to be lifted off its room as it is next drawn (see `carry`): it, all that
    /// goes with it (it first: what is chosen with it), the drag's number, and where it was pressed.
    lifting: Option<(view::Hit, Vec<view::Hit>, u64, POINT)>,
    /// The pages under a press (the list turned by them, how far it turns, a page's width) and when it was pressed:
    /// swiped across, the hand going across them first (see `begin_swipe`).
    swipes: Option<(view::List, f32, f32, Instant)>,
    /// The skin it was last drawn in, and whether dark (what opens beside it is drawn alike).
    drawn_look: (Skin, bool),
    behind: Option<Behind>,
    /// When it is next drawn: at once while it moves, else after a rest (as
    /// its charts step), or sooner as the pointer comes or it is pressed.
    next_frame: Instant,
    /// Its surface, and what it shows, as last laid out (DIPs, its own at
    /// its zoom), and the zoom.
    surface_size: (f32, f32),
    shown: (f32, f32),
    zoom: f32,
    last_frame: Instant,
    /// A note stuck on the desktop on it written in where it is (see `InPlace`).
    in_place: Option<InPlace>,
}

/// How often a note written in where it is looks at its file for a change made outside.
const READ_OUTSIDE: Duration = Duration::from_secs(1);

/// A note stuck on the desktop written in where it is, on its widget (see `ui::keys`, `ui::note::Writing`): its words
/// in a box the text services write, placed where its card's are and drawn in the card's place (see
/// `view::written_in_place`), the widget taking the keys. Given back (Esc, another program's window in front), what
/// was written is kept, and it is drawn so till the notes as read show it as written (drawn from them before, it would
/// show for a moment as it was), then let go.
struct InPlace {
    writing: crate::ui::note::Writing,
    /// The note's number (as a press on it names it: see `view::Hit::Words`), and its name, as its widget shows it.
    number: u64,
    name: String,
    hwnd: HWND,
    lang: Lang,
    /// Taking the keys.
    taking: bool,
    /// The window in front as it began to take them (Esc gives the front back to it).
    before: HWND,
    clicks: crate::ui::keys::Clicks,
    /// A press on its words followed by them till it is let go, and a press on its menu's button.
    selecting: bool,
    menu_pressed: bool,
    /// When what was written is next kept, and its file next looked at (see `READ_OUTSIDE`).
    keep_at: Option<Instant>,
    read_at: Instant,
    /// Since when the caret blinks (it shows at once as it moves, or as a key comes), and what was chosen then.
    blink_from: Instant,
    chosen: (i32, i32),
    /// What its words were last styled for: pixels a DIP, the skin, and whether dark.
    styled: (f32, Skin, bool),
    /// Its card, where it was last drawn (DIPs from its window's corner).
    card: (f32, f32, f32, f32),
}

/// Let go of with its widget (put away, merged, the note taken off the desktop): what was written kept, or where it
/// cannot be, put aside (see `Writing::put_aside`), its card drawn as the notes read show it again.
impl Drop for InPlace {
    fn drop(&mut self) {
        crate::ui::keys::give_back(self.hwnd);
        if !self.writing.keep(self.lang) {
            self.writing.put_aside();
        }
        view::written_in_place(&self.name, false);
    }
}

impl InPlace {
    /// What was written kept on a thread of its own (see `Writing::keep_soon`): looked at again shortly while under way,
    /// tried again a while after it failed.
    fn keep_soon(&mut self, now: Instant) {
        let after = |ms: u32| Some(now + Duration::from_millis(u64::from(ms)));
        self.keep_at = match self.writing.keep_soon(self.lang) {
            crate::ui::note::Keeping::Kept => None,
            crate::ui::note::Keeping::Under => after(crate::ui::note::KEEP_LOOK_MS),
            crate::ui::note::Keeping::Failed => after(crate::ui::note::KEEP_AGAIN_MS),
        };
    }

    /// Its menu's button, in its card's head (DIPs from its window's corner).
    fn menu_button(&self) -> (f32, f32, f32, f32) {
        crate::ui::note::button_at(0, (self.card.0, self.card.1, self.card.2))
    }

    /// Whether `client` (physical px in its window, `px` pixels a DIP) is on its menu's button, shown while it takes
    /// the keys.
    fn on_menu_button(&self, client: POINT, px: f32) -> bool {
        let (x, y) = (client.x as f32 / px, client.y as f32 / px);
        let (bx, by, bw, bh) = self.menu_button();
        self.taking && x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    /// Whether its caret shows now, in its blink (Windows' time for it).
    fn caret_on(&self, now: Instant) -> bool {
        let blink = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime() };
        // (No blink asked for: always shown.)
        blink == u32::MAX || blink == 0 || (now.saturating_duration_since(self.blink_from).as_millis() / u128::from(blink)).is_multiple_of(2)
    }

    /// When it is next due: its caret's blink turning, what was written kept, its file looked at.
    fn due(&self, now: Instant) -> Option<Instant> {
        let blink = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime() };
        let turns = (self.taking && blink != u32::MAX && blink != 0).then(|| {
            let blink = u128::from(blink);
            let past = now.saturating_duration_since(self.blink_from).as_millis();
            self.blink_from + Duration::from_millis(((past / blink + 1) * blink) as u64)
        });
        [turns, self.keep_at, self.taking.then_some(self.read_at)].into_iter().flatten().min()
    }
}

/// The card of the note `number` stuck on the desktop among `hits` (as drawn from `from`, its widget's content's corner,
/// in its window `margin` from its corner: DIPs), where its words are pressed to be written in (see `view::Hit::Words`):
/// DIPs from the window's corner.
fn card_in(hits: &[view::HitBox], from: (f32, f32), number: u64, margin: f32) -> Option<(f32, f32, f32, f32)> {
    hits.iter().any(|hit| hit.4 == view::Hit::Words(number)).then_some(())?;
    hits.iter().find(|hit| hit.4 == view::Hit::Note(number)).map(|hit| (margin + from.0 + hit.0, margin + from.1 + hit.1, hit.2, hit.3))
}

/// Where a note's words are written on its card `card` (DIPs from the window's corner, `px` pixels a DIP): in its
/// window (physical px).
fn words_rect(card: (f32, f32, f32, f32), px: f32) -> RECT {
    let at = |dips: f32| (dips * px).round() as i32;
    crate::ui::note::words_box(at(card.0), at(card.1), at(card.2), at(card.3), px)
}

/// Whether the window in front is one of this thread's (a widget's, a menu up), or none for now.
fn front_is_ours() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let front = unsafe { GetForegroundWindow() };
    front.is_invalid() || unsafe { GetWindowThreadProcessId(front, None) } == unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

/// What a widget shows.
enum Content {
    /// The panel's readings, as much of them as fits (see `arrange::Ladder`).
    Readings(Readings),
    /// What plays (see `music`).
    Music(MusicShown),
    /// Widgets merged into one (see `Merged`).
    Merged(Box<Merged>),
}

/// Widgets merged into one (see `Widgets::merge`): the one dropped onto first, whose look the whole one has (its
/// ground under all of it, its type and its ink). Laid out as one, by the whole one's size (see `lay_out`): what plays
/// merged into readings is one of their lanes, laid out among them as theirs are (or, where they show a small layout,
/// a row of their own under it on the same ground); readings merged into what plays a band of it, along its bottom or
/// its side (`beside`).
struct Merged {
    /// Readings or what plays, never merged ones.
    parts: Vec<Content>,
    beside: bool,
    /// Its size (DIPs, as large as it is on the screen), where each part is drawn (from its corner, as last laid
    /// out), where each is taken to be torn out, and the least it can be.
    size: (f32, f32),
    rects: Vec<(f32, f32, f32, f32)>,
    touch: Vec<(f32, f32, f32, f32)>,
    least: (f32, f32),
    /// Merged or torn apart just now: since when, and where each part was (from its corner; none, it comes in where it
    /// is), its parts moving from there to where they go over `MORPH`.
    moving: Option<(Instant, Vec<Option<(f32, f32, f32, f32)>>)>,
    /// Where its parts were as the journal was last told.
    told: Vec<(f32, f32, f32, f32)>,
    /// What plays is in one of the readings' lanes as last laid out (else in a row of its own beside or under them).
    in_lane: bool,
    /// Laid out once since it was merged: its size then made at least what its parts need at their own size (a part
    /// merged in brings what it shows; what was there is not made smaller for it).
    grown: bool,
}

/// The readings laid out for `room` (DIPs on the screen) as the panel's are, in a work area `work` large, with the
/// music lane or not; none where nothing could be.
fn readings_for(scene: &Scene, theme: &Theme, work: (f32, f32), seen: &Seen, room: (f32, f32), with_music: bool) -> Option<arrange::Opening> {
    readings_as(scene, theme, work, seen, Want::Room(room, None), with_music)
}

/// The readings laid out as `want` says (see `readings_for`).
fn readings_as(scene: &Scene, theme: &Theme, work: (f32, f32), seen: &Seen, want: Want, with_music: bool) -> Option<arrange::Opening> {
    let heights = view::heights(scene);
    let lanes: Vec<(&str, arrange::Heights)> = heights.iter().filter(|(id, _)| with_music || id != "lyrics").map(|(id, h)| (id.as_str(), *h)).collect();
    let opening = arrange::Opening::new(theme, Edge::Right, &lanes, forms::count(scene), work, None, 1.0, want, seen.clone());
    opening.choice.is_some().then_some(opening)
}

/// How tall the row of what plays under readings in a small layout is (DIPs), and how wide or tall a band of readings
/// in what plays is at least, of its size.
const MUSIC_ROW: f32 = 64.0;
/// How tall a merged widget is, at most, to be one row (the readings' strip and what plays after it).
const ONE_ROW: f32 = 100.0;
const READINGS_BAND: f32 = 0.3;

impl Merged {
    fn new(parts: Vec<Content>, beside: bool, size: (f32, f32)) -> Self {
        Merged { parts, beside, size, rects: Vec::new(), touch: Vec::new(), least: (0.0, 0.0), moving: None, told: Vec::new(), grown: false, in_lane: false }
    }

    /// Whether the one dropped onto (whose look it has) is what plays.
    fn host_plays(&self) -> bool {
        matches!(self.parts.first(), Some(Content::Music(_)))
    }

    /// Whether what plays is one of its readings' lanes (drawn apart in the room left in it: see `Scene::music_apart`).
    fn music_apart(&self) -> bool {
        !self.host_plays() && self.parts.iter().any(|part| matches!(part, Content::Music(_)))
    }

    /// The readings in it, laid out as `opening` lays them out (turning into that from what they showed, where it is
    /// another form or has other lanes).
    fn show_readings(&mut self, opening: &arrange::Opening, now: Instant) {
        let Some(Content::Readings(readings)) = self.parts.iter_mut().find(|part| matches!(part, Content::Readings(_))) else { return };
        let choice = opening.choice.unwrap();
        readings.choice = choice;
        let shown = Shown { form: choice.form, zoom: opening.zoom, size: opening.size, layout: opening.layout.clone(), small: opening.small(), detail: opening.detail() };
        if let Some(last) = readings.last.take().filter(|last| last.form != shown.form || last.layout.heights.len() != shown.layout.heights.len()) {
            let before = if readings.morph.is_some() { Before::Drawn(std::mem::take(&mut readings.drawn)) } else { Before::Shown(last) };
            readings.morph = Some((now, before));
        }
        readings.last = Some(shown);
        if readings.morph.as_ref().is_some_and(|(since, _)| now.duration_since(*since) >= MORPH) {
            readings.morph = None;
            readings.drawn.clear();
            readings.morph_layer.release();
        }
    }

    /// Lays its parts out in its size (`scene`'s readings with the music lane, drawn apart, where what plays is merged
    /// into them). Whether it could be.
    fn lay_out(&mut self, scene: &Scene, theme: &Theme, work: (f32, f32), seen: &Seen, now: Instant) -> bool {
        let on_screen = |opening: &arrange::Opening| (opening.size.0 * opening.zoom, opening.size.1 * opening.zoom);
        let music = self.parts.iter().position(|part| matches!(part, Content::Music(_)));
        let readings = self.parts.iter().position(|part| matches!(part, Content::Readings(_)));
        let (Some(music), Some(readings)) = (music, readings) else { return false };
        let mut rects = vec![(0.0, 0.0, 0.0, 0.0); self.parts.len()];
        let mut touch = rects.clone();
        // Just merged: as large as the readings' form, as it was, with what plays as one of their lanes at its own
        // size (or as what plays, with the readings' band at theirs: see below).
        if !std::mem::replace(&mut self.grown, true) && !self.host_plays() {
            let form = self.parts.iter().find_map(|part| if let Content::Readings(readings) = part { Some(readings.choice) } else { None });
            if let Some(opening) = form.and_then(|choice| readings_as(scene, theme, work, seen, Want::Kept(choice), true)).filter(|opening| opening.small().is_none()) {
                let size = on_screen(&opening);
                self.size = (self.size.0.max(size.0), self.size.1.max(size.1));
            }
        }
        let (w, h) = self.size;
        if !self.host_plays() {
            // What plays as one of the readings' lanes: laid out among them, where they show their lanes.
            let least = readings_for(scene, theme, work, seen, (1.0, 1.0), false);
            let lanes = readings_for(scene, theme, work, seen, (w, h), true).filter(|opening| opening.small().is_none());
            let room = lanes.as_ref().and_then(|opening| {
                let z = opening.zoom;
                let room = view::music_room(&opening.layout, opening.lane_ids(), theme)?;
                let lane = opening.lane_ids().iter().position(|id| id == "lyrics").and_then(|index| opening.layout.lanes().get(index).copied())?;
                Some(((room.x * z, room.y * z, room.w * z, room.h * z), (lane.x * z, lane.y * z, lane.w * z, lane.h * z)))
            });
            match (lanes, room) {
                (Some(opening), Some((room, lane))) => {
                    self.in_lane = true;
                    let size = on_screen(&opening);
                    self.show_readings(&opening, now);
                    rects[readings] = (0.0, 0.0, size.0, size.1);
                    rects[music] = room;
                    touch[music] = lane;
                    self.size = size;
                }
                // Short and wide: one row, the readings' strip and what plays after it, as tall.
                _ if { self.in_lane = false; h < ONE_ROW } && readings_for(scene, theme, work, seen, (1.0, h), false).is_some_and(|least| w - on_screen(&least).0 >= music::MERGED_LEAST.0) => {
                    let Some(opening) = readings_for(scene, theme, work, seen, (w - music::MERGED_LEAST.0, h), false) else { return false };
                    let size = on_screen(&opening);
                    self.show_readings(&opening, now);
                    rects[readings] = (0.0, 0.0, size.0, h);
                    rects[music] = (size.0, 0.0, (w - size.0).max(music::MERGED_LEAST.0), h);
                    touch[music] = rects[music];
                    self.size = (size.0 + rects[music].2, h);
                }
                // A small layout: what plays in a row of its own under it, as wide.
                _ => {
                    self.in_lane = false;
                    let Some(opening) = readings_for(scene, theme, work, seen, (w, (h - MUSIC_ROW).max(1.0)), false) else { return false };
                    let size = on_screen(&opening);
                    self.show_readings(&opening, now);
                    rects[readings] = (0.0, 0.0, size.0, size.1);
                    rects[music] = (0.0, size.1, size.0, MUSIC_ROW);
                    touch[music] = rects[music];
                    self.size = (size.0, size.1 + MUSIC_ROW);
                }
            }
            let least = least.map_or((0.0, 0.0), |least| on_screen(&least));
            self.least = (least.0.max(music::MERGED_LEAST.0), least.1 + MUSIC_ROW);
        } else {
            // Readings as a band along what plays' bottom (or its side), as large as they are at their least without
            // being made smaller, larger as what plays grows (`READINGS_BAND` of it).
            let natural = readings_for(scene, theme, work, seen, if self.beside { (1.0, h) } else { (w, 1.0) }, false).map_or((0.0, 0.0), |least| least.size);
            let band = if self.beside { natural.0.max(w * READINGS_BAND) } else { natural.1.max(h * READINGS_BAND) };
            let room = if self.beside { (band, h) } else { (w, band) };
            let Some(opening) = readings_for(scene, theme, work, seen, room, false) else { return false };
            let size = on_screen(&opening);
            self.show_readings(&opening, now);
            if self.beside {
                let rest = (w - size.0).max(music::MERGED_LEAST.0);
                rects[music] = (0.0, 0.0, rest, h.max(size.1));
                rects[readings] = (rest, 0.0, size.0, h.max(size.1));
                self.size = (rest + size.0, h.max(size.1));
                self.least = (music::MERGED_LEAST.0 + natural.0, music::MERGED_LEAST.1.max(natural.1));
            } else {
                let rest = (h - size.1).max(music::MERGED_LEAST.1);
                rects[music] = (0.0, 0.0, w.max(size.0), rest);
                rects[readings] = (0.0, rest, w.max(size.0), size.1);
                self.size = (w.max(size.0), rest + size.1);
                self.least = (music::MERGED_LEAST.0.max(natural.0), music::MERGED_LEAST.1 + natural.1);
            }
            touch = rects.clone();
        }
        self.rects = rects;
        self.touch = touch;
        if self.moving.as_ref().is_some_and(|(since, _)| now.duration_since(*since) >= MORPH) {
            self.moving = None;
        }
        // (For the journal, as the parts' places change.)
        if self.told != self.rects {
            self.told = self.rects.clone();
            let readings = self.parts.iter().find_map(|part| if let Content::Readings(readings) = part { readings.last.as_ref().map(|shown| (shown.form.key(), shown.zoom)) } else { None });
            note(format!("widget: merged laid out in {:.0} x {:.0}: {:?}; readings {readings:?}", self.size.0, self.size.1, self.rects));
        }
        true
    }

    /// Whether it is to be drawn again soon: what plays in it, its readings turning, its parts moving.
    fn busy(&self) -> bool {
        self.moving.is_some() || self.parts.iter().any(|part| matches!(part, Content::Music(shown) if shown.busy) || matches!(part, Content::Readings(readings) if readings.morph.is_some()))
    }

    /// The part at `(x, y)` (DIPs, from its corner) to be taken (the readings, everywhere but what plays), and that
    /// point from the part's corner.
    fn part_at(&self, (x, y): (f32, f32)) -> Option<(usize, (f32, f32))> {
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let index = self.parts.iter().position(|part| matches!(part, Content::Music(_))).filter(|&music| self.touch.get(music).is_some_and(|rect| inside(*rect)));
        let index = index.or_else(|| self.touch.iter().position(|rect| inside(*rect)))?;
        let rect = self.rects[index];
        Some((index, (x - rect.0, y - rect.1)))
    }
}

/// Draws widgets merged (`merged`), from the frame's origin moved by `local`, `px` physical pixels a DIP: the ground
/// of the one dropped onto under all of it (the skin's, or what plays' own), each part in its place on it in that
/// one's type and ink (moving there from where it was, merged or torn apart just now). Where a press does something
/// in each, from its corner; whether all of it was drawn.
///
/// A thing on a station's lane in it lifted off its room (`lifting`: all carried, and those of them whose rooms show,
/// each with where its room is, from its corner, the one pressed first): what each draws recorded into `lifted` from
/// its own room's corner (see `carry`), their rooms left to be drawn empty.
#[allow(clippy::too_many_arguments)]
fn draw_merged(frame: &gfx::Frame, merged: &mut Merged, scene: &Scene, local: Matrix3x2, px: f32, backdrop: Option<(&ID2D1Bitmap1, Vector2, u64, f32)>, frost: f32, rough: bool, pointer: Option<(f32, f32)>, shows: &Shows, now: Instant, lifting: Option<(&[view::Hit], &[render::TileBox])>, lifted: &mut Option<Vec<render::Recorded>>, field: &view::todo::Field) -> (Vec<view::HitBox>, bool) {
    let theme = scene.theme;
    let size = merged.size;
    let host_plays = merged.host_plays();
    // (What plays, the one dropped onto, drawn on the skin: the skin's ground under it all, as under readings dropped
    // onto, what plays in the skin's ink on it.)
    let on_skin = host_plays && shows.ground == MusicGround::Skin;
    let apart = merged.music_apart() && merged.in_lane;
    let rects = merged.rects.clone();
    let moving = merged.moving.as_ref().map(|(since, from)| (morph::ease((now.duration_since(*since).as_secs_f32() / MORPH.as_secs_f32()).min(1.0)), from.clone()));
    let mut hits = Vec::new();
    let mut whole = true;
    let mut backdrop = backdrop;
    // What plays' own ground (or the skin's), under it all.
    if host_plays {
        if let Some(Content::Music(host)) = merged.parts.first_mut() {
            if on_skin {
                frame.origin(0.0, 0.0);
                if let Err(error) = host.skin_ground.draw_whole(frame, theme, size, backdrop.take(), frost, rough, local, px) {
                    note(format!("widget: its ground not drawn: {error}"));
                    whole = false;
                }
            } else {
                frame.place(local);
                host.music.paint_ground(frame, size);
            }
        }
    }
    for (index, part) in merged.parts.iter_mut().enumerate() {
        let Some(&(rx, ry, rw, rh)) = rects.get(index) else { continue };
        // Moving there: from where it was to its place, scaled from that size to its own; come in where it is, faded
        // in.
        let (shift, fade) = match moving.as_ref().map(|(t, from)| (*t, from.get(index).copied().flatten())) {
            Some((t, Some((fx, fy, fw, _)))) => {
                let at = |a: f32, b: f32| a + (b - a) * t;
                // (Scaled alike both ways, its words kept in shape: as wide as it is on its way there.)
                let (x, y, w) = (at(fx, rx), at(fy, ry), at(fw, rw));
                let k = w / rw.max(1.0);
                (Matrix3x2::translation(-rx, -ry) * Matrix3x2::scale(k, k) * Matrix3x2::translation(x, y), 1.0)
            }
            Some((t, None)) => (Matrix3x2::identity(), t),
            None => (Matrix3x2::identity(), 1.0),
        };
        if fade < 1.0 {
            let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
            frame.origin(0.0, 0.0);
            unsafe { frame.dc.PushLayer(&D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: fade, ..Default::default() }, None) };
        }
        let placed = |at: (f32, f32), list: &mut Vec<view::HitBox>, from: Vec<view::HitBox>, k: f32| {
            list.extend(from.into_iter().map(|(hx, hy, hw, hh, hit)| (at.0 + hx * k, at.1 + hy * k, hw * k, hh * k, hit)));
        };
        match part {
            Content::Readings(readings) => {
                let Some(shown) = readings.last.clone() else { continue };
                let z = shown.zoom;
                // (Its music lane only where what plays is drawn in it.)
                let mut lanes: Vec<view::Lane> = view::lanes_at(scene, |_| shown.detail).into_iter().filter(|lane| apart || lane.id() != "lyrics").collect();
                view::todo::write_in(&mut lanes, field);
                // On the skin's ground (one piece, as large as all of it), where it is the one dropped onto; on what
                // plays' ground, none.
                let picture = render::Picture {
                    scene,
                    lanes: &lanes,
                    layout: &shown.layout,
                    small: shown.small.map(|small| (small, shown.size)),
                    size: if host_plays { (rw / z, rh / z) } else { (size.0 / z, size.1 / z) },
                    at: (0.0, 0.0),
                    edge: None,
                    backdrop: if host_plays { None } else { backdrop.take() },
                    frost,
                    rough,
                };
                // (Drawn from its place, its own DIPs scaled to the screen's: its layers and its charts, drawn straight
                // onto the frame each frame, alike.)
                frame.place(Matrix3x2::scale(z, z) * Matrix3x2::translation(rx, ry) * shift * local);
                let halo = (theme.skin == Skin::Glass && (!host_plays || on_skin)).then_some(theme.legibility);
                if let Some((carried, boxes)) = lifting.filter(|_| lifted.is_none()) {
                    *lifted = Some(render::record_tiles(frame, scene, &lanes, &shown.layout, picture.small, z, (rx, ry), boxes)).filter(|tiles| tiles.first().is_some_and(|(first, ..)| first == &boxes[0].0));
                    if lifted.is_some() {
                        view::carry_tiles(carried.to_vec());
                    }
                }
                let keep = |old: &Shown| (old.zoom / z, (0.0, 0.0));
                let (drawn, drawn_whole) = paint_readings(frame, readings, scene, &picture, &keep, Matrix3x2::identity(), px * z, halo, (!host_plays).then_some(&[][..]), now);
                whole &= drawn_whole;
                placed((rx, ry), &mut hits, readings.found(drawn), z);
                // A small layout with what plays in a row of its own under it: a rule between them.
                let below = rects.iter().enumerate().find(|(other, _)| *other != index).map(|(_, rect)| rect.1).filter(|y| *y > 0.0);
                if let (false, false, Some(rule_y)) = (host_plays, apart, below) {
                    frame.place(local);
                    frame.fill(theme.rule, theme.pad_x, rule_y, size.0 - 2.0 * theme.pad_x, 1.0);
                }
            }
            Content::Music(shown) => {
                let MusicShown { music, layer, frames, busy, .. } = shown;
                *frames += 1;
                // In the readings' look: their type, their ink, as far in as their lanes' content (a lane's room is
                // already inside its lane).
                let inset = if apart { 0.0 } else { theme.pad_x };
                // (The one dropped onto: its own look, its ground drawn under it all already.)
                let ground = match index {
                    0 if on_skin => music::Ground::Skin { dark: theme.dark },
                    0 => music::Ground::Under,
                    _ => music::Ground::Host { dark: theme.dark, inset },
                };
                let here = pointer.filter(|(x, y)| *x >= rx && *x < rx + rw && *y >= ry && *y < ry + rh).map(|(x, y)| (x - rx, y - ry));
                let mut painted = None;
                frame.origin(0.0, 0.0);
                let drew = layer.draw(frame, *frames, Matrix3x2::translation(rx, ry) * shift * local, (-4.0, -4.0, rw + 8.0, rh + 8.0), px, None, |frame| {
                    painted = Some(music.paint(frame, (rw, rh), shows, ground, theme.body.family, px, here, now));
                    Ok(())
                });
                if let Err(error) = drew {
                    note(format!("widget: what plays not drawn: {error}"));
                    whole = false;
                }
                if let Some(painted) = painted {
                    *busy = painted.busy;
                    placed((rx, ry), &mut hits, painted.hits, 1.0);
                }
            }
            Content::Merged(_) => {}
        }
        if fade < 1.0 {
            frame.origin(0.0, 0.0);
            unsafe { frame.dc.PopLayer() };
        }
    }
    frame.origin(0.0, 0.0);
    (hits, whole)
}

/// Draws `readings` as `picture` shows them, placed by `local` within the frame's transform, `px` physical pixels a
/// DIP of theirs: turning from what they showed (each reading between where it was and where it goes, what showed
/// placed by `placed`) while they turn, else at rest; on the skin's ground under them (and `others`, as `draw_on`
/// says; none, on no ground of their own). Where a press does something (when they were drawn at rest and anew), and
/// whether all of it was drawn.
#[allow(clippy::too_many_arguments)]
fn paint_readings(frame: &gfx::Frame, readings: &mut Readings, scene: &Scene, picture: &render::Picture, placed: &dyn Fn(&Shown) -> (f32, (f32, f32)), local: Matrix3x2, px: f32, halo: Option<Color>, others: Option<&[view::Rect]>, now: Instant) -> (Option<Vec<view::HitBox>>, bool) {
    let Some(shown) = readings.last.clone() else { return (None, true) };
    let mut whole = true;
    // A thing carried off its station put down and taken off it: turning from how they were drawn as it was carried
    // (its room outlined) into how they are drawn now.
    if let Some(from) = readings.away.follow(|| render::record(frame, scene, picture.lanes, &shown.layout, picture.small, 1.0, picture.at)) {
        if readings.morph.is_none() {
            readings.morph = Some((now, Before::Drawn(from)));
        }
    }
    // What is on its station otherwise than it was drawn (something came from a device nearby, or begins to, or was put
    // on it or taken off): turning from how it was drawn.
    let station_now = crate::station::run::shown();
    if let Some(was) = readings.station_was.replace(station_now.clone()).filter(|was| station_now.moved_from(was) && !crate::panel::reduced_motion()) {
        if readings.morph.is_none() {
            readings.morph = Some((now, Before::Drawn(crate::station::run::as_was(was, || render::record(frame, scene, picture.lanes, &shown.layout, picture.small, 1.0, picture.at)))));
        }
    }
    let turning = readings.morph.as_ref().map(|(since, _)| morph::ease(now.duration_since(*since).as_secs_f32() / MORPH.as_secs_f32()));
    let old = match &readings.morph {
        Some((_, Before::Shown(old))) => Some((view::lanes_at(scene, |_| old.detail), placed(old), old.clone())),
        _ => None,
    };
    let (layers, before, drawn, morph_layer) = (&mut readings.layers, readings.morph.as_mut().map(|(_, before)| before), &mut readings.drawn, &mut readings.morph_layer);
    match (turning, before) {
        (Some(t), Some(before)) => {
            // Turning: the skin's ground as it is now, and over it each
            // reading between where it was and where it goes.
            if others.is_some() {
                if let Err(error) = layers.draw_ground(frame, picture, local, px) {
                    note(format!("widget: its ground not drawn: {error}"));
                    whole = false;
                }
            }
            if let Some((old_lanes, (k, old_at), old)) = &old {
                let small = old.small.map(|small| (small, old.size));
                *before = Before::Drawn(render::record(frame, scene, old_lanes, &old.layout, small, *k, *old_at));
            }
            let now_drawn = render::record(frame, scene, picture.lanes, &shown.layout, picture.small, 1.0, picture.at);
            let Before::Drawn(from) = before else { return (None, whole) };
            let parts = morph::between(from, &now_drawn, t);
            // Within its frame: what is still on its way from a larger
            // layout does not spill past it.
            let area = (0.0, 0.0, picture.size.0, picture.size.1);
            let turned = morph_layer.draw(frame, t.to_bits() as u64 + 1, local, area, px, halo, |frame| {
                morph::draw(frame, &parts);
                Ok(())
            });
            if let Err(error) = turned {
                note(format!("widget: its turn between layouts not drawn: {error}"));
                whole = false;
            }
            *drawn = parts;
            (None, whole)
        }
        _ => match layers.draw_on(frame, picture, local, px, others) {
            // (Its readings drawn again: where each thing to press now is.)
            Ok(drawn) => (drawn.hits, whole),
            Err(error) => {
                note(format!("widget: not drawn: {error}"));
                (None, false)
            }
        },
    }
}

/// The readings a widget shows, and what it keeps of them from frame to frame.
struct Readings {
    layers: PanelLayers,
    choice: Choice,
    /// What it showed last frame.
    last: Option<Shown>,
    /// Turning from one layout into the one it shows: since when, from
    /// what; what was drawn last as it turns, and the layer it is drawn on.
    morph: Option<(Instant, Before)>,
    drawn: Vec<Part>,
    morph_layer: Layer,
    /// What it drew as a thing was carried off its station (see `render::Away`).
    away: render::Away,
    /// What its station showed as it was last drawn: changed since, it turns from how that was drawn (as the panel
    /// does: see `Panel::station_was`).
    station_was: Option<std::sync::Arc<crate::station::run::Shown>>,
    /// The form it shows and the room it was taken in while sized: a form
    /// showing more comes back only with room to spare.
    taken: Option<arrange::Shown>,
    /// How far its process list is turned (DIPs), and where the wheel turns it to (see `Widgets::wheel`).
    /// How far its lists are scrolled (see `view::Scrolls`).
    scrolls: view::Scrolls,
    /// Its station's page shown (see `view::Tab`).
    tab: view::Tab,
    /// Its lanes the workbench's (torn off a workbench's panel), not the readings'.
    workbench: bool,
    /// The one lane it shows (its module's id: torn off by that lane's head), whatever else the panel shows, and shown
    /// though it is switched off there.
    only: Option<String>,
    /// Which note its notes turned through by pages show (see `view::keep_note`).
    note_shown: view::NoteShown,
    /// Where a press finds what it shows, as it was last drawn anew (DIPs from its corner): merged, given with what
    /// plays' again each frame it is drawn from its layers as it was (see `draw_merged`).
    hits: Vec<view::HitBox>,
}

impl Readings {
    fn new(choice: Choice) -> Self {
        Readings { layers: PanelLayers::default(), choice, last: None, morph: None, drawn: Vec::new(), morph_layer: Layer::default(), away: render::Away::default(), station_was: None, taken: None, scrolls: view::Scrolls::default(), tab: view::Tab::default(), workbench: false, only: None, note_shown: view::NoteShown::default(), hits: Vec::new() }
    }

    /// Where a press finds what it shows, as it was drawn this frame: anew (where `drawn` says), or from its layers as
    /// it was, where it was last drawn anew. (Merged, a frame that drew what plays alone anew left them out: a press,
    /// the pointer and the wheel found nothing on the readings till they were next drawn anew.)
    fn found(&mut self, drawn: Option<Vec<view::HitBox>>) -> Vec<view::HitBox> {
        if let Some(drawn) = drawn {
            self.hits = drawn;
        }
        self.hits.clone()
    }

    /// What it shows, as the settings keep it.
    fn kept(&self) -> crate::settings::PanelLayout {
        let kept = self.choice.kept();
        crate::settings::PanelLayout { workbench: self.workbench, form: form_of(self.only.as_deref(), kept.form), ..kept }
    }
}

/// What plays as a widget shows it: its size (DIPs, as large as it is on the screen: it is laid out for its room, not
/// zoomed), and the layer it is drawn in, again each frame. (Drawn in a layer, not straight onto the surface: text drawn
/// straight onto it is drawn a little differently in each place of the compositor's atlas the frames go to, and so
/// shook by a pixel frame by frame; a layer is drawn in its own place, and put on whole pixels.)
struct MusicShown {
    music: Music,
    size: (f32, f32),
    layer: Layer,
    /// Frames drawn: each draws its layer anew.
    frames: u64,
    /// Whether it is to be drawn again soon (see `music::Painted`).
    busy: bool,
    /// The skin's ground under it, where it is drawn on the skin (alone, or as the one others are merged into).
    skin_ground: render::GroundLayers,
}

impl MusicShown {
    fn new(size: (f32, f32)) -> Self {
        MusicShown { music: Music::default(), size: (size.0.max(music::LEAST.0), size.1.max(music::LEAST.1)), layer: Layer::default(), frames: 0, busy: false, skin_ground: render::GroundLayers::default() }
    }
}

/// The room round what plays for its shadow (DIPs).
const MUSIC_SHADOW: f32 = 24.0;

/// `theme` as a widget of the workbench's lanes, or of one lane alone, is drawn in: no bar under them (its buttons are
/// over it, as every widget's are, and the bar held only how long the PC has been up, which none of them is about).
pub fn workbench_dressed(mut theme: Theme) -> Theme {
    theme.bar_gap = 0.0;
    theme.bar_height = 0.0;
    theme
}

impl Content {
    /// What it kept to be drawn again, let go of (its window hidden).
    fn release(&mut self) {
        match self {
            Content::Readings(readings) => {
                readings.layers.release();
                readings.morph = None;
                readings.morph_layer.release();
            }
            Content::Music(shown) => {
                shown.music.release();
                shown.layer.release();
            }
            Content::Merged(merged) => merged.parts.iter_mut().for_each(Content::release),
        }
    }

    /// Its readings (merged, the first readings part).
    fn readings_mut(&mut self) -> Option<&mut Readings> {
        match self {
            Content::Readings(readings) => Some(readings),
            Content::Music(_) => None,
            Content::Merged(merged) => merged.parts.iter_mut().find_map(Content::readings_mut),
        }
    }

    /// Its station's page shown: its readings' (merged, the first readings part's).
    fn tab(&self) -> view::Tab {
        match self {
            Content::Readings(readings) => readings.tab,
            Content::Music(_) => view::Tab::default(),
            Content::Merged(merged) => merged.parts.iter().find(|part| matches!(part, Content::Readings(_))).map_or_else(view::Tab::default, Content::tab),
        }
    }

    /// How far its lists are scrolled: its readings' (merged, the first readings part's).
    /// Whether it is, or holds, the single lane `only` (see `Readings::only`): which of its parts, where merged.
    /// Its own colour, its hint drawn in it: what plays, its ground's, a note, its own (`written`: as it is being written
    /// in, else as it was last read).
    fn ground(&self, written: Option<crate::notes::Colour>, music: MusicGround) -> Option<Ground> {
        match self {
            // (On the skin, the skin's.)
            Content::Music(_) if music == MusicGround::Skin => None,
            Content::Music(shown) => Some(Ground::Own(shown.music.ground_colour(shown.size))),
            Content::Readings(Readings { only: Some(only), .. }) => only.strip_prefix(view::STUCK_NOTE_LANE).and_then(|name| {
                written.or_else(|| crate::notes::shown().iter().find(|note| note.name == name).map(|note| note.colour)).map(|colour| Ground::Tint(view::note_colour(colour)))
            }),
            _ => None,
        }
    }

    fn holding(&self, only: &str) -> Option<Option<usize>> {
        match self {
            Content::Readings(readings) => (readings.only.as_deref() == Some(only)).then_some(None),
            Content::Merged(merged) => merged.parts.iter().position(|part| part.holding(only).is_some()).map(Some),
            Content::Music(_) => None,
        }
    }

    fn scrolls(&self) -> view::Scrolls {
        match self {
            Content::Readings(readings) => readings.scrolls,
            Content::Music(_) => view::Scrolls::default(),
            Content::Merged(merged) => merged.parts.iter().find(|part| matches!(part, Content::Readings(_))).map_or_else(view::Scrolls::default, Content::scrolls),
        }
    }

    /// Whether it shows the workbench's lanes (torn off its panel), not the readings'.
    fn workbench(&self) -> bool {
        match self {
            Content::Readings(readings) => readings.workbench,
            Content::Merged(merged) => merged.parts.iter().any(|part| matches!(part, Content::Readings(readings) if readings.workbench)),
            Content::Music(_) => false,
        }
    }

    /// `theme` as it is drawn in: the workbench's, or one lane's alone, with no bar under its lanes (see
    /// `workbench_dressed`). (All the panel's lanes torn off together keep the panel's bar.)
    ///
    /// A note stuck on the desktop alone: its card the whole of it, with no room round it, the skin's ground (the
    /// desktop frosted, on glass) as round as the card, under it alone (in the skin's sheet, the card read as a note in
    /// a frame; on no ground, glass's card was clear over the desktop, not frosted as on the panel).
    fn dressed(&self, mut theme: Theme) -> Theme {
        if self.workbench() || matches!(self, Content::Readings(readings) if readings.only.is_some()) {
            theme = workbench_dressed(theme);
        }
        if self.bare() {
            (theme.pad_x, theme.pad_top, theme.pad_bottom) = (0.0, 0.0, 0.0);
            theme.outer = (0.0, 0.0, 0.0);
            // (Its page alone, no lane ruled off under it: chart paper's rule ran under the page's foot, past its corners.)
            theme.ruled = false;
        }
        theme
    }

    /// Whether it is a note stuck on the desktop alone: its card the whole of it (see `dressed`).
    fn bare(&self) -> bool {
        self.stuck_note().is_some()
    }

    /// The name of the note it is, stuck on the desktop alone.
    fn stuck_note(&self) -> Option<&str> {
        match self {
            Content::Readings(readings) => readings.only.as_deref().and_then(|only| only.strip_prefix(view::STUCK_NOTE_LANE)),
            _ => None,
        }
    }

    /// The preferences its readings are drawn by, where they are not the panel's (`prefs`): without the music lane
    /// (what plays is a widget of its own); merged with what plays as one of their lanes, with it, on.
    fn prefs(&self, prefs: &crate::ui::prefs::Prefs) -> Option<crate::ui::prefs::Prefs> {
        // The workbench's lanes, for readings torn off its panel.
        let workbench = self.workbench();
        let mut prefs = if workbench { prefs.clone().holding(crate::settings::Part::Workbench) } else { prefs.clone() };
        // One lane alone: that one, on, whatever else the panel shows (taken out of the edge since, or switched off
        // there, it is still what this widget shows).
        let only = match self {
            Content::Readings(readings) => readings.only.clone(),
            Content::Merged(merged) => merged.parts.iter().find_map(|part| if let Content::Readings(readings) = part { readings.only.clone() } else { None }),
            Content::Music(_) => None,
        };
        if let Some(only) = only {
            prefs.modules.retain(|module| module.id == only);
            match prefs.modules.first_mut() {
                Some(module) => module.on = true,
                None => prefs.modules.push(crate::ui::prefs::ModuleEntry { id: only, on: true, items: Default::default() }),
            }
        }
        match self {
            Content::Merged(merged) if merged.music_apart() => match prefs.modules.iter_mut().find(|module| module.id == "lyrics") {
                Some(module) => module.on = true,
                None => prefs.modules.push(crate::ui::prefs::ModuleEntry { id: "lyrics".into(), on: true, items: Default::default() }),
            },
            Content::Readings(_) | Content::Merged(_) => prefs.modules.retain(|module| module.id != "lyrics"),
            Content::Music(_) => return None,
        }
        Some(prefs)
    }

    /// Whether what plays in it is one of its readings' lanes.
    fn music_apart(&self) -> bool {
        matches!(self, Content::Merged(merged) if merged.music_apart())
    }

    /// Whether `guest` may be merged into it: one of its own, not merged (merged ones' parts would be nested in it,
    /// drawn by nothing, and kept as what plays alone), of a kind it has not got, its parts not too many.
    fn takes(&self, guest: &Content) -> bool {
        if matches!(guest, Content::Merged(_)) {
            return false;
        }
        let same = |part: &Content| std::mem::discriminant(part) == std::mem::discriminant(guest);
        match self {
            Content::Merged(merged) => merged.parts.len() < PARTS_MOST && !merged.parts.iter().any(same),
            _ => !same(self),
        }
    }

    /// Whether it is drawn on the skin (the readings, or what was merged into them; what plays, as `music` says).
    fn skinned(&self, music: MusicGround) -> bool {
        match self {
            Content::Readings(_) => true,
            Content::Music(_) => music == MusicGround::Skin,
            Content::Merged(merged) => !merged.host_plays() || music == MusicGround::Skin,
        }
    }

    /// As the journal names what it shows.
    fn key(&self) -> String {
        match self {
            Content::Readings(readings) => readings.choice.form.key(),
            Content::Music(_) => MUSIC_FORM.into(),
            Content::Merged(merged) => format!("merged ({})", merged.parts.iter().map(Content::key).collect::<Vec<_>>().join(", ")),
        }
    }

    /// Its buttons, as they stand in their row: the game's only with the readings (the game lane is theirs).
    fn buttons(&self) -> &'static [Button] {
        match self {
            // (A game shown or not changes only the readings': the workbench's has nothing of it.)
            Content::Readings(readings) if readings.workbench => &[Button::Settings, Button::Pin, Button::Through, Button::Close],
            Content::Readings(_) => &buttons::ALL,
            Content::Music(_) => &[Button::Full, Button::Settings, Button::Pin, Button::Through, Button::Close],
            // (With the readings' in it, as theirs; else with the workbench's, as its.)
            Content::Merged(merged) => merged
                .parts
                .iter()
                .find(|part| matches!(part, Content::Readings(readings) if !readings.workbench))
                .or_else(|| merged.parts.iter().find(|part| matches!(part, Content::Readings(_))))
                .map_or(&[Button::Settings, Button::Pin, Button::Through, Button::Close], Content::buttons),
        }
    }
}

/// Every widget on the desktop, and those put away this session, the last
/// first back.
#[derive(Default)]
pub struct Widgets {
    list: Vec<Widget>,
    put_away: Vec<WidgetAt>,
    /// The settings window, while it is in front: the widgets give way to it.
    settings: Option<HWND>,
    /// Restored from the settings once a graphics device is there.
    restored: bool,
    /// Widgets to add as they are (see `adopt`).
    adopted: Vec<WidgetAt>,
    /// The system saying when other programs' windows change (see
    /// `WindowWatch`), while there are widgets.
    watching: Option<WindowWatch>,
    /// Where each widget was last (its window, physical px), shown.
    areas: Vec<(isize, RECT)>,
    /// Live refraction, as the widgets were last drawn (see `Look::live`).
    live: bool,
    /// What plays is drawn on, as the widgets were last drawn (see `prefs::MusicGround`).
    music_ground: MusicGround,
    /// A widget in hand held over another, to be merged into it as it is let go (see `look_for_a_drop`).
    dropping: Option<Dropping>,
    /// The menu a press on a widget's station asked for (see `take_menu_asked`): the button, the page its station shows.
    menu_asked: Option<(view::Hit, view::Tab)>,
    /// The menu of a note written in where it is asked for by its button (see `take_note_menu_asked`): the note's number.
    note_menu_asked: Option<u64>,
    /// The writing menu of a note written in where it is asked for on its words (see `take_writing_menu_asked`): the
    /// widget, the note's name, where on the screen.
    writing_menu_asked: Option<(HWND, String, POINT)>,
    /// The music shown over the whole of a screen, while it is (asked for by a music widget's button).
    full: Option<crate::fullscreen::FullMusic>,
}

/// A widget in hand (`guest`) held over another (`target`): since when, whether it goes beside it (else below), and
/// whether it has been held there long enough to be merged into it as it is let go.
#[derive(Clone, Copy, PartialEq)]
struct Dropping {
    guest: isize,
    target: isize,
    beside: bool,
    since: Instant,
    armed: bool,
}

/// Whether a widget held over another is merged into it. Off for now (the owner's call, 2026-10-19): merged widgets'
/// looks are to be designed first. Those merged before stay as they are.
const MERGING: bool = false;

/// How long a widget in hand is held over another before it is merged into it as it is let go; how far into the
/// other's width from its right edge it goes beside it (else below); how many parts a widget merges at most.
const DROP_HOLD: Duration = Duration::from_millis(600);
const DROP_BESIDE: f32 = 0.3;
const PARTS_MOST: usize = 4;

impl Widgets {
    /// Whether `hwnd` is one of the widgets'.
    pub fn owns(&self, hwnd: HWND) -> bool {
        self.list.iter().any(|w| w.window.hwnd == hwnd)
    }

    /// A widget torn off the panel: showing `form` at `scale` with its
    /// corner at `at` on `contact`'s screen, in hand, the pointer `grab`
    /// from its corner; on the desktop the panel had taken (`lent`: its
    /// capture, as a bitmap, and its tone), if it had, till it takes its own
    /// (on another thread: it is drawn at once). `from`: the edge the panel
    /// was at, if it was at one; `part`: what the panel showed; `only`: the one lane of it it shows (torn off by that
    /// lane's head), if it shows one.
    #[allow(clippy::too_many_arguments)]
    pub fn tear(&mut self, form: Form, scale: f32, at: POINT, grab: POINT, size: (f32, f32), lent: Option<Lent>, from: Option<Edge>, part: crate::settings::Part, only: Option<String>) -> bool {
        let Some(contact) = monitor_at(at) else { return false };
        // (Its top on the screen, not above it: torn off high, as the hand pulled it, it is held lower.)
        let below = (contact.work.top - at.y).max(0);
        let (at, grab) = (POINT { y: at.y + below, ..at }, POINT { y: grab.y - below, ..grab });
        let choice = Choice { form, scale, extra: (0.0, 0.0) };
        let made = Instant::now();
        // What the panel showed, as large as it was.
        let content = match part {
            crate::settings::Part::Monitor => Content::Readings(Readings { only, ..Readings::new(choice) }),
            crate::settings::Part::Workbench => Content::Readings(Readings { workbench: true, only, ..Readings::new(choice) }),
            crate::settings::Part::Music => Content::Music(MusicShown::new(size)),
        };
        let Some(mut widget) = Widget::new(contact, at, content) else { return false };
        let made = made.elapsed();
        if made > SLOW_FRAME {
            note(format!("widget: made in {:.1} ms", made.as_secs_f32() * 1000.0));
        }
        widget.drag = Some(Drag::Move { grab, trail: Vec::new(), torn: true, moved: true });
        let side = from.map(|edge| match edge {
            Edge::Left => Side::Left,
            Edge::Right => Side::Right,
            Edge::Top => Side::Top,
        });
        widget.torn_from = Some((side, contact.monitor));
        if let Some(Lent { capture, bitmap, tone }) = lent {
            widget.behind = Some(Behind { capture, bitmap: Some(bitmap), former: None, tone, taken: Instant::now() });
            widget.replace = Some(false);
        }
        widget.window.yield_to(self.settings);
        widget.window.lift(true);
        unsafe { SetCapture(widget.window.hwnd) };
        self.list.push(widget);
        true
    }

    /// The note `name` stuck on the desktop: a widget of its page alone, its corner at `at`, `size` large (DIPs: the
    /// page's it is stuck from; none, as `view::STUCK_NOTE` makes it). One stuck already is raised instead. (The page
    /// it comes from told as it shows: see `beside::stuck_shown`.)
    pub fn stick_note(&mut self, name: &str, at: POINT, size: Option<(f32, f32)>) {
        if self.raise_note(name) {
            crate::ui::beside::stuck_shown(name);
            return;
        }
        let Some(contact) = monitor_at(at) else {
            crate::ui::beside::stuck_shown(name);
            return;
        };
        let (wide, tall) = size.unwrap_or(view::STUCK_NOTE);
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (wide - view::STUCK_NOTE.0, tall - view::STUCK_NOTE.1) };
        let content = Content::Readings(Readings { workbench: true, only: Some(format!("{}{name}", view::STUCK_NOTE_LANE)), ..Readings::new(choice) });
        match Widget::new(contact, at, content) {
            Some(widget) => {
                widget.window.yield_to(self.settings);
                self.list.push(widget);
                self.save();
            }
            None => crate::ui::beside::stuck_shown(name),
        }
    }

    /// The note `name` stuck on the desktop taken away (the note deleted): its widget not put away to be brought back,
    /// gone; merged with another, the other left as it was.
    pub fn unstick_note(&mut self, name: &str) {
        let only = format!("{}{name}", view::STUCK_NOTE_LANE);
        let Some((index, part)) = self.list.iter().enumerate().find_map(|(index, widget)| widget.content.holding(&only).map(|part| (index, part))) else { return };
        match part {
            None => {
                let mut widget = self.list.remove(index);
                widget.gone();
                widget.window.destroy();
            }
            Some(part) => {
                let widget = &mut self.list[index];
                let _ = widget.take_part(part);
                widget.settle = Some((Instant::now(), widget.surface_size));
                widget.next_frame = Instant::now();
            }
        }
        self.save();
    }

    /// The widget the note `name` is stuck on the desktop as (alone, or merged with another), raised in front; whether
    /// there is one.
    pub fn raise_note(&mut self, name: &str) -> bool {
        let only = format!("{}{name}", view::STUCK_NOTE_LANE);
        let Some(widget) = self.list.iter_mut().find(|widget| widget.content.holding(&only).is_some()) else { return false };
        widget.window.raise();
        // (Lifted for a moment where it is, that it is seen to be the one.)
        widget.lift.speed = NUDGE;
        widget.next_frame = Instant::now();
        true
    }

    /// The note `name` pulled off the panel (or its page) onto the desktop: stuck there with its corner at `at`, in
    /// hand, the pointer `grab` from its corner, to be put down where it is let go. Where it is stuck already, that one
    /// raised instead (a note is never on the desktop twice); whether one was made.
    pub fn stick_note_held(&mut self, pulled: NotePulled) -> bool {
        let NotePulled { name, card, from, over, px } = pulled;
        if self.raise_note(&name) {
            return false;
        }
        let at = POINT { x: card.left, y: card.top };
        let grab = POINT { x: from.x - card.left, y: from.y - card.top };
        let Some(contact) = monitor_at(at) else { return false };
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let content = Content::Readings(Readings { workbench: true, only: Some(format!("{}{name}", view::STUCK_NOTE_LANE)), ..Readings::new(choice) });
        let Some(mut widget) = Widget::new(contact, at, content) else { return false };
        // (Not torn off: the panel it came from stays, as it does as a note is stuck from its menu.)
        widget.drag = Some(Drag::Move { grab, trail: Vec::new(), torn: false, moved: true });
        // Lifted off its card, as large and where it was (see `Pulled`).
        widget.surface_size = ((card.right - card.left) as f32 / contact.scale, (card.bottom - card.top) as f32 / contact.scale);
        let held = ((from.x - card.left) as f32 / (card.right - card.left).max(1) as f32, (from.y - card.top) as f32 / contact.scale);
        widget.pulled = Some(Pulled { number: crate::notes::number_of(&name), over, card, card_px: px, grab: held, morph: Spring::at(0.0, CARD_TO_NOTE.0, CARD_TO_NOTE.1), back: None, landed: false });
        widget.about = (grab.x as f32 / contact.scale, grab.y as f32 / contact.scale);
        widget.window.yield_to(self.settings);
        widget.window.lift(true);
        widget.window.raise();
        unsafe { SetCapture(widget.window.hwnd) };
        self.list.push(widget);
        self.save();
        true
    }

    /// What plays torn off the panel's music lane: a music widget `size` large (DIPs) with its corner at `at`, in
    /// hand, the pointer `grab` from its corner.
    pub fn tear_music(&mut self, size: (f32, f32), at: POINT, grab: POINT) -> bool {
        let Some(contact) = monitor_at(at) else { return false };
        let Some(mut widget) = Widget::new(contact, at, Content::Music(MusicShown::new(size))) else { return false };
        note("widget: what plays torn off the panel");
        widget.drag = Some(Drag::Move { grab, trail: Vec::new(), torn: true, moved: true });
        widget.torn_from = Some((None, contact.monitor));
        widget.window.yield_to(self.settings);
        // (In hand: over the panel, as one pressed is.)
        widget.window.lift(true);
        unsafe { SetCapture(widget.window.hwnd) };
        self.list.push(widget);
        true
    }

    /// The notes carried off the panel now, by their numbers: in hand, or going back to it (see `Pulled`).
    pub fn carried(&self) -> Vec<u64> {
        let in_hand = |widget: &Widget| matches!(widget.drag, Some(Drag::Move { .. }));
        self.list.iter().filter_map(|widget| widget.pulled.as_ref().filter(|pulled| pulled.carried(in_hand(widget))).map(|pulled| pulled.number)).collect()
    }

    /// Whether the widget `hwnd` is in hand (moved, sized, or its sound's track slid).
    pub fn held(&self, hwnd: HWND) -> bool {
        self.list.iter().any(|widget| widget.window.hwnd == hwnd && widget.drag.is_some())
    }

    /// The widget torn off the panel and in hand still, if one is.
    pub fn torn_in_hand(&self) -> Option<HWND> {
        self.list.iter().find(|widget| matches!(widget.drag, Some(Drag::Move { torn: true, .. }))).map(|widget| widget.window.hwnd)
    }

    /// Whether the widget torn off last is on the screen yet.
    pub fn torn_shown(&self) -> bool {
        self.list.last().is_some_and(|widget| widget.shown_once)
    }

    /// Whether the widget torn off last is there, waiting to show.
    pub fn torn_waiting(&self) -> bool {
        self.list.last().is_some_and(|widget| !widget.shown_once)
    }

    /// Brings back the widget put away last; whether there was one.
    pub fn recall(&mut self) -> bool {
        let Some(kept) = self.put_away.pop() else { return false };
        if let Some(widget) = Widget::restore(&kept) {
            widget.window.yield_to(self.settings);
            self.list.push(widget);
            self.save();
        }
        true
    }

    /// The settings window came to the front (`Some`) or left it: every
    /// widget gives way to it meanwhile, as the panel does, so that it is
    /// never covered.
    pub fn settings_front(&mut self, settings: Option<HWND>) {
        // Left the front: what was taken while it was there, to be taken again.
        if self.settings.is_some() && settings.is_none() {
            for widget in &mut self.list {
                widget.retake = true;
            }
        }
        // (Drawn again as the settings come or go: a widget they were asked from shows its settings' button while they
        // are open.)
        for widget in &mut self.list {
            widget.next_frame = Instant::now();
        }
        self.settings = settings;
        for widget in &self.list {
            widget.window.yield_to(settings);
        }
    }

    /// A frame of what is behind `hwnd` has come, or has been read (see `CAPTURED`): taken, if it is due, and drawn
    /// with at once, if what is behind changed (see `Widget::look_behind`).
    pub fn captured(&mut self, hwnd: HWND) {
        let live = self.live;
        if let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) {
            let live = live && widget.content.skinned(self.music_ground);
            if widget.look_behind(live) {
                widget.next_frame = Instant::now();
            }
        }
    }

    /// The screens changed (one added or taken away, its size or scale):
    /// each widget on the screen it is on now, all of it on it once at rest,
    /// the desktop behind it taken again.
    pub fn screens_changed(&mut self) {
        if let Some(full) = self.full.as_mut() {
            full.screens_changed();
        }
        for widget in &mut self.list {
            let r = widget.rect();
            let Some(contact) = monitor_at(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }).or_else(|| monitor_at(widget.at)) else { continue };
            // Its screen as it was (a window placed for the first time hears
            // of its DPI too): what was taken of it stands.
            if contact == widget.contact {
                continue;
            }
            note("widget: its screen changed; the desktop taken again");
            widget.moved_to(contact);
            if widget.behind.is_some() {
                widget.replace = Some(true);
            }
            widget.next_frame = Instant::now();
        }
        // Each hidden on the device of its screen now, where no screen needs the one it held, and this thread's devices
        // no window holds and no screen needs let go of: a card whose screen was unplugged is kept awake by none.
        for widget in self.list.iter_mut().filter(|widget| !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(widget.window.hwnd) }.as_bool()) {
            widget.follow_device_hidden();
        }
        gfx::let_go_of_strays();
    }

    /// A widget to add as the others are restored (a 0.2.3 panel left on
    /// the desktop), saved with them.
    pub fn adopt(&mut self, kept: WidgetAt) {
        self.adopted.push(kept);
    }

    /// Whether any widget was put away this session.
    pub fn any_put_away(&self) -> bool {
        !self.put_away.is_empty()
    }

    /// Whether a widget on the screen now shows the process list (what it was last drawn with says).
    /// Whether any is on the screen, or the music full screen is up.
    pub fn any_seen(&self) -> bool {
        self.full.is_some() || self.list.iter().any(|widget| widget.showing.is_some())
    }

    pub fn any_showing_processes(&self) -> bool {
        self.list.iter().any(|widget| widget.showing.is_some() && widget.hits.iter().any(|hit| matches!(hit.4, view::Hit::List(view::List::Processes, _))))
    }

    /// The widgets kept in the settings (and a panel 0.2.3 left on the
    /// desktop, as a widget showing all of each lane), once.
    fn restore(&mut self, kept: &[WidgetAt]) {
        self.restored = true;
        self.list.extend(kept.iter().filter_map(Widget::restore));
        if !self.adopted.is_empty() {
            let adopted = std::mem::take(&mut self.adopted);
            self.list.extend(adopted.iter().filter_map(Widget::restore));
            self.save();
        }
        // Giving way to the settings window, if it is in front.
        for widget in &self.list {
            widget.window.yield_to(self.settings);
        }
    }

    /// Saves where every widget is and what it shows: by one thread, the
    /// newest of what was asked for since it last saved, so that an older
    /// list never lands after a newer one.
    fn save(&self) {
        static SAVER: std::sync::OnceLock<std::sync::mpsc::Sender<Vec<WidgetAt>>> = std::sync::OnceLock::new();
        let kept: Vec<WidgetAt> = self.list.iter().map(Widget::kept).collect();
        let saver = SAVER.get_or_init(|| {
            let (send, receive) = std::sync::mpsc::channel::<Vec<WidgetAt>>();
            std::thread::spawn(move || {
                while let Ok(mut kept) = receive.recv() {
                    while let Ok(newer) = receive.try_recv() {
                        kept = newer;
                    }
                    crate::app().change(|settings| settings.widgets = kept);
                }
            });
            send
        });
        let _ = saver.send(kept);
    }

    /// A press on a widget's window at `client` (physical px in it), `keys` held.
    pub fn press(&mut self, hwnd: HWND, client: POINT, keys: crate::station::select::Keys) {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) else { return };
        widget.next_frame = Instant::now();
        widget.press(client, keys);
    }

    /// The pointer over a widget's window at `client` (physical px in it):
    /// the shape of what a press there does.
    pub fn hover(&mut self, hwnd: HWND, client: POINT, lang: Lang) {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) else { return };
        // Its buttons show as the pointer comes: drawn at once (moving over
        // it after, nothing that shows changes).
        if !widget.hover {
            widget.next_frame = Instant::now();
        }
        // A thing on its station pressed and moved far enough: dragged off it (by the station, which takes the
        // pointer from here).
        if let (Some(press), Some(now)) = (widget.shelf_press, cursor_position()) {
            if crate::station::run::drag_begins(press.from, now) {
                widget.shelf_press = None;
                // Stroked across its pages at once: they are swiped (the pointer kept).
                if widget.swipes.is_some_and(|(.., pressed)| pressed.elapsed() < HOLD) && widget.begin_swipe(press.from, now, Instant::now()) {
                    widget.next_frame = Instant::now();
                    return;
                }
                crate::ui::window::let_go_of_capture(widget.window.hwnd);
                // (What was said of what was pressed is not said while it is carried: the pointer is the drag's from
                // here, and the widget hears no more of it to take it away.)
                widget.unsay();
                // (With what is chosen with it.)
                let carried = crate::station::select::drag(press);
                if let Some(drag) = crate::station::drag_off(&carried, unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }) {
                    widget.lifting = Some((press.hit, carried, drag, press.from));
                    widget.next_frame = Instant::now();
                }
            }
            return;
        }
        if widget.drag.is_some() {
            return;
        }
        let px = widget.contact.scale * widget.zoom;
        let margin = widget.margin_px();
        let (x, y) = ((client.x - margin) as f32 / px, (client.y - margin) as f32 / px);
        // What the button under the pointer does, said under it.
        let over = widget.button_at(x, y);
        if over != widget.tip {
            widget.tip = over;
            // (Under the button itself, where that is on the screen: not wherever the pointer came onto it.)
            let about = over.and_then(|button| widget.buttons().into_iter().find(|(_, one)| *one == button)).map(|(centre, _)| {
                let on_screen = |dx: f32, dy: f32| ((widget.at.x as f32 + (centre.x + dx) * px).round() as i32, (widget.at.y as f32 + (centre.y + dy) * px).round() as i32);
                let ((left, top), (right, bottom)) = (on_screen(-BUTTON / 2.0, -BUTTON / 2.0), on_screen(BUTTON / 2.0, BUTTON / 2.0));
                RECT { left, top, right, bottom }
            });
            match (over, about) {
                (Some(button), Some(at)) => {
                    let lit = match button {
                        Button::Pin => widget.pinned,
                        Button::Through => widget.click_through,
                        Button::Game => crate::ui::view::game_shown(crate::app().controller.history.latest().as_ref().map(std::slice::from_ref).unwrap_or_default()),
                        _ => false,
                    };
                    crate::ui::tip::show(widget.window.hwnd, button.says(lit, lang, false), at, crate::os::apps_dark());
                    widget.saying = false;
                }
                _ => crate::ui::tip::hide(),
            }
        }
        // What a press there would open or do, lit (a drive's row, a player's button).
        let (lx, ly) = (x - widget.hits_from.0, y - widget.hits_from.1);
        let hit = widget.hits.iter().find(|(hx, hy, w, h, _)| lx >= *hx && lx < hx + w && ly >= *hy && ly < hy + h).map(|hit| hit.4).filter(|hit| hit.takes_press());
        if hit != widget.hit_hover {
            // What a press does, said under what says so (a drive's row, the line that switches players, its buttons'
            // own said over it). Nothing while a thing is dragged off a station's lane: the pointer is the drag's, and
            // nothing here would hear it go to take it away.
            match hit.and_then(|hit| Some((hit, crate::panel::says(hit, lang, &widget.hits)?))) {
                Some((hit, said)) if over.is_none() && !crate::station::drag::under_way() => {
                    if let Some((hx, hy, hw, hh)) = widget.hits.iter().find(|one| one.4 == hit).map(|one| (one.0, one.1, one.2, one.3)) {
                        let on_screen = |dx: f32, dy: f32| {
                            // (Its surface's corner is at `at`; the window's margin is before it.)
                            ((widget.at.x as f32 + (widget.hits_from.0 + dx) * px).round() as i32, (widget.at.y as f32 + (widget.hits_from.1 + dy) * px).round() as i32)
                        };
                        let ((left, top), (right, bottom)) = (on_screen(hx, hy), on_screen(hx + hw, hy + hh));
                        crate::ui::tip::show(widget.window.hwnd, &said, RECT { left, top, right, bottom }, crate::os::apps_dark());
                        widget.saying = true;
                    }
                }
                _ if widget.saying && over.is_none() => {
                    crate::ui::tip::hide();
                    widget.saying = false;
                }
                _ => {}
            }
            widget.hit_hover = hit;
            widget.next_frame = Instant::now();
        }
        // (Over the words written in where they are, theirs: see `ui::keys`. Its menu's button, the hand.)
        let writing = widget.in_place.as_ref().filter(|in_place| in_place.taking);
        if over.is_none() && writing.is_some_and(|in_place| in_place.writing.editor.holds(client)) {
            return;
        }
        // (As a press there acts: see `takes_at`.)
        let shape = pointer_for(&widget.takes_at(x, y), over.is_some(), hit, writing.is_some_and(|in_place| in_place.on_menu_button(client, px)));
        widget.window.point(shape);
    }

    /// Turns the process list under the pointer (`screen`, physical px) by the wheel's `delta`, as the panel's; or
    /// the lines of what plays. Turned `across` (a touchpad stroked sideways, a wheel tilted), the pages under it alone,
    /// to the right on.
    pub fn wheel(&mut self, hwnd: HWND, screen: POINT, delta: i16, across: bool) {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) else { return };
        // What it shows where the wheel turned, as a press there finds it.
        let hit: Vec<view::Hit> = {
            let mut client = screen;
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client);
            }
            let px = widget.contact.scale * widget.zoom;
            let margin = widget.margin_px();
            let (x, y) = ((client.x - margin) as f32 / px - widget.hits_from.0, (client.y - margin) as f32 / px - widget.hits_from.1);
            view::under(&widget.hits, (x, y)).map(|hit| hit.4).collect()
        };
        // Its pages turned, or its list scrolled (under what is on it: the wheel over a thing on the station scrolls the
        // station, and over its bar too). Turned across, only pages.
        let turn = |widget: &mut Widget| {
            if widget.content.readings_mut().is_some_and(|readings| view::wheel(&mut readings.scrolls, &hit, delta, across)) {
                widget.unsay();
                widget.next_frame = Instant::now();
            }
        };
        if across {
            turn(widget);
            return;
        }
        let on_sound = hit.first().is_some_and(|hit| hit.of_sound());
        if let Content::Music(shown) = &mut widget.content {
            shown.music.wheel(delta, on_sound);
            widget.next_frame = Instant::now();
            return;
        }
        if let Content::Merged(merged) = &mut widget.content {
            let mut client = screen;
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client);
            }
            let margin = (widget.margin * widget.contact.scale).ceil() as i32;
            let at = ((client.x - margin) as f32 / widget.contact.scale, (client.y - margin) as f32 / widget.contact.scale);
            if let Some((index, _)) = merged.part_at(at) {
                if let Content::Music(shown) = &mut merged.parts[index] {
                    shown.music.wheel(delta, on_sound);
                    widget.next_frame = Instant::now();
                    return;
                }
            }
        }
        turn(widget);
    }

    /// Let go of a press on a widget, or (`cancelled`) its capture lost: then no press is completed.
    pub fn release(&mut self, hwnd: HWND, now: Instant, cancelled: bool) {
        // A thing on its station pressed and let go where it was: pressed, or with Ctrl or Shift chosen (its capture
        // lost: nothing).
        if let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) {
            if let Some(press) = widget.shelf_press.take() {
                crate::ui::window::let_go_of_capture(widget.window.hwnd);
                if !cancelled && crate::station::select::release(press) {
                    crate::station::pressed(press.hit);
                }
                widget.next_frame = now;
                return;
            }
        }
        if let Some(widget) = self.list.iter().find(|w| w.window.hwnd == hwnd) {
            note(format!("widget: release ({}), {}", if cancelled { "its capture lost" } else { "let go" }, match &widget.drag { Some(Drag::Move { moved, torn, .. }) => format!("moving (moved {moved}, torn {torn})"), Some(Drag::Size { .. }) => "sizing".into(), Some(Drag::Slide { .. }) => "sliding".into(), Some(Drag::Swipe { .. }) => "swiping".into(), Some(Drag::Scroll(_)) => "scrolling".into(), None => "in no hand".into() }));
        }
        // Held over another long enough: merged into it.
        if let Some(drop) = self.dropping.take() {
            if drop.armed && drop.guest == hwnd.0 as isize && !cancelled {
                self.merge(drop);
                self.save();
                return;
            }
            if let Some(target) = self.list.iter_mut().find(|w| w.window.hwnd.0 as isize == drop.target) {
                target.drop_target = None;
                target.next_frame = now;
            }
        }
        let Some(index) = self.list.iter().position(|w| w.window.hwnd == hwnd) else { return };
        let widget = &mut self.list[index];
        widget.next_frame = now;
        match widget.release(now, cancelled) {
            Released::Closed => self.put_away_at(index),
            Released::Changed => {}
            Released::Settings(place) => {
                // (What its button said goes with the settings coming.)
                if self.list[index].tip.take().is_some() {
                    crate::ui::tip::hide();
                }
                crate::show_settings(place);
                return;
            }
            Released::Full => {
                if self.list[index].tip.take().is_some() {
                    crate::ui::tip::hide();
                }
                let contact = self.list[index].contact;
                self.open_full(contact.monitor, contact.scale);
                return;
            }
            Released::Menu(hit) => {
                self.menu_asked = Some((hit, self.list[index].content.tab()));
                return;
            }
            Released::Nothing => return,
        }
        self.save();
    }

    /// The menu a press on a widget's station asked for (see `Released::Menu`), taken: the button pressed, and the page
    /// its station shows.
    /// The station's page `tab` shown on the widget at `hwnd` (chosen in its menu of them, or something dragged over
    /// where it is chosen): the one shown already keeps its filter.
    pub fn show_page(&mut self, hwnd: HWND, tab: view::Tab) {
        if let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) {
            if let Some(readings) = widget.content.readings_mut().filter(|readings| readings.tab != tab.again(readings.tab)) {
                readings.tab = tab.again(readings.tab);
                crate::station::select::turned(widget.window.hwnd.0 as isize, readings.tab);
                widget.next_frame = Instant::now();
            }
        }
    }

    pub fn take_menu_asked(&mut self) -> Option<(view::Hit, view::Tab)> {
        self.menu_asked.take()
    }

    /// The menu of a note written in where it is asked for by its button (see `InPlace`), taken: the note's number.
    pub fn take_note_menu_asked(&mut self) -> Option<u64> {
        self.note_menu_asked.take()
    }

    /// Whether a note's writing menu was asked for (see `take_writing_menu_asked`).
    pub fn writing_menu_was_asked(&self) -> bool {
        self.writing_menu_asked.is_some()
    }

    /// The writing menu of a note written in where it is (see `ui::note::writing_menu`), asked for on its words (the
    /// right button, the menu key), up at where it was asked for: the note's name, and what each row does. None where
    /// none was asked for.
    pub fn take_writing_menu_asked(&mut self, lang: Lang, over_the_panel: bool) -> Option<(crate::ui::menu::Menu, String, Vec<crate::ui::note::Choice>)> {
        let (hwnd, name, at) = self.writing_menu_asked.take()?;
        let spot = self.list.iter().find(|w| w.window.hwnd == hwnd).and_then(|w| w.in_place.as_ref()).map(|in_place| in_place.writing.spot()).unwrap_or_default();
        let (items, choices) = crate::ui::note::writing_menu(lang, spot);
        let open = crate::ui::menu::Menu::open_at(items, self.look_of(hwnd), at, false, over_the_panel)?;
        Some((open, name, choices))
    }

    /// A press, a move, a let go or a wheel's turn on the widget at `msg.hwnd`, for a note stuck on the desktop on it to
    /// be written in where it is (see `InPlace`): its words pressed, written in there (the press theirs, as the text
    /// services take it). Written in, the pointer's on its words (a press there, the hand followed by them till it is
    /// let go, the wheel), and on its menu's button. Whether it was theirs (else the widget's, as ever: moved, sized,
    /// its buttons pressed).
    pub fn write(&mut self, msg: &windows::Win32::UI::WindowsAndMessaging::MSG) -> bool {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == msg.hwnd) else { return false };
        widget.write(msg, &mut self.note_menu_asked)
    }

    /// What was written in the note written in where it is on the widget at `hwnd` (its words took a message: see
    /// `ui::keys::took`) seen to: dressed, kept a moment after it changed (see `note::KEEP_AFTER_MS`), drawn, the
    /// caret shown at once where it moved.
    pub fn written(&mut self, hwnd: HWND) {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) else { return };
        let now = Instant::now();
        // (A to-do written on it: drawn as it now is.)
        if crate::ui::todo::writing(hwnd) {
            widget.next_frame = now;
        }
        let Some(in_place) = widget.in_place.as_mut() else { return };
        let changed = in_place.writing.written();
        if changed {
            in_place.keep_at = Some(now + Duration::from_millis(u64::from(crate::ui::note::KEEP_AFTER_MS)));
        }
        if let Some(at) = in_place.writing.take_menu_asked() {
            self.writing_menu_asked = Some((hwnd, in_place.name.clone(), at));
        }
        let chosen = in_place.writing.chosen();
        if changed || chosen != in_place.chosen {
            (in_place.chosen, in_place.blink_from) = (chosen, now);
        }
        widget.next_frame = now;
    }

    /// The widget at `hwnd` to give back the keys it takes (see `ui::keys::LEFT`): Esc pressed (`escaped`: the front
    /// given back to the window that had it), or another program's window come in front (one of this thread's, a menu,
    /// it takes them again as that goes).
    pub fn keys_left(&mut self, hwnd: HWND, escaped: bool) {
        let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) else { return };
        // (A to-do written on it gives them back to any window come in front: the picker of its time among them, which
        // writes it in again as it goes.)
        widget.stop_todo(escaped);
        if escaped || !front_is_ours() {
            widget.stop_writing(escaped);
        }
    }

    /// A to-do written on the widget at `hwnd` again (the picker of its time put away: see `ui::todo::RESUME`).
    pub fn write_todo(&mut self, hwnd: HWND) {
        if let Some(widget) = self.list.iter_mut().find(|w| w.window.hwnd == hwnd) {
            widget.write_todo();
        }
    }

    /// Whether a note is written in where it is now (its widget taking the keys).
    pub fn writing(&self) -> bool {
        self.list.iter().any(|w| w.in_place.as_ref().is_some_and(|in_place| in_place.taking))
    }

    /// The menu of the note `number` stuck on the desktop on the widget at `hwnd` (its card pressed with the right
    /// button, or its menu's button while it is written in), up: its colours, it taken off the desktop, it taken away
    /// (see `ui::note::menu`). Its name, and what each row does. None where it is no such note there.
    pub fn note_menu(&self, hwnd: HWND, number: u64, lang: Lang, over_the_panel: bool) -> Option<(crate::ui::menu::Menu, String, Vec<crate::ui::note::Choice>)> {
        let widget = self.list.iter().find(|w| w.window.hwnd == hwnd)?;
        card_in(&widget.hits, widget.hits_from, number, 0.0)?;
        let written = widget.in_place.as_ref().filter(|in_place| in_place.number == number).map(|in_place| in_place.writing.colour);
        self.menu_of_note(number, written, lang, self.look_of(hwnd), over_the_panel)
    }

    /// The menu of the note numbered `number` (see `note_menu`), its colour as it is being written (`written`) or as it
    /// was kept, in `look`: stuck on the desktop, taken off it, or not, stuck there (its card on the panel's, or on a
    /// workbench of the desktop's).
    pub fn menu_of_note(&self, number: u64, written: Option<crate::notes::Colour>, lang: Lang, look: crate::ui::menu::Look, over_the_panel: bool) -> Option<(crate::ui::menu::Menu, String, Vec<crate::ui::note::Choice>)> {
        let name = crate::notes::named(number)?;
        let colour = written.or_else(|| crate::notes::shown().iter().find(|note| note.name == name).map(|note| note.colour)).unwrap_or_default();
        let (items, choices) = crate::ui::note::menu(lang, colour, true, self.note_stuck(&name));
        let open = crate::ui::menu::Menu::open(items, look, over_the_panel)?;
        Some((open, name, choices))
    }

    /// Whether the note `name` is stuck on the desktop.
    fn note_stuck(&self, name: &str) -> bool {
        let only = format!("{}{name}", view::STUCK_NOTE_LANE);
        self.list.iter().any(|widget| widget.content.holding(&only).is_some())
    }

    /// `choice` made in the menu of the note `name` stuck on the desktop (see `note_menu`): its colour, or it taken off the
    /// desktop, or taken away, what is written in it kept first (not kept, it stays, saying so: taken away, the
    /// Recycle Bin would have had the note as it was before).
    pub fn note_chose(&mut self, name: &str, choice: crate::ui::note::Choice) {
        use crate::ui::note::Choice;
        let index = self.list.iter().position(|w| w.in_place.as_ref().is_some_and(|in_place| in_place.name == name));
        // (Its writing menu's: done to the words, seen to as what is typed is.)
        if let Choice::Write(how) = choice {
            if let Some(index) = index {
                self.list[index].in_place.as_ref().unwrap().writing.write(how);
                let hwnd = self.list[index].window.hwnd;
                self.written(hwnd);
            }
            return;
        }
        // (The notes' Markdown shown as written, or as it reads: each note written in dressed again so, the stuck ones
        // drawn again.)
        if choice == Choice::Source {
            crate::ui::note::turn_source();
            let written: Vec<HWND> = self.list.iter().filter(|w| w.in_place.is_some()).map(|w| w.window.hwnd).collect();
            for hwnd in written {
                self.written(hwnd);
            }
            for widget in &mut self.list {
                widget.next_frame = Instant::now();
            }
            return;
        }
        let kept = index.is_none_or(|index| {
            let widget = &mut self.list[index];
            widget.next_frame = Instant::now();
            let in_place = widget.in_place.as_mut().unwrap();
            in_place.writing.keep(in_place.lang)
        });
        let written = index.and_then(|index| self.list[index].in_place.as_ref());
        let folder = written.map_or_else(crate::notes::folder, |in_place| crate::notes::folder_at(in_place.writing.folder.clone()));
        let named = written.and_then(|in_place| in_place.writing.name.clone()).unwrap_or_else(|| name.to_string());
        match choice {
            Choice::Colour(colour) => match index {
                Some(index) => {
                    let widget = &mut self.list[index];
                    let theme = widget.content.dressed(Theme::new(widget.drawn_look.0, widget.drawn_look.1));
                    widget.in_place.as_mut().unwrap().writing.recolour(&theme, colour);
                }
                None => {
                    let _ = folder.colour(&named, colour);
                    crate::notes::read_again();
                }
            },
            Choice::Unstick if kept => self.unstick_note(name),
            Choice::TakeAway if kept => {
                if folder.take_away(&named) {
                    crate::notes::read_again();
                    self.unstick_note(name);
                }
            }
            _ => {}
        }
    }

    /// Glance going: what is written in notes where they are kept (or, where it cannot be, put aside: see
    /// `Writing::put_aside`).
    pub fn keep_as_glance_goes(&mut self) {
        for in_place in self.list.iter_mut().filter_map(|w| w.in_place.as_mut()) {
            if !in_place.writing.keep(in_place.lang) {
                in_place.writing.put_aside();
                in_place.writing.unkept = false;
            }
        }
    }

    /// The music shown over the whole of the screen `monitor` (physical px) at `scale`.
    pub fn open_full(&mut self, monitor: RECT, scale: f32) {
        // (On the device of that screen's adapter: see `gfx::on_screen`.)
        match gfx::for_area(monitor).ok().and_then(|gfx| crate::fullscreen::FullMusic::open(gfx, monitor, scale)) {
            Some(full) => self.full = Some(full),
            None => note("music full screen: its window could not be made"),
        }
    }

    /// The music full screen, while it is up.
    pub fn full(&mut self) -> Option<&mut crate::fullscreen::FullMusic> {
        self.full.as_mut()
    }

    /// Whether `hwnd` is the music full screen's.
    pub fn full_owns(&self, hwnd: HWND) -> bool {
        self.full.as_ref().is_some_and(|full| full.owns(hwnd))
    }

    /// The music full screen left.
    pub fn leave_full(&mut self) {
        // (A drag under way given up first: left mid-drag, the track stayed dragged.)
        if let Some(full) = self.full.as_mut() {
            full.lost();
        }
        self.full = None;
    }

    /// What of the widget at `hwnd` is at `client` (physical px in its window), as a press there finds it.
    pub fn hit_at(&self, hwnd: HWND, client: POINT) -> Option<view::Hit> {
        let widget = self.list.iter().find(|w| w.window.hwnd == hwnd)?;
        let px = widget.contact.scale * widget.zoom;
        let margin = widget.margin_px();
        let (x, y) = ((client.x - margin) as f32 / px - widget.hits_from.0, (client.y - margin) as f32 / px - widget.hits_from.1);
        widget.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4)
    }

    /// The station's page the widget at `hwnd` shows (see `view::Tab`).
    pub fn page_of(&self, hwnd: HWND) -> view::Tab {
        self.list.iter().find(|w| w.window.hwnd == hwnd).map_or_else(view::Tab::default, |widget| widget.content.tab())
    }

    /// Where the pointer at `client` (physical px in the window of the widget at `hwnd`) is on a list of things that
    /// may be chosen together, one of them there (see `view::list_at`).
    pub fn list_at(&self, hwnd: HWND, client: POINT) -> Option<view::Hit> {
        let widget = self.list.iter().find(|w| w.window.hwnd == hwnd)?;
        let px = widget.contact.scale * widget.zoom;
        let margin = widget.margin_px();
        view::list_at(&widget.hits, (client.x - margin) as f32 / px - widget.hits_from.0, (client.y - margin) as f32 / px - widget.hits_from.1)
    }

    /// What may be looked at larger on the widget at `hwnd`, as it was just drawn (see `ui::look`): each such thing, and
    /// where its tile is on the screen (physical px).
    pub fn looks(&self, hwnd: HWND) -> Vec<(view::Hit, RECT)> {
        self.list.iter().find(|w| w.window.hwnd == hwnd).map_or_else(Vec::new, |widget| crate::panel::looks_in(&widget.hits_on_screen()))
    }

    /// Where the widget at `hwnd` is on the screen (physical px): its surface.
    pub fn rect_of(&self, hwnd: HWND) -> Option<RECT> {
        self.list.iter().find(|w| w.window.hwnd == hwnd).map(Widget::rect)
    }

    /// How a menu opened on the widget at `hwnd` looks: in its skin, light or dark and as frosted as it is drawn.
    pub fn look_of(&self, hwnd: HWND) -> crate::ui::menu::Look {
        self.list.iter().find(|w| w.window.hwnd == hwnd).map_or_else(crate::ui::menu::Look::system, Widget::menu_look)
    }

    /// The menu of the widget at `hwnd` (a right click on it), up (see `chose`).
    pub fn menu(&mut self, hwnd: HWND, lang: Lang, over_the_panel: bool) -> Option<crate::ui::menu::Menu> {
        self.list.iter().find(|w| w.window.hwnd == hwnd)?.menu(lang, over_the_panel)
    }

    /// The choice at `row` of the menu of the widget at `hwnd` made (see `menu`); what to do for the panel's side of
    /// it. (A widget gone meanwhile, nothing.)
    pub fn chose(&mut self, hwnd: HWND, row: usize) -> Option<WidgetChoice> {
        let index = self.list.iter().position(|w| w.window.hwnd == hwnd)?;
        let choice = *menu_choices(self.list[index].shows_full()).get(row)?;
        self.list[index].next_frame = Instant::now();
        match choice {
            WidgetChoice::Pin => self.list[index].pinned ^= true,
            WidgetChoice::ClickThrough => self.list[index].click_through ^= true,
            WidgetChoice::AboveOthers => {
                let widget = &mut self.list[index];
                widget.on_desktop ^= true;
                widget.window.set_on_desktop(widget.low());
            }
            WidgetChoice::Full => {
                let contact = self.list[index].contact;
                self.open_full(contact.monitor, contact.scale);
                return None;
            }
            WidgetChoice::Close => self.put_away_at(index),
            WidgetChoice::Settings => return Some(choice),
        }
        self.save();
        None
    }

    /// Follows the pointer, draws what is due, and says when next to.
    /// The thing on a station pressed on any of them, to be dragged off it (see `Widget::shelf_press`).
    pub fn shelf_pressed(&self) -> Option<view::Hit> {
        self.list.iter().find_map(|widget| widget.shelf_press.map(|press| press.hit))
    }

    /// Each drawn at its next turn, not after its rest (what it shows has changed).
    pub fn draw_soon(&mut self) {
        let now = Instant::now();
        for widget in &mut self.list {
            widget.next_frame = now;
        }
    }

    pub fn tick(&mut self, now: Instant, look: &Look, kept: &[WidgetAt], history: &[Arc<Sample>], seen: &Seen, controller: &crate::panel::Controller) -> Option<Duration> {
        // The music full screen, over everything while it is up: drawn at every frame.
        let full = self.full.as_mut().map(|full| full.tick(now));
        let due = self.tick_widgets(now, look, kept, history, seen, controller);
        match (full, due) {
            (Some(full), Some(due)) => Some(full.min(due)),
            (full, due) => full.or(due),
        }
    }

    fn tick_widgets(&mut self, now: Instant, look: &Look, kept: &[WidgetAt], history: &[Arc<Sample>], seen: &Seen, controller: &crate::panel::Controller) -> Option<Duration> {
        if !self.restored {
            // Restored once each screen they are on can be drawn on (else they would be lost, not made): the devices
            // asked for held as they are made, each on its own.
            let at = |kept: &WidgetAt| RECT { left: kept.at.0, top: kept.at.1, right: kept.at.0 + 1, bottom: kept.at.1 + 1 };
            let Ok(_devices) = kept.iter().map(|kept| gfx::for_area(at(kept))).collect::<windows::core::Result<Vec<_>>>() else { return None };
            self.restore(kept);
        }
        if self.list.is_empty() {
            self.watching = None;
            return None;
        }
        self.music_ground = look.prefs.music.ground;
        let playing = crate::ui::view::game_shown(history);
        // Each shown (the other programs' windows looked at only while one is: see `WindowWatch`).
        let shown: Vec<&Widget> = self.list.iter().filter(|w| !w.game_only || playing).collect();
        let any_shown = !shown.is_empty();
        if !any_shown {
            self.watching = None;
        } else if self.watching.is_none() {
            self.watching = Some(WindowWatch::start());
        }
        // Those kept on the desktop, just above it again wherever it went.
        if ORDER_CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            for widget in &self.list {
                widget.window.sink();
            }
        }
        // Where they are, for what changes behind them (see `WindowWatch`);
        // one of them moved, sized, come or gone where it meets another
        // changes what is behind that one too, as another program's would.
        self.live = look.live && look.skin.sees_backdrop();
        WATCHED_LIVE.store(self.live, std::sync::atomic::Ordering::Relaxed);
        // Each shown, with its shadow and the room for its buttons round it.
        let areas: Vec<(isize, RECT)> = shown
            .iter()
            .map(|w| {
                let (r, m) = (w.rect(), w.margin_px());
                (w.window.hwnd.0 as isize, RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m })
            })
            .collect();
        // What each depends on: all it took round it, kept for it to be
        // moved over.
        let watched: Vec<(isize, RECT)> = shown
            .iter()
            .zip(&areas)
            .map(|(w, (hwnd, area))| {
                let kept = w.behind.as_ref().map_or(*area, |b| union(*area, b.capture.rect));
                (*hwnd, w.pending.as_ref().map_or(kept, |request| union(kept, request.area)))
            })
            .collect();
        let others = |hwnd: isize, r: RECT| watched.iter().any(|(w, a)| *w != hwnd && r.left < a.right && a.left < r.right && r.top < a.bottom && a.top < r.bottom);
        let moved = areas.iter().filter(|area| !self.areas.contains(area)).chain(self.areas.iter().filter(|area| !areas.contains(area)));
        let moved_by_another = moved.into_iter().any(|(hwnd, r)| others(*hwnd, *r));
        *WIDGET_AREAS.lock().unwrap() = watched.iter().map(|(_, r)| *r).collect();
        if moved_by_another {
            behind_changed();
        }
        self.areas = areas;
        self.look_for_a_drop(now);
        let mut due = REST_FRAME;
        let mut gone = Vec::new();
        let mut back = Vec::new();
        let mut torn = Vec::new();
        for (index, widget) in self.list.iter_mut().enumerate() {
            // (Letting clicks through, but for the one on its button; and over its shadow.)
            let through = clicks_through(widget.click_through, widget.reached, cursor_position(), widget.window_rect(), widget.taking(), widget.drag.is_some());
            widget.shadowed = through && !(widget.click_through && !widget.reached);
            widget.window.set_click_through(through);
            let hidden = widget.game_only && !playing;
            // A skin that does not see the desktop (or what it shows is not in the skin): nothing of it kept, shown or
            // hidden.
            if !(look.skin.sees_backdrop() && widget.content.skinned(look.prefs.music.ground)) {
                widget.unsee();
            }
            if hidden {
                widget.window.hide();
                // (Nothing behind it looked at meanwhile: as it shows again, again.)
                widget.hidden();
                continue;
            }
            // A note on it written in where it is seen to, drawn as it is due (see `tend_writing`).
            if let Some(at) = widget.tend_writing(now) {
                widget.next_frame = widget.next_frame.min(at);
            }
            // A to-do written on it: its caret blinking.
            if let (_, Some(at)) = crate::ui::todo::field(widget.window.hwnd, now) {
                widget.next_frame = widget.next_frame.min(at);
            }
            // What is behind it, looked at as often as it is due, drawn only as it changes.
            let sees = look.skin.sees_backdrop() && widget.content.skinned(look.prefs.music.ground);
            if sees && widget.look_behind(self.live && sees) {
                widget.next_frame = now;
            }
            let behind_due = widget.backdrop_wait(self.live && sees);
            // (Windows behind it changed: put together again now, live.)
            if sees && widget.windows_moved_behind(now, self.live) {
                widget.next_frame = now;
            }
            // Drawn when it is due: in hand, flung or turning, at once; else
            // after its rest, not each time the panel draws a frame.
            let due_now = widget.drag.is_some() || widget.fling.is_some() || now >= widget.next_frame;
            if !due_now {
                due = due.min(widget.next_frame.saturating_duration_since(now));
                due = behind_due.map_or(due, |wait| due.min(wait));
                continue;
            }
            if widget.drag.is_some() {
                widget.follow(now);
            }
            if let Some(split) = widget.split.take() {
                torn.push((index, split));
            }
            match widget.frame(now, look, history, seen, controller) {
                Frame::Gone => gone.push(index),
                Frame::Back => back.push(index),
                Frame::Moving => {
                    widget.next_frame = now;
                    due = Duration::ZERO;
                }
                Frame::Rest | Frame::Waiting => {
                    // (The pointer over it: looked at often, so that what comes with it goes as soon as it leaves.)
                    // (Over its shadow, letting clicks through and so hearing nothing of the pointer: looked at as
                    // often, so that it takes the pointer as soon as that comes onto it.)
                    let rest = if widget.lingered.is_some() || widget.shadowed { WATCHED } else if widget.hover { HOVERED_FRAME } else { REST_FRAME };
                    // (Sooner, a note written in where it is due: its caret's blink, its keeping.)
                    let rest = widget.in_place.as_ref().and_then(|in_place| in_place.due(now)).map_or(rest, |at| rest.min(at.saturating_duration_since(now)));
                    // (And the preview's shortcut due to be said, or coming in.)
                    let rest = widget.preview_hint.due(now).map_or(rest, |at| rest.min(at.saturating_duration_since(now)));
                    widget.next_frame = now + rest;
                    due = due.min(rest);
                    // (What is behind looked at sooner, a frame of it having come: drawn only if it changed.)
                    let sees = look.skin.sees_backdrop() && widget.content.skinned(look.prefs.music.ground);
                    if let Some(wait) = widget.backdrop_wait(self.live && sees) {
                        due = due.min(wait);
                    }
                }
            }
        }
        for (index, (part, grab)) in torn {
            self.tear_part(index, part, grab);
            due = Duration::ZERO;
        }
        // Where they came to rest, kept.
        let rested = self.list.iter_mut().fold(false, |any, widget| std::mem::take(&mut widget.unsaved) | any);
        if rested && gone.is_empty() {
            self.save();
        }
        // (From the last: each taken out before the ones before it.)
        let mut ended: Vec<(usize, bool)> = gone.into_iter().map(|index| (index, false)).chain(back.into_iter().map(|index| (index, true))).collect();
        ended.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (index, back) in ended {
            if back {
                let mut widget = self.list.remove(index);
                note("widget: a note pulled off the panel gone back to it, not stuck");
                widget.gone();
                widget.window.destroy();
            } else {
                self.put_away_at(index);
            }
            self.save();
        }
        // (None shown, nothing is due of them: a game come, a sample wakes the thread, and they show.)
        any_shown.then_some(due)
    }

    /// The widget in hand held over another it may be merged into: since when, and, held there long enough, that
    /// one shown as where it would go (beside it near its right edge, else below it).
    fn look_for_a_drop(&mut self, now: Instant) {
        let held = self.list.iter().position(|w| matches!(w.drag, Some(Drag::Move { moved: true, .. })));
        let found = held.filter(|_| MERGING).zip(cursor_position()).and_then(|(guest, cursor)| {
            let target = self.list.iter().position(|w| {
                let r = w.rect();
                w.window.hwnd != self.list[guest].window.hwnd && w.shown_once && (r.left..r.right).contains(&cursor.x) && (r.top..r.bottom).contains(&cursor.y) && w.content.takes(&self.list[guest].content)
            })?;
            let (r, beside) = (self.list[target].rect(), match &self.list[target].content {
                Content::Merged(merged) => merged.beside,
                _ => false,
            });
            let beside = beside || (!matches!(self.list[target].content, Content::Merged(_)) && cursor.x as f32 > r.right as f32 - (r.right - r.left) as f32 * DROP_BESIDE);
            Some((self.list[guest].window.hwnd.0 as isize, self.list[target].window.hwnd.0 as isize, beside))
        });
        let dropping = found.map(|(guest, target, beside)| {
            let since = self.dropping.filter(|was| was.guest == guest && was.target == target).map_or(now, |was| was.since);
            Dropping { guest, target, beside, since, armed: now.saturating_duration_since(since) >= DROP_HOLD }
        });
        for widget in &mut self.list {
            let shown = dropping.filter(|drop| drop.armed && drop.target == widget.window.hwnd.0 as isize).map(|drop| drop.beside);
            if widget.drop_target != shown {
                widget.drop_target = shown;
                widget.next_frame = now;
            }
        }
        if dropping.is_some_and(|drop| drop.armed) && self.dropping.is_none_or(|was| !was.armed) {
            note("widget: held over another, to be merged into it");
        }
        self.dropping = dropping;
    }

    /// The widget in hand merged into the one it was held over: that one's look and size (no less than both need),
    /// the one in hand below it or beside it; its window gone.
    fn merge(&mut self, drop: Dropping) {
        let (Some(guest), Some(_)) = (self.list.iter().position(|w| w.window.hwnd.0 as isize == drop.guest), self.list.iter().position(|w| w.window.hwnd.0 as isize == drop.target)) else { return };
        let mut guest = self.list.remove(guest);
        crate::ui::window::let_go_of_capture(guest.window.hwnd);
        guest.hide_hint();
        if guest.tip.take().is_some() | std::mem::take(&mut guest.saying) {
            crate::ui::tip::hide();
        }
        let content = std::mem::replace(&mut guest.content, Content::Music(MusicShown::new(music::LEAST)));
        let guest_rect = guest.rect();
        guest.gone();
        guest.window.destroy();
        let Some(target) = self.list.iter_mut().find(|w| w.window.hwnd.0 as isize == drop.target) else { return };
        let size = (target.surface_size.0 * target.zoom, target.surface_size.1 * target.zoom);
        // Where it was let go, from the other's corner (DIPs), and where the other's parts were.
        let scale = target.contact.scale;
        let came_from = ((guest_rect.left - target.at.x) as f32 / scale, (guest_rect.top - target.at.y) as f32 / scale, (guest_rect.right - guest_rect.left) as f32 / scale, (guest_rect.bottom - guest_rect.top) as f32 / scale);
        let host = std::mem::replace(&mut target.content, Content::Music(MusicShown::new(music::LEAST)));
        let mut merged = match host {
            Content::Merged(merged) => merged,
            host => Box::new(Merged::new(vec![host], drop.beside, size)),
        };
        let mut from: Vec<Option<(f32, f32, f32, f32)>> = if merged.rects.is_empty() { vec![Some((0.0, 0.0, size.0, size.1))] } else { merged.rects.iter().copied().map(Some).collect() };
        merged.parts.push(content);
        from.push(Some(came_from));
        merged.moving = Some((Instant::now(), from));
        target.content = Content::Merged(merged);
        target.settle = Some((Instant::now(), target.surface_size));
        note(format!("widget: merged into another, {}", if drop.beside { "beside it" } else { "below it" }));
        target.drop_target = None;
        target.settle = None;
        target.next_frame = Instant::now();
    }

    /// The part at `part` of the merged widget at `index` torn out, in hand where it was taken (`grab`, DIPs from its
    /// corner): a widget of its own, as large as it was; the other parts as they were, the one left alone a widget
    /// of its own kind again.
    fn tear_part(&mut self, index: usize, part: usize, grab: (f32, f32)) {
        let Some(widget) = self.list.get_mut(index) else { return };
        if part == 0 {
            return;
        }
        let Some(content) = widget.take_part(part) else { return };
        widget.drag = None;
        widget.settle = Some((Instant::now(), widget.surface_size));
        widget.next_frame = Instant::now();
        crate::ui::window::let_go_of_capture(widget.window.hwnd);
        widget.window.lift(false);
        let Some(cursor) = cursor_position() else { return };
        let scale = widget.contact.scale;
        let grab = POINT { x: (grab.0 * scale) as i32, y: (grab.1 * scale) as i32 };
        let at = POINT { x: cursor.x - grab.x, y: cursor.y - grab.y };
        let Some(contact) = monitor_at(cursor) else { return };
        let Some(mut torn) = Widget::new(contact, at, content) else { return };
        note(format!("widget: a part torn out of a merged one, at ({}, {}), held {:?} from its corner", at.x, at.y, (grab.x, grab.y)));
        torn.drag = Some(Drag::Move { grab, trail: Vec::new(), torn: true, moved: true });
        torn.torn_from = Some((None, contact.monitor));
        torn.window.yield_to(self.settings);
        torn.window.lift(true);
        unsafe { SetCapture(torn.window.hwnd) };
        self.list.push(torn);
        self.save();
    }

    /// Puts the widget at `index` away: kept to be brought back, its window gone, and what one of its buttons said
    /// under the pointer gone with it (the pointer, no longer over it, would never take it away).
    fn put_away_at(&mut self, index: usize) {
        let widget = self.list.remove(index);
        if widget.tip.is_some() || widget.saying {
            crate::ui::tip::hide();
        }
        self.put_away.push(widget.kept());
        // (All it drew with goes with its window: see `life`.)
        let mut widget = widget;
        widget.gone();
        widget.window.destroy();
    }
}

/// What a widget's menu was used for.
#[derive(Clone, Copy)]
pub enum WidgetChoice {
    Pin,
    ClickThrough,
    AboveOthers,
    /// What plays over the whole screen (offered by a widget that shows it).
    Full,
    Close,
    Settings,
}

/// Where a widget's buttons are (see `Widget::rack`).
#[derive(Clone, Copy, PartialEq)]
enum Rack {
    Above,
    Under,
    Right,
    Left,
}

enum Released {
    Nothing,
    Changed,
    Closed,
    /// Its settings' button pressed, where it is on the screen (physical px).
    Settings(Option<RECT>),
    /// Its music asked to be shown over the whole of its screen.
    Full,
    /// One of its station's head's buttons pressed: its menu asked for.
    Menu(view::Hit),
}

enum Frame {
    Rest,
    Moving,
    /// Not drawn: waiting on the desktop behind it (see `CAPTURED`).
    Waiting,
    Gone,
    /// A note pulled off the panel come back to it (see `Pulled`): no longer on the desktop.
    Back,
}

/// What is behind a widget arranged anew, its frame awaited: when, for
/// which screen (see `Widget::generation`), and whether of somewhere else
/// than the last.
struct Request {
    /// What it stands for (physical px).
    area: RECT,
    generation: u64,
    moved: bool,
    /// When it was asked for.
    since: Instant,
}

/// Posted to a widget's window as a frame of what is behind it has come.
pub const CAPTURED: u32 = WM_APP + 12;

/// What a press at (`x`, `y`) on a widget `size` large (DIPs from its surface's corner) takes, among `hits` (as drawn
/// from `from`), `pinned` or not, `button` (one of its buttons there) or not: see `Takes`.
fn takes(hits: &[view::HitBox], from: (f32, f32), size: (f32, f32), pinned: bool, button: bool, (x, y): (f32, f32)) -> Takes {
    let (lx, ly) = (x - from.0, y - from.1);
    let under = |hit: &&view::HitBox| lx >= hit.0 && lx < hit.0 + hit.2 && ly >= hit.1 && ly < hit.1 + hit.3;
    // A thing on its station: dragged off it, or pressed (see `shelf_press`).
    if let Some(hit) = hits.iter().find(under).map(|hit| hit.4).filter(|hit| hit.drags()) {
        return Takes::Shelf(hit);
    }
    // The sound's track: slid along till let go (pinned or not: that is not moving it).
    if let Some(track) = hits.iter().filter(|hit| hit.4.slider().is_some()).find(under) {
        return Takes::Track(*track);
    }
    // A list's bar: its thumb dragged along it, as a track is.
    if let Some(bar) = hits.iter().filter(|hit| matches!(hit.4, view::Hit::Bar(..))).find(under) {
        return Takes::Bar(*bar);
    }
    // Pinned, it is neither moved nor sized: only its buttons and what it shows that takes a press (the keys of what
    // plays, a line of its lyrics, a drive's row) are pressed. (Its buttons alone were, and those did nothing pinned.)
    let on_content = hits.iter().filter(|hit| hit.4.takes_press()).any(|hit| under(&hit));
    if pinned && !button && !on_content {
        return Takes::Nothing;
    }
    if button || pinned {
        return Takes::Pressed;
    }
    // (Its edges over what it shows there: a note stuck on the desktop is its card to its edges.)
    let edges = edges_at(x, y, size.0, size.1);
    if edges.0 || edges.1 || edges.2 || edges.3 {
        Takes::Edges(edges)
    } else {
        Takes::Whole
    }
}

/// The pointer's shape over a widget where a press takes `takes`, `over` one of its buttons or not, over `hit` of what
/// it shows (that takes a press), on the menu's button of a note written in where it is (`note_button`) or not: as
/// a press there acts (sized by its edges, the hand on what is pressed, the I-beam over a note's words).
fn pointer_for(takes: &Takes, over: bool, hit: Option<view::Hit>, note_button: bool) -> windows::core::PCWSTR {
    use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_HAND, IDC_IBEAM};
    match takes {
        _ if note_button => IDC_HAND,
        Takes::Nothing => IDC_ARROW,
        Takes::Edges((left, right, top, bottom)) => crate::panel::sizing(*left, *right, *top, *bottom),
        _ if over => IDC_HAND,
        _ if matches!(hit, Some(view::Hit::Words(_))) => IDC_IBEAM,
        _ if hit.is_some() => IDC_HAND,
        _ => IDC_ARROW,
    }
}

/// What a press on a widget takes (see `Widget::takes_at`): a thing on its station (dragged off it, or pressed), the
/// sound's track (slid along), a list's bar (its thumb dragged along it), nothing (pinned, off its buttons and what it shows that takes a press), one of its
/// buttons or, pinned, what it shows that takes a press (acted on as it is let go), its edges or a corner (sized by
/// them), or the whole of it (moved, or what it shows there pressed as it is let go where it was).
enum Takes {
    Shelf(view::Hit),
    Track(view::HitBox),
    Bar(view::HitBox),
    Nothing,
    Pressed,
    Edges((bool, bool, bool, bool)),
    Whole,
}

impl Widget {
    /// Its part at `part` (merged) taken out: as large as it was drawn (what plays), the others as they were, shorter
    /// by it along the way they go; the one left alone a widget of its own kind again. What was taken.
    fn take_part(&mut self, part: usize) -> Option<Content> {
        let Content::Merged(merged) = &mut self.content else { return None };
        if part >= merged.parts.len() {
            return None;
        }
        let (_, _, pw, ph) = merged.rects.get(part).copied().unwrap_or((0.0, 0.0, music::LEAST.0, music::LEAST.1));
        let mut content = merged.parts.remove(part);
        let removed = merged.rects.remove(part);
        merged.moving = Some((Instant::now(), merged.rects.iter().copied().map(Some).collect()));
        if let Content::Music(shown) = &mut content {
            shown.size = (pw.max(music::LEAST.0), ph.max(music::LEAST.1));
        }
        if merged.beside {
            merged.size.0 -= removed.2;
        } else {
            merged.size.1 -= removed.3;
        }
        if merged.parts.len() == 1 {
            let rest = merged.rects.first().copied();
            let mut last = merged.parts.pop().unwrap();
            if let (Content::Music(shown), Some((_, _, w, h))) = (&mut last, rest) {
                shown.size = (w.max(music::LEAST.0), h.max(music::LEAST.1));
            }
            self.content = last;
        }
        Some(content)
    }

    /// The note `number` stuck on the desktop on it written in where it is (see `InPlace`), taking the keys: the caret
    /// at its end (`at_end`: its head or its foot clicked), else where it was, or where its words are pressed next.
    /// Whether it is.
    fn write_note(&mut self, number: u64, at_end: bool) -> bool {
        // (Another note on it written in, merged with this one: given back, kept, first.)
        if self.in_place.as_ref().is_some_and(|in_place| in_place.number != number) {
            self.stop_writing(false);
            self.in_place = None;
        }
        if self.in_place.is_none() {
            let px = self.contact.scale * self.zoom;
            let Some(card) = card_in(&self.hits, self.hits_from, number, self.margin_px() as f32 / px) else { return false };
            let Some(name) = crate::notes::named(number) else { return false };
            let theme = self.content.dressed(Theme::new(self.drawn_look.0, self.drawn_look.1));
            let Some(writing) = crate::ui::note::Writing::open(self.window.hwnd, words_rect(card, px), &crate::notes::folder(), Some(name.clone()), &theme, crate::ui::note::WORDS * px) else { return false };
            // (After its last words: not on the empty line the line break ending its file leaves.)
            if at_end {
                let end = writing.editor.text().trim_end_matches('\n').encode_utf16().count() as i32;
                writing.editor.select((end, end));
            }
            // (Its words scrolled where its card had them: written in, they stay where they were read.)
            writing.editor.scroll_to((self.content.scrolls().at(view::List::Words) * px).round() as i32);
            view::written_in_place(&name, true);
            let now = Instant::now();
            let chosen = writing.editor.selection();
            let hwnd = self.window.hwnd;
            self.in_place = Some(InPlace { writing, number, name, hwnd, lang: crate::update::language(), taking: false, before: HWND::default(), clicks: Default::default(), selecting: false, menu_pressed: false, keep_at: None, read_at: now + READ_OUTSIDE, blink_from: now, chosen, styled: (px, theme.skin, theme.dark), card });
        }
        self.take_keys();
        true
    }

    /// A to-do written on it: its field written in, it taking the keys (see `ui::todo::begin`). (Not while a note on it
    /// takes them.)
    fn write_todo(&mut self) {
        if self.in_place.as_ref().is_some_and(|in_place| in_place.taking) {
            return;
        }
        crate::ui::todo::begin(&self.window);
        self.next_frame = Instant::now();
    }

    /// A to-do written on it no more: the keys given back (the front too, where `escaped`).
    fn stop_todo(&mut self, escaped: bool) {
        if crate::ui::todo::writing(self.window.hwnd) {
            crate::ui::todo::end(&self.window, escaped);
            self.next_frame = Instant::now();
        }
    }

    /// Its note written in where it is taking the keys (see `ui::keys`): it comes to the front with them, the caret
    /// shown at once.
    fn take_keys(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let Some(in_place) = self.in_place.as_mut() else { return };
        let now = Instant::now();
        (in_place.blink_from, self.next_frame) = (now, now);
        if in_place.taking {
            return;
        }
        in_place.taking = true;
        in_place.before = unsafe { GetForegroundWindow() };
        crate::ui::keys::take(self.window.hwnd, in_place.writing.answer());
        // (Its window with the focus already, no longer taking the keys since: its words told they have it, as coming
        // to the front would tell them.)
        let focused = unsafe { GetFocus() } == self.window.hwnd;
        self.window.take_keys(true);
        if focused {
            in_place.writing.focus(true);
        }
    }

    /// The keys its note written in where it is takes given back (see `InPlace`): what was written kept (not kept, tried
    /// again a while after, saying so), the front given back to the window that had it where `escaped` (Esc pressed).
    fn stop_writing(&mut self, escaped: bool) {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
        let Some(in_place) = self.in_place.as_mut().filter(|in_place| in_place.taking) else { return };
        in_place.taking = false;
        (in_place.selecting, in_place.menu_pressed) = (false, false);
        crate::ui::keys::give_back(self.window.hwnd);
        // (What the input method is composing written, the caret hidden.)
        in_place.writing.focus(false);
        // Its card's words left where the box they were written in had them.
        let px = self.contact.scale * self.zoom;
        if let Some(readings) = self.content.readings_mut() {
            readings.scrolls.set(view::List::Words, in_place.writing.editor.scrolled() as f32 / px);
        }
        let _ = in_place.writing.written();
        let now = Instant::now();
        in_place.keep_soon(now);
        let before = in_place.before;
        self.window.take_keys(false);
        if escaped && !before.is_invalid() && unsafe { IsWindow(Some(before)) }.as_bool() {
            let _ = unsafe { SetForegroundWindow(before) };
        }
        self.next_frame = now;
    }

    /// Its note written in where it is seen to as time goes (see `InPlace`): the keys given back where another
    /// program's window came in front unheard (as a menu of this thread's went), what was written kept when it is due,
    /// its file looked at for a change made outside. Given back and kept, let go once the notes read show it as
    /// written. When it is next due.
    fn tend_writing(&mut self, now: Instant) -> Option<Instant> {
        if self.in_place.as_ref().is_some_and(|in_place| in_place.taking) && !front_is_ours() {
            self.stop_writing(false);
        }
        let in_place = self.in_place.as_mut()?;
        // (Kept on a thread of its own: see `InPlace::keep_soon`.)
        if in_place.keep_at.is_some_and(|at| now >= at) && !in_place.writing.answering() {
            in_place.keep_soon(now);
            self.next_frame = now;
        }
        // Its file read (on a thread of its own, one reading at a time), and what was read taken in.
        if in_place.taking && now >= in_place.read_at {
            in_place.read_at = now + READ_OUTSIDE;
            if in_place.writing.read_outside() {
                self.next_frame = now;
            }
        }
        if !in_place.taking && in_place.keep_at.is_none() && in_place.writing.read_as_written() {
            self.in_place = None;
            self.next_frame = now;
            return None;
        }
        in_place.due(now)
    }

    /// The pointer's `msg` for its note written in where it is, or to be (see `Widgets::write`), its menu asked for by
    /// its button told in `menu_asked`. Whether it was theirs.
    fn write(&mut self, msg: &windows::Win32::UI::WindowsAndMessaging::MSG, menu_asked: &mut Option<u64>) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP};
        let px = self.contact.scale * self.zoom;
        let hwnd = self.window.hwnd;
        let mut client = POINT { x: (msg.lParam.0 & 0xFFFF) as i16 as i32, y: ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32 };
        // (The wheel's, on the screen.)
        if msg.message == WM_MOUSEWHEEL {
            let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client) };
        }
        let hand_on = |message: u32| unsafe { SendMessageW(hwnd, message, Some(msg.wParam), Some(msg.lParam)) };
        match msg.message {
            WM_LBUTTONDOWN => {
                let margin = self.margin_px();
                let (x, y) = ((client.x - margin) as f32 / px, (client.y - margin) as f32 / px);
                // (Its own buttons, the widget's.)
                if self.button_at(x, y).is_some() {
                    return false;
                }
                if !self.in_place.as_ref().is_some_and(|in_place| in_place.taking) {
                    let (lx, ly) = (x - self.hits_from.0, y - self.hits_from.1);
                    let Some(view::Hit::Words(number)) = self.hits.iter().find(|(hx, hy, w, h, _)| lx >= *hx && lx < hx + w && ly >= *hy && ly < hy + h).map(|hit| hit.4) else { return false };
                    if !self.write_note(number, false) {
                        return false;
                    }
                }
                let Some(in_place) = self.in_place.as_mut() else { return false };
                if in_place.on_menu_button(client, px) {
                    in_place.menu_pressed = true;
                    return true;
                }
                // (Pressed off its words, and off the link being written there: that left, the press the widget's.)
                if !in_place.writing.holds(client) {
                    in_place.writing.pressed_off();
                    return false;
                }
                in_place.selecting = true;
                let pressed = in_place.clicks.press(client);
                hand_on(pressed);
                true
            }
            WM_LBUTTONUP | WM_MOUSEMOVE => {
                let Some(in_place) = self.in_place.as_mut().filter(|in_place| in_place.taking) else { return false };
                if msg.message == WM_LBUTTONUP && std::mem::take(&mut in_place.menu_pressed) {
                    if in_place.on_menu_button(client, px) {
                        *menu_asked = Some(in_place.number);
                    }
                    return true;
                }
                if !in_place.selecting {
                    return false;
                }
                in_place.selecting &= msg.message != WM_LBUTTONUP;
                hand_on(msg.message);
                true
            }
            // (The right button's on its words theirs: the writing menu. Pressed elsewhere, a link being written there
            // left.)
            WM_MOUSEWHEEL | WM_RBUTTONDOWN | WM_RBUTTONUP if self.in_place.as_ref().is_some_and(|in_place| in_place.taking && in_place.writing.holds(client)) => {
                hand_on(msg.message);
                true
            }
            WM_RBUTTONDOWN => {
                if let Some(in_place) = self.in_place.as_ref().filter(|in_place| in_place.taking) {
                    in_place.writing.pressed_off();
                }
                false
            }
            _ => false,
        }
    }

    /// `page` opened beside it, on the side of its screen with more room, drawn as it is.
    fn open_beside(&self, page: crate::ui::beside::Page) {
        use crate::ui::beside::{Opening, Side};
        let by = self.rect();
        let screen = self.contact.monitor;
        let side = if (by.left + by.right) / 2 > (screen.left + screen.right) / 2 { Side::Left } else { Side::Right };
        let (skin, dark) = self.drawn_look;
        crate::ui::beside::open(Opening { page, by, side, skin, dark, folder: None, owner: self.window.hwnd.0 as isize });
    }

    fn new(contact: Contact, at: POINT, content: Content) -> Option<Self> {
        // (On the device of its screen's adapter: see `gfx::on_screen`.)
        let gfx = gfx::for_area(contact.monitor).ok()?;
        let window = Window::new().ok()?;
        window.under_the_panel();
        let surface = Surface::new(&gfx, window.hwnd).ok();
        Some(Widget {
            window,
            gfx,
            surface,
            content,
            showing: None,
            contact,
            at,
            room: None,
            settle: None,
            pulled: None,
            lift: Spring::at(0.0, LIFTING.0, LIFTING.1),
            lean: Spring::at(0.0, LEANING.0, LEANING.1),
            about: (0.0, 0.0),
            keeps_to: (false, false),
            drag: None,
            fling: None,
            opacity: 1.0,
            stuck: None,
            also: None,
            along: None,
            margin: Theme::new(Skin::Paper, false).margin,
            anchor: None,
            pinned: false,
            click_through: false,
            torn_from: None,
            game_only: false,
            on_desktop: false,
            was_sized: false,
            hint: None,
            backdrop: None,
            arranged: None,
            arranged_on: None,
            retake: false,
            pending: None,
            look_again: None,
            behind_changes: 0,
            replace: None,
            generation: 0,
            unsaved: false,
            failed: None,
            shown_once: false,
            frost_at: None,
            frost_shown: None,
            tone_due: false,
            frame_took: gfx::Took::default(),
            drop_target: None,
            splitting: None,
            split: None,
            hover: false,
            hit_hover: None,
            preview_hint: view::Hint::default(),
            saying: false,
            thumbs: crate::ui::thumb::Dots::new(),
            lingered: None,
            shadowed: false,
            settings_from: None,
            reached: false,
            tip: None,
            hits: Vec::new(),
            hits_from: (0.0, 0.0),
            shelf_press: None,
            lifting: None,
            swipes: None,
            drawn_look: (Skin::Paper, false),
            behind: None,
            next_frame: Instant::now(),
            surface_size: (0.0, 0.0),
            shown: (0.0, 0.0),
            zoom: 1.0,
            last_frame: Instant::now(),
            in_place: None,
        })
    }

    /// A widget as the settings keep it, on the screen nearest where it was.
    fn restore(kept: &WidgetAt) -> Option<Self> {
        let at = POINT { x: kept.at.0, y: kept.at.1 };
        let contact = monitor_at(at)?;
        let one = |layout: &crate::settings::PanelLayout| -> Option<Content> {
            Some(match layout.form.as_str() {
                MUSIC_FORM => Content::Music(MusicShown::new(kept.size.unwrap_or(music::LEAST))),
                form => {
                    let (only, form) = only_of(form);
                    let choice = Choice::from_kept(&crate::settings::PanelLayout { form, ..layout.clone() })?;
                    Content::Readings(Readings { workbench: layout.workbench, only, ..Readings::new(choice) })
                }
            })
        };
        let content = match &kept.parts {
            Some(parts) if parts.len() > 1 => {
                let parts = parts.iter().map(one).collect::<Option<Vec<_>>>()?;
                // (As large as it was kept: grown already, as it was merged.)
                Content::Merged(Box::new(Merged { grown: kept.size.is_some(), ..Merged::new(parts, kept.beside, kept.size.unwrap_or(music::LEAST)) }))
            }
            _ => one(&kept.layout)?,
        };
        let mut widget = Widget::new(contact, at, content)?;
        widget.pinned = kept.pinned;
        widget.click_through = kept.click_through;
        widget.game_only = kept.game_only;
        widget.on_desktop = kept.on_desktop;
        widget.window.set_on_desktop(widget.low());
        widget.stuck = kept.stuck.map(Side::from_kept);
        widget.also = kept.stuck.and(kept.corner).map(Side::from_kept);
        Some(widget)
    }

    fn kept(&self) -> WidgetAt {
        let layout_of = |content: &Content| match content {
            Content::Readings(readings) => readings.kept(),
            _ => crate::settings::PanelLayout { form: MUSIC_FORM.into(), scale: 1.0, extra: (0.0, 0.0), workbench: false },
        };
        let (layout, size, parts, beside) = match &self.content {
            Content::Readings(readings) => (readings.kept(), None, None, false),
            Content::Music(shown) => (layout_of(&self.content), Some(shown.size), None, false),
            Content::Merged(merged) => (layout_of(&merged.parts[0]), Some(merged.size), Some(merged.parts.iter().map(layout_of).collect()), merged.beside),
        };
        WidgetAt {
            at: (self.at.x, self.at.y),
            layout,
            pinned: self.pinned,
            click_through: self.click_through,
            game_only: self.game_only,
            on_desktop: self.on_desktop,
            stuck: self.stuck.map(Side::kept),
            corner: self.also.map(Side::kept),
            size,
            parts,
            beside,
            before: None,
        }
    }

    /// Where each thing drawn is on the screen (physical px), as it was just drawn.
    fn hits_on_screen(&self) -> Vec<(RECT, view::Hit)> {
        let mut window = RECT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.window.hwnd, &mut window) }.is_err() {
            return Vec::new();
        }
        let px = self.contact.scale * self.zoom;
        let margin = self.margin_px();
        let screen = |at: f32, from: f32, origin: i32| origin + margin + ((at + from) * px).round() as i32;
        self.hits
            .iter()
            .map(|(x, y, w, h, hit)| {
                let rect = RECT { left: screen(*x, self.hits_from.0, window.left), top: screen(*y, self.hits_from.1, window.top), right: screen(x + w, self.hits_from.0, window.left), bottom: screen(y + h, self.hits_from.1, window.top) };
                (rect, *hit)
            })
            .collect()
    }

    /// Where it shows a station, as it was just drawn, told to the station (see `station::run::shows_at`); where what
    /// may be looked at larger is, told to the look at one of them up (see `ui::look::tiles`).
    fn show_stations(&self) {
        let on_screen = self.hits_on_screen();
        crate::station::run::shows_at(self.window.hwnd, on_screen.iter().filter_map(|(rect, hit)| Some((*rect, hit.lands()?))).collect());
        crate::ui::look::tiles(self.window.hwnd, crate::panel::looks_in(&on_screen).into_iter().map(|(hit, rect)| (hit.key(), rect)).collect());
    }

    /// What the pointer is taken to be over: while a look at a thing on it is up, that thing (another is had by the
    /// arrows or a click, never by the pointer passing over it).
    fn hover_shown(&self) -> Option<view::Hit> {
        match crate::ui::look::shown_for(self.window.hwnd) {
            Some(key) => self.hits.iter().map(|hit| hit.4).find(|hit| hit.looks() && hit.key() == key),
            None => self.hit_hover,
        }
    }

    /// The desktop taken behind it, from `taken` on (`moved`: elsewhere
    /// than the last was).
    fn took(&mut self, capture: Option<Capture>, moved: bool, taken: Instant, px: f32) {
        let now = taken;
        // (The same desktop taken again where the last was, nothing waiting on it: nothing to draw anew.)
        let same = matches!((&capture, &self.behind), (Some(capture), Some(behind)) if capture.digest == behind.capture.digest && capture.rect == behind.capture.rect && !moved);
        if !(same && self.replace.is_none() && self.frost_at.is_none()) {
            self.behind_changes += 1;
        }
        // None could be taken (the lock screen): what it had stands as it
        // was, as old as it was, and is taken again once it has rested.
        let Some(capture) = capture else {
            self.failed = Some(Instant::now());
            self.retake |= self.behind.is_some();
            return;
        };
        self.failed = None;
        // One to be replaced, replaced by this.
        self.replace = None;
        self.frost_at = None;
        match (Some(capture), &mut self.behind) {
            // The same desktop as before, taken where the last was: the last
            // stands.
            (Some(capture), Some(behind)) if capture.digest == behind.capture.digest && capture.rect == behind.capture.rect && !moved => behind.taken = now,
            // Refreshed in place, the theme holds (as the panel's does); a
            // first or a new place's sets it.
            (Some(capture), behind) => {
                let tone = match behind {
                    Some(behind) if !moved => behind.tone,
                    _ => capture.luminance(self.rect(), px),
                };
                // (The one before's, or what it had from its own before, if it was not drawn.)
                let former = self.behind.take().and_then(|behind| behind.bitmap.map(|(bitmap, _)| bitmap).or(behind.former));
                self.behind = Some(Behind { capture, bitmap: None, former, tone, taken: now });
            }
            (None, _) => {}
        }
    }

    /// On `contact`'s screen now: a capture asked for on another is of the
    /// screen as it was, and let go of.
    fn moved_to(&mut self, contact: Contact) {
        if contact != self.contact {
            self.contact = contact;
            self.generation += 1;
        }
    }

    /// How often what is behind is taken anew as it changes, with live
    /// refraction (without, it is taken only as it is arranged anew: see
    /// `collect`).
    fn backdrop_every(live: bool) -> Duration {
        if live { LIVE_INTERVAL } else { RETAKE_EVERY }
    }

    /// A frame of what is behind, read, put in place (if it is of its
    /// screen as it is); else one wanted, if one is due: the one awaited
    /// (arranged anew) at once, else as often as it is taken (see
    /// `backdrop_every`).
    fn collect(&mut self, live: bool, px: f32) {
        let Some(scene) = &self.backdrop else { return };
        let Some(capture) = scene.ready() else {
            // Looked for again a while, and none came (none being read either, which is waited for): what was taken
            // stands, and the scene rests.
            if self.look_again.is_some_and(|till| Instant::now() >= till) && !scene.reading() {
                self.look_again = None;
                if let Some(scene) = &mut self.backdrop {
                    scene.rest();
                }
                return;
            }
            // (Without live refraction, only the one awaited. With it, each as it comes where they come no oftener than
            // they are taken, else as often as they are.)
            let again = live && self.behind.as_ref().is_some_and(|behind| scene.paced() || behind.taken.elapsed() >= Self::backdrop_every(live));
            if self.pending.is_some() || self.behind.is_none() || again {
                scene.want();
            }
            // Asked for a while ago, and none come (nor being read): arranged anew. A frame comes only as what the
            // scene shows changes, and one passed over (put together before the desktop was in it, as it can be just
            // after Glance starts) is followed by none while nothing behind changes: the widget, waiting for its
            // first, would never show.
            if self.pending.as_ref().is_some_and(|request| request.since.elapsed() >= ASK_AGAIN) && !scene.reading() {
                note_sparingly("ask again", || "widget: the desktop behind asked for again (none came)".into());
                if let Some(scene) = &mut self.backdrop {
                    scene.rest();
                }
                self.pending = None;
                self.look_again = None;
                self.retake |= self.behind.is_some();
            }
            return;
        };
        let now = Instant::now();
        // Without live refraction, what is behind is as it was when looked at, till it is looked at again (see
        // `frame`): the scene rests meanwhile. Not at once: a frame put together while the compositor had put in
        // only some of the windows behind is followed by one with all of them (a frame comes as what is there
        // changes), which is looked for a while, and taken in its place if it comes; then it rests.
        if !live {
            if self.look_again.take().is_some() {
                if let Some(scene) = &mut self.backdrop {
                    scene.rest();
                }
            } else {
                self.look_again = Some(now + LOOK_AGAIN);
                scene.want();
            }
        }
        match self.pending.take() {
            Some(request) if request.generation != self.generation => self.retake |= self.behind.is_some(),
            Some(request) => self.took(Some(capture), request.moved, now, px),
            None => self.took(Some(capture), false, now, px),
        }
        // Live, its frames paced: the next wanted now, read as it comes.
        if let Some(scene) = self.backdrop.as_ref().filter(|scene| live && scene.paced() && self.behind.is_some()) {
            scene.want();
        }
    }

    /// What is behind looked at between its frames (see `collect`): a frame of it read, taken; one due, wanted (read
    /// on another thread, which tells the thread that draws as it has been). Whether what it shows changed, or what
    /// is to be done about it (to be drawn at once): a live backdrop the same as before is not drawn again.
    /// Whether other windows changed behind it since they were put together there (see `Scene::arrange`): with live
    /// refraction, as they change; without, once they have settled. Not while it is in hand.
    fn windows_moved_behind(&self, now: Instant, live: bool) -> bool {
        self.drag.is_none() && self.fling.is_none() && windows_changed().zip(self.arranged).is_some_and(|(changed, arranged)| changed > arranged && (live || now.saturating_duration_since(changed) >= SETTLED))
    }

    fn look_behind(&mut self, live: bool) -> bool {
        if self.backdrop.is_none() {
            return false;
        }
        let before = (self.behind_changes, self.retake, self.pending.is_some(), self.look_again.is_some());
        self.collect(live, self.contact.scale * self.zoom);
        (self.behind_changes, self.retake, self.pending.is_some(), self.look_again.is_some()) != before
    }

    /// With live refraction, how long till a frame of what is behind that
    /// has come is due to be taken (none: none has come; without, none is
    /// taken but the one awaited, which wakes it as it is read).
    fn backdrop_wait(&self, live: bool) -> Option<Duration> {
        // (Without it, while another frame is looked for: till it is given up, or a while, one being read.)
        if let Some(till) = self.look_again.filter(|_| !live && self.backdrop.is_some()) {
            return Some(till.saturating_duration_since(Instant::now()).max(LOOK_AGAIN_STEP));
        }
        // (Not while one is read: it tells the thread that draws as it has been. Nor where each is taken as it comes:
        // see `collect`.)
        self.backdrop.as_ref().filter(|scene| live && !scene.paced() && scene.fresh() && !scene.reading())?;
        Some(self.behind.as_ref().map_or(Duration::ZERO, |behind| Self::backdrop_every(live).saturating_sub(behind.taken.elapsed())))
    }

    /// What is taken of the desktop behind it: round it by half its width
    /// across and half its height up and down (room to be moved in, and for
    /// the blur past its edges), on every screen it is on (across two, the
    /// half on the other has what is behind it too).
    fn backdrop_area(&self) -> RECT {
        let r = self.rect();
        let least = (64.0 * self.contact.scale) as i32;
        let (across, down) = (((r.right - r.left) / 2).max(least), ((r.bottom - r.top) / 2).max(least));
        clip(RECT { left: r.left - across, top: r.top - down, right: r.right + across, bottom: r.bottom + down }, crate::mouse::desktop())
    }

    /// Its surface on the screen (physical px).
    fn rect(&self) -> RECT {
        let px = self.contact.scale * self.zoom;
        let (w, h) = ((self.surface_size.0 * px).round() as i32, (self.surface_size.1 * px).round() as i32);
        RECT { left: self.at.x, top: self.at.y, right: self.at.x + w, bottom: self.at.y + h }
    }

    /// Its buttons' centres, from its surface's corner (DIPs, its own): in
    /// a row above its right end, close last.
    /// Whether its buttons are under it: where its screen has no room for them above it.
    fn buttons_under(&self) -> bool {
        let reach = ((BUTTON + 2.0 * BUTTON_INSET) * self.contact.scale * self.zoom) as i32;
        self.at.y - reach < self.contact.work.top
    }

    /// Where each of its buttons is, whether it shows or not: in a row
    /// ending at its right edge.
    fn buttons(&self) -> Vec<(Point, Button)> {
        let (w, h) = self.surface_size;
        let r = BUTTON / 2.0;
        let row = self.content.buttons();
        let count = row.len();
        let step = BUTTON + buttons::GAP;
        match self.rack() {
            Rack::Above | Rack::Under => {
                let y = if self.rack() == Rack::Under { h + BUTTON_INSET + r } else { -BUTTON_INSET - r };
                row.iter().enumerate().map(|(place, button)| (Point { x: w - r - (count - 1 - place) as f32 * step, y }, *button)).collect()
            }
            Rack::Right | Rack::Left => {
                let x = if self.rack() == Rack::Right { w + BUTTON_INSET + r } else { -BUTTON_INSET - r };
                row.iter().enumerate().map(|(place, button)| (Point { x, y: r + place as f32 * step }, *button)).collect()
            }
        }
    }

    /// Where its buttons are: in a row over its right end (under it, where its screen has no room above it); or,
    /// narrower than the row and tall enough, in a column down its right side (its left, where its screen has no
    /// room on the right).
    fn rack(&self) -> Rack {
        let count = self.content.buttons().len() as f32;
        let row = count * BUTTON + (count - 1.0) * buttons::GAP;
        let (w, h) = self.surface_size;
        if w >= row || h < row {
            return if self.buttons_under() { Rack::Under } else { Rack::Above };
        }
        let reach = ((BUTTON + 2.0 * BUTTON_INSET) * self.contact.scale * self.zoom) as i32;
        if self.rect().right + reach <= self.contact.work.right { Rack::Right } else { Rack::Left }
    }

    /// The buttons that show now: all of them, the pointer over it; while
    /// it lets clicks through, the one that ends that, once the pointer has
    /// stayed over it a while. And its settings' button while the settings asked
    /// from it are open, the pointer over it or not: they pour back into it.
    fn offered(&self) -> Vec<(Point, Button)> {
        let all = self.buttons();
        let mut offered = match (self.hover, self.click_through) {
            (false, _) => Vec::new(),
            (true, false) => all.clone(),
            (true, true) if self.lingered.is_some_and(|since| since.elapsed() >= buttons::LINGER) => all.iter().copied().filter(|(_, button)| *button == Button::Through).collect(),
            (true, true) => Vec::new(),
        };
        if self.settings_lit().is_some() && !offered.iter().any(|(_, button)| *button == Button::Settings) {
            offered.extend(all.into_iter().filter(|(_, button)| *button == Button::Settings));
        }
        offered
    }

    /// Where its button `button` is on the screen (physical px), whether it shows or not.
    fn button_rect(&self, button: Button) -> Option<RECT> {
        let px = self.contact.scale * self.zoom;
        let (middle, _) = self.buttons().into_iter().find(|(_, one)| *one == button)?;
        let (x, y, half) = (self.at.x as f32 + middle.x * px, self.at.y as f32 + middle.y * px, BUTTON / 2.0 * px);
        Some(RECT { left: (x - half).round() as i32, top: (y - half).round() as i32, right: (x + half).round() as i32, bottom: (y + half).round() as i32 })
    }

    /// Where its button `button` is on the screen (physical px), while it is offered.
    fn button_on_screen(&self, button: Button) -> Option<RECT> {
        self.offered().iter().any(|(_, offered)| *offered == button).then(|| self.button_rect(button)).flatten()
    }

    /// The settings, open, asked from its settings' button: told where that is now, as it moves with it (or is gone),
    /// so they pour back into it there and it shows so there.
    fn follow_settings(&mut self) {
        let from = crate::ui::settings_window::asked_from();
        let button = self.button_rect(Button::Settings);
        self.settings_from = match from {
            Some(from) if Some(from) == button => button,
            Some(from) if Some(from) == self.settings_from => {
                crate::ui::settings_window::asked_from_moved(from, button);
                button
            }
            _ => None,
        };
    }

    /// Its settings' button lit, and how large (1 as it is): the settings asked from it open, or just poured back into
    /// it (it bounces: see `settings_window::bounce`).
    fn settings_lit(&self) -> Option<f32> {
        if self.settings_asked_here() { Some(1.0) } else { crate::ui::settings_window::bounce(self.button_rect(Button::Settings)) }
    }

    /// Whether the settings, open, were asked from its settings' button, where it is now.
    fn settings_asked_here(&self) -> bool {
        crate::ui::settings_window::asked_from().is_some_and(|from| self.button_rect(Button::Settings) == Some(from))
    }

    fn button_at(&self, x: f32, y: f32) -> Option<Button> {
        self.offered().into_iter().find(|(c, _)| (c.x - x).hypot(c.y - y) <= BUTTON / 2.0 + 2.0).map(|(_, b)| b)
    }

    /// What a press at (`x`, `y`) (DIPs from its surface's corner) takes (see `Takes`), as `press` acts on it and the
    /// pointer's shape says it (see `Widgets::hover`).
    fn takes_at(&self, x: f32, y: f32) -> Takes {
        takes(&self.hits, self.hits_from, self.surface_size, self.pinned, self.button_at(x, y).is_some(), (x, y))
    }

    fn press(&mut self, client: POINT, keys: crate::station::select::Keys) {
        let px = self.contact.scale * self.zoom;
        let margin = self.margin_px();
        let (x, y) = ((client.x - margin) as f32 / px, (client.y - margin) as f32 / px);
        let (w, h) = self.surface_size;
        crate::perf::mark(|| format!("widget pressed at {x:.0},{y:.0} on {:?}, the pointer over it {}", self.button_at(x, y), self.hover));
        if self.button_at(x, y).is_none() && (x < -EDGE || y < -EDGE || x > w + EDGE || y > h + EDGE) {
            note(format!("widget: press at ({x:.0}, {y:.0}) outside its {w:.0} x {h:.0}"));
            return;
        }
        let Some(from) = cursor_position() else { return };
        // Caught as it was flung or settling: kept where it was caught.
        self.unsaved |= self.settle.is_some() || self.fling.is_some();
        self.settle = None;
        self.fling = None;
        self.opacity = 1.0;
        self.anchor = None;
        let (lx, ly) = (x - self.hits_from.0, y - self.hits_from.1);
        // On pages more than one: swiped across them, the hand going across first.
        self.swipes = self.hits.iter().find_map(|&(hx, hy, w, h, hit)| match hit {
            view::Hit::Pages(list, most) if f32::from_bits(most) > 0.0 && lx >= hx && lx < hx + w && ly >= hy && ly < hy + h => Some((list, f32::from_bits(most), w, Instant::now())),
            _ => None,
        });
        let takes = self.takes_at(x, y);
        // Where nothing may be chosen, pressed alone: nothing chosen here any more.
        if keys.none() && !matches!(takes, Takes::Shelf(_)) {
            crate::station::select::leave(self.window.hwnd.0 as isize);
        }
        match takes {
            Takes::Shelf(hit) => {
                self.shelf_press = Some(crate::station::select::press(self.window.hwnd.0 as isize, hit, self.content.tab(), keys, from));
                unsafe { SetCapture(self.window.hwnd) };
                return;
            }
            Takes::Track((hx, hy, wide, tall, hit)) => {
                self.drag = Some(match hit.upright() {
                    true => Drag::Slide { slider: hit.slider().unwrap(), from: hy + tall + self.hits_from.1, wide: tall, upright: true },
                    false => Drag::Slide { slider: hit.slider().unwrap(), from: hx + self.hits_from.0, wide, upright: false },
                });
                unsafe { SetCapture(self.window.hwnd) };
                self.slide();
                return;
            }
            Takes::Bar(bar) => {
                let Some(thumb) = self.content.readings_mut().and_then(|readings| view::Thumb::pressed(bar, ly, &readings.scrolls)) else { return };
                self.drag = Some(Drag::Scroll(thumb));
                unsafe { SetCapture(self.window.hwnd) };
                self.drag_thumb();
                return;
            }
            Takes::Nothing => {
                note("widget: pressed while pinned");
                return;
            }
            // (Acted on as it is let go.)
            Takes::Pressed => self.drag = Some(Drag::Move { grab: POINT { x: from.x - self.at.x, y: from.y - self.at.y }, trail: Vec::new(), torn: false, moved: false }),
            Takes::Edges(edges) => {
                let room = (self.surface_size.0 * self.zoom, self.surface_size.1 * self.zoom);
                self.drag = Some(Drag::Size { edges, from, room, corner: self.at });
                self.keeps_to = (edges.0, edges.2);
                // (What showed is measured against the room it has now, at each sizing: not the room one long ago left.)
                if let Content::Readings(readings) = &mut self.content {
                    readings.taken = None;
                }
            }
            Takes::Whole => {
                self.drag = Some(Drag::Move { grab: POINT { x: from.x - self.at.x, y: from.y - self.at.y }, trail: Vec::new(), torn: false, moved: false });
                self.splitting = match &self.content {
                    Content::Merged(merged) => merged.part_at((x, y)).filter(|(part, _)| *part > 0),
                    _ => None,
                };
            }
        }
        unsafe { SetCapture(self.window.hwnd) };
        // Above every window while it is in hand, kept on the desktop or not.
        self.window.lift(true);
        self.window.raise();
        // Over the others now where they meet: a widget that took the
        // desktop with them there takes it again.
        let (r, m) = (self.rect(), self.margin_px());
        own_window_changed(RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m });
        note(match &self.drag {
            Some(Drag::Size { edges, .. }) => format!("widget: sized by {edges:?} (left, right, top, bottom)"),
            Some(Drag::Move { .. }) => "widget: pressed to move".to_string(),
            Some(Drag::Slide { slider, .. }) => format!("widget: a track pressed ({slider:?})"),
            Some(Drag::Swipe { list, .. }) => format!("widget: pages swiped ({list:?})"),
            Some(Drag::Scroll(thumb)) => format!("widget: a list's bar pressed ({:?})", thumb.list()),
            None => "widget: pressed, nothing taken".to_string(),
        });
    }

    /// What is under the pointer changed without it (pages turned): what was said of what was there taken away, and
    /// it lit no more (the pointer moved again, what is there now is).
    fn unsay(&mut self) {
        if std::mem::take(&mut self.saying) {
            crate::ui::tip::hide();
        }
        self.hit_hover = None;
    }

    /// A press on pages (see `swipes`), the hand gone from `from` to `to` (physical px): swiped, if it went across
    /// more than down. Whether it is.
    fn begin_swipe(&mut self, from: POINT, to: POINT, now: Instant) -> bool {
        let Some((list, most, wide, _)) = self.swipes.take() else { return false };
        if (to.x - from.x).abs() <= (to.y - from.y).abs() {
            return false;
        }
        let Some(readings) = self.content.readings_mut() else { return false };
        let start = readings.scrolls.at(list);
        note(format!("widget: pages swiped ({list:?})"));
        self.drag = Some(Drag::Swipe { list, most, wide, from, start, trail: vec![(now, to)] });
        self.unsay();
        true
    }

    /// The pages swiped as far across as the hand has gone: a page's width, a page.
    fn swipe(&mut self, now: Instant) {
        let Some(cursor) = cursor_position() else { return };
        let px = self.contact.scale * self.zoom;
        let Some(Drag::Swipe { list, most, wide, from, start, trail }) = &mut self.drag else { return };
        trail.push((now, cursor));
        trail.retain(|(at, _)| now.duration_since(*at) <= FLING_SPAN * 2);
        let at = *start - (cursor.x - from.x) as f32 / px / *wide * view::PAGE;
        let (list, most) = (*list, *most);
        if let Some(readings) = self.content.readings_mut() {
            readings.scrolls.hold(list, at, most);
        }
    }

    /// Follows the pointer while it is in hand.
    /// The track slid along as far as where the pointer is along it.
    fn slide(&self) {
        let (Some(Drag::Slide { slider, from, wide, upright }), Some(cursor)) = (&self.drag, cursor_position()) else { return };
        let px = self.contact.scale * self.zoom;
        let along = if *upright { from - (cursor.y - self.at.y) as f32 / px } else { (cursor.x - self.at.x) as f32 / px - from };
        crate::playing::slide_along(*slider, along / wide);
    }

    /// The list whose bar's thumb is in hand scrolled as far as the thumb goes with the hand where the pointer is.
    fn drag_thumb(&mut self) {
        let (Some(Drag::Scroll(thumb)), Some(cursor)) = (&self.drag, cursor_position()) else { return };
        let thumb = *thumb;
        let y = (cursor.y - self.at.y) as f32 / (self.contact.scale * self.zoom) - self.hits_from.1;
        if let Some(readings) = self.content.readings_mut() {
            readings.scrolls.hold(thumb.list(), thumb.scroll_at(y), thumb.most());
        }
        self.unsay();
        self.next_frame = Instant::now();
    }

    fn follow(&mut self, now: Instant) {
        if matches!(self.drag, Some(Drag::Slide { .. })) {
            self.slide();
            return;
        }
        if matches!(self.drag, Some(Drag::Scroll(_))) {
            self.drag_thumb();
            return;
        }
        if matches!(self.drag, Some(Drag::Swipe { .. })) {
            self.swipe(now);
            return;
        }
        let Some(cursor) = cursor_position() else { return };
        let scale = self.contact.scale;
        // Pressed on pages and gone far enough across them before down: they are swiped (pinned too: that is not
        // moving it).
        if let Some(Drag::Move { grab, moved: false, .. }) = self.drag {
            let from = POINT { x: self.at.x + grab.x, y: self.at.y + grab.y };
            let (dx, dy) = ((cursor.x - from.x) as f32 / scale, (cursor.y - from.y) as f32 / scale);
            if dx.hypot(dy) >= MOVE_FROM && self.begin_swipe(from, cursor, now) {
                self.swipe(now);
                return;
            }
        }
        match &mut self.drag {
            // Pinned: pressed on what it shows, acted on as it is let go; not moved.
            Some(Drag::Move { .. }) if self.pinned => {}
            Some(Drag::Move { grab, trail, moved, .. }) => {
                trail.push((now, cursor));
                trail.retain(|(at, _)| now.duration_since(*at) <= FLING_SPAN * 2);
                let to = POINT { x: cursor.x - grab.x, y: cursor.y - grab.y };
                if !*moved {
                    let (dx, dy) = ((to.x - self.at.x) as f32 / scale, (to.y - self.at.y) as f32 / scale);
                    if dx.hypot(dy) < MOVE_FROM {
                        return;
                    }
                    *moved = true;
                    // Taken by a part merged into it: that part torn out (it moves; this stays).
                    if let Some(split) = self.splitting.take() {
                        self.split = Some(split);
                        return;
                    }
                }
                // Pulled away from the edge it was stuck to: as it was before,
                // under the hand where it was taken.
                if let Some(side) = self.stuck {
                    let work = self.contact.work;
                    let (w, h) = (self.surface_size.0 * self.zoom * scale, self.surface_size.1 * self.zoom * scale);
                    let away = match side {
                        Side::Left => to.x - work.left,
                        Side::Right => work.right - (to.x + w as i32),
                        Side::Top => to.y - work.top,
                        Side::Bottom => work.bottom - (to.y + h as i32),
                    } as f32
                        / scale;
                    if away > UNSTICK {
                        self.stuck = None;
                        self.along = None;
                        self.also = None;
                        self.keeps_to = (false, false);
                    }
                    // Pulled out of its corner along the edge: stuck to that edge alone, slid along it.
                    let rect = RECT { left: to.x, top: to.y, right: to.x + w as i32, bottom: to.y + h as i32 };
                    if self.also.is_some_and(|also| corner_side(work, rect, scale, side, UNSTICK) != Some(also)) {
                        self.also = None;
                        self.keeps_to = (side == Side::Right, side == Side::Bottom);
                    }
                }
                self.at = POINT { x: cursor.x - grab.x, y: cursor.y - grab.y };
                note_sparingly("followed", || format!("widget: followed the hand to ({}, {})", cursor.x, cursor.y));
                if let Some(contact) = monitor_at(cursor) {
                    self.moved_to(contact);
                }
                // Torn off and now well clear of the edge the panel was at
                // (off a panel at none, at once), or on another screen: in
                // hand as any widget is, to stick where it is brought to an
                // edge. (Once it has a size: how far it is from an edge is
                // not known before.)
                if matches!(self.drag, Some(Drag::Move { torn: true, .. })) && self.surface_size != (0.0, 0.0) {
                    let here = self.contact.monitor;
                    if self.torn_from.is_none_or(|(side, screen)| screen != here || side.is_none_or(|side| self.away_from(side) > UNSTICK)) {
                        self.torn_from = None;
                        if let Some(Drag::Move { torn, .. }) = &mut self.drag {
                            *torn = false;
                        }
                    }
                }
            }
            Some(Drag::Size { edges, from, room, corner }) => {
                let (dx, dy) = ((cursor.x - from.x) as f32 / scale, (cursor.y - from.y) as f32 / scale);
                let (l, r, t, b) = *edges;
                let width = (room.0 + if r { dx } else if l { -dx } else { 0.0 }).max(1.0);
                let height = (room.1 + if b { dy } else if t { -dy } else { 0.0 }).max(1.0);
                self.room = Some((width, height));
                self.stuck = None;
                self.along = None;
                self.also = None;
                // The edges not held stay where they were.
                let start = (room.0 * scale, room.1 * scale);
                let right = corner.x + start.0 as i32;
                let bottom = corner.y + start.1 as i32;
                self.at = POINT { x: if l { right } else { corner.x }, y: if t { bottom } else { corner.y } };
            }
            Some(Drag::Slide { .. } | Drag::Swipe { .. } | Drag::Scroll(_)) | None => {}
        }
    }

    /// Let go: a press where it was is a click (on a button, what it does); a move, flung if fast enough. Cancelled
    /// (the capture taken away), neither: what was pressed is not done, and it stays where it was.
    fn release(&mut self, now: Instant, cancelled: bool) -> Released {
        // (Its own capture: not one another has taken since, a part torn out of it.)
        crate::ui::window::let_go_of_capture(self.window.hwnd);
        // Back where it is kept, let go: but a note pulled off the panel going back to it, over the panel still on its
        // way, as in hand (kept under it, it went back behind it; the capture's end after it let go comes here too).
        let going_back = self.pulled.as_ref().is_some_and(|pulled| pulled.back.is_some());
        self.window.lift(going_back);
        self.hide_hint();
        self.splitting = None;
        // (The capture's end after a release comes here too, with nothing.)
        let Some(drag) = self.drag.take() else { return Released::Nothing };
        match drag {
            // The track let go: as it was left (cancelled, as it was), and looked at again.
            Drag::Slide { slider, .. } => {
                crate::playing::slid(slider, !cancelled);
                Released::Nothing
            }
            // A list's bar let go: the list left where the thumb took it.
            Drag::Scroll(_) => Released::Nothing,
            // Pages let go: on to the page nearest; flicked, the next that way (the hand going left, the next on).
            Drag::Swipe { list, most, trail, .. } => {
                let px = self.contact.scale * self.zoom;
                let recent: Vec<&(Instant, POINT)> = trail.iter().filter(|(at, _)| now.duration_since(*at) <= FLING_SPAN).collect();
                let speed = match (recent.first(), recent.last()) {
                    (Some(first), Some(last)) if last.0.duration_since(first.0).as_secs_f32() > 0.01 => (last.1.x - first.1.x) as f32 / px / (last.0.duration_since(first.0).as_secs_f32() * 1000.0),
                    _ => 0.0,
                };
                let lean = if cancelled || speed.abs() < FLICK { 0.0 } else { -speed.signum() };
                if let Some(readings) = self.content.readings_mut() {
                    readings.scrolls.let_go_pages(list, lean, view::PAGE, most);
                }
                Released::Nothing
            }
            Drag::Size { .. } => {
                note(format!("widget: let go, sized to {:.0} x {:.0}{}", self.surface_size.0 * self.zoom, self.surface_size.1 * self.zoom, if cancelled { " (its capture lost)" } else { "" }));
                // It settles from the surface it shows (the hand's room, given
                // as a band gives) onto what it shows, the edges not dragged
                // where they are.
                if self.room.take().is_some() {
                    // What plays, within the range its layout is right in, from as large as the hand made it.
                    if let Content::Music(shown) = &mut self.content {
                        shown.size = music::rests_at(shown.size, &Shows::from(&crate::app().settings.lock().unwrap().view));
                        self.shown = shown.size;
                    }
                    self.settle = Some((now, self.surface_size));
                    let r = self.rect();
                    self.anchor = Some(POINT { x: r.right, y: r.bottom });
                }
                self.retone();
                Released::Changed
            }
            Drag::Move { trail, torn, moved, .. } => {
                if !moved && cancelled {
                    return Released::Nothing;
                }
                // A note pulled off the panel let go over it again: back to its card there; elsewhere, a note where it is
                // put down (not flung: as it is let go, it is still settling into a note).
                if let Some(pulled) = &mut self.pulled {
                    if pulled.wants(cursor_position()) == 0.0 {
                        note("widget: a note pulled off the panel let go over it, going back");
                        let to = |at: i32, card: i32| {
                            let mut spring = Spring::at(at as f32, GOING_BACK.0, GOING_BACK.1);
                            spring.target = card as f32;
                            spring
                        };
                        pulled.back = Some((to(self.at.x, pulled.card.left), to(self.at.y, pulled.card.top)));
                        pulled.morph.target = 0.0;
                        self.window.lift(true);
                        return Released::Nothing;
                    }
                    pulled.morph.target = 1.0;
                    self.retone();
                    self.stick();
                    return Released::Changed;
                }
                if !moved {
                    // A press let go where it was: a button, if it was on one.
                    let px = self.contact.scale * self.zoom;
                    let Some(cursor) = cursor_position() else { return Released::Nothing };
                    let (x, y) = ((cursor.x - self.at.x) as f32 / px, (cursor.y - self.at.y) as f32 / px);
                    crate::perf::mark(|| format!("widget let go at {x:.0},{y:.0} on {:?}", self.button_at(x, y)));
                    return match self.button_at(x, y) {
                        Some(Button::Pin) => {
                            self.pinned ^= true;
                            Released::Changed
                        }
                        Some(Button::Through) => {
                            self.click_through ^= true;
                            (self.lingered, self.reached) = (None, false);
                            Released::Changed
                        }
                        Some(Button::Close) => Released::Closed,
                        Some(Button::Settings) => Released::Settings(self.button_on_screen(Button::Settings)),
                        Some(Button::Full) => Released::Full,
                        Some(Button::Game) => {
                            press_game();
                            Released::Nothing
                        }
                        // No button: what of its lanes is there (what plays, the sound).
                        None => {
                            let (lx, ly) = (x - self.hits_from.0, y - self.hits_from.1);
                            let hit = self.hits.iter().find(|(hx, hy, w, h, _)| lx >= *hx && lx < hx + w && ly >= *hy && ly < hy + h).map(|hit| hit.4);
                            // A to-do being written: a press away from its field gives back the keys.
                            if !matches!(hit, Some(view::Hit::Todo(view::TodoHit::Field | view::TodoHit::Add | view::TodoHit::FieldDue))) {
                                self.stop_todo(false);
                            }
                            match hit {
                                Some(view::Hit::Todo(hit)) => match crate::ui::todo::pressed(hit) {
                                    crate::ui::todo::Act::Nothing => {}
                                    crate::ui::todo::Act::Write => self.write_todo(),
                                    crate::ui::todo::Act::Pick(of) => {
                                        if let Some(anchor) = self.hits_on_screen().into_iter().find(|(_, on)| *on == view::Hit::Todo(hit)).map(|(rect, _)| rect) {
                                            crate::ui::todo::pick::open(self.window.hwnd, of, anchor, self.menu_look(), crate::update::language());
                                        }
                                    }
                                },
                                Some(view::Hit::Media(key, follow)) => crate::playing::act_on(follow, crate::playing::Act::Key(key)),
                                Some(view::Hit::Drive(letter)) => crate::panel::open_drive(letter),
                                Some(hit) if crate::station::pressed(hit) => {}
                                Some(view::Hit::Tab(tab)) => {
                                    if let Some(readings) = self.content.readings_mut() {
                                        readings.tab = tab;
                                    }
                                    crate::station::select::turned(self.window.hwnd.0 as isize, tab);
                                }
                                // A grid's way to its next page: turned to it.
                                Some(view::Hit::NextPage(list, most)) => {
                                    if let Some(readings) = self.content.readings_mut() {
                                        readings.scrolls.turn_pages(list, view::PAGE, view::PAGE, f32::from_bits(most));
                                    }
                                    self.unsay();
                                }
                                Some(hit) if hit.opens_menu() => return Released::Menu(hit),
                                // A note stuck on the desktop, its card clicked (its head, its foot): written in where
                                // it is, the caret at its end.
                                Some(view::Hit::Note(number)) if card_in(&self.hits, self.hits_from, number, 0.0).is_some() => {
                                    self.write_note(number, true);
                                }
                                Some(view::Hit::Search) => crate::ui::find::toggle(),
                                // Quick launch's choosing of programs, a note: beside it on the side with more room.
                                Some(hit @ (view::Hit::AddApp | view::Hit::NewNote | view::Hit::Note(_))) => {
                                    use crate::ui::beside::Page;
                                    let page = match hit {
                                        view::Hit::AddApp => Some(Page::Picker),
                                        view::Hit::NewNote => Some(Page::Note(None)),
                                        view::Hit::Note(number) => crate::notes::named(number).map(|name| Page::Note(Some(name))),
                                        _ => None,
                                    };
                                    if let Some(page) = page {
                                        self.open_beside(page);
                                    }
                                }
                                Some(view::Hit::Line(index)) => {
                                    music::play_line(index);
                                    match &mut self.content {
                                        Content::Music(shown) => shown.music.back_to_now(),
                                        Content::Merged(merged) => merged.parts.iter_mut().for_each(|part| {
                                            if let Content::Music(shown) = part {
                                                shown.music.back_to_now();
                                            }
                                        }),
                                        Content::Readings(_) => {}
                                    }
                                }
                                _ => {}
                            }
                            Released::Nothing
                        }
                    };
                }
                // A fling: fast enough as it was let go (a tear is put down).
                let scale = self.contact.scale;
                let recent: Vec<&(Instant, POINT)> = trail.iter().filter(|(at, _)| now.duration_since(*at) <= FLING_SPAN).collect();
                if let (false, false, Some(first), Some(last)) = (torn, cancelled, recent.first(), recent.last()) {
                    let ms = last.0.duration_since(first.0).as_secs_f32() * 1000.0;
                    if ms > 10.0 {
                        let (vx, vy) = ((last.1.x - first.1.x) as f32 / scale / ms, (last.1.y - first.1.y) as f32 / scale / ms);
                        if vx.hypot(vy) > FLING {
                            note(format!("widget: flung at {:.1} DIP/ms", vx.hypot(vy)));
                            self.fling = Some((vx, vy));
                            return Released::Changed;
                        }
                    }
                }
                self.retone();
                // Torn off the panel and still at its edge, it is put down where it is let go.
                if !torn {
                    self.stick();
                }
                Released::Changed
            }
        }
    }

    /// Let go near an edge of its screen's work area: stuck to it, as it is (sticking only says where it is, never
    /// what it shows or how large); already stuck, flush with its edge again. Near the edge across it too, stuck in
    /// their corner.
    fn stick(&mut self) {
        let near = self.nearest_side().filter(|(_, away)| *away < STICK).map(|(side, _)| side);
        let was = self.stuck.map(|side| (side, self.along.unwrap_or_else(|| self.middle_along(side))));
        let Some((side, along)) = stuck_after(near, was, self.rect()) else { return };
        let also = corner_side(self.contact.work, self.rect(), self.contact.scale, side, STICK);
        if (self.stuck, self.also) != (Some(side), also) {
            match also {
                Some(also) => note(format!("widget: stuck in the {side:?} {also:?} corner")),
                None => note(format!("widget: stuck to the {side:?} edge")),
            }
        }
        self.stuck = Some(side);
        self.along = Some(along);
        self.also = also;
        let keeps = |to: Side| side == to || also == Some(to);
        self.keeps_to = (keeps(Side::Right), keeps(Side::Bottom));
    }

    /// The edge of its screen's work area it is nearest, and how near (DIPs).
    fn nearest_side(&self) -> Option<(Side, f32)> {
        [Side::Left, Side::Right, Side::Top, Side::Bottom].into_iter().map(|side| (side, self.away_from(side))).min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// How far it is from `side` of its screen's work area (DIPs).
    fn away_from(&self, side: Side) -> f32 {
        let (work, rect) = (self.contact.work, self.rect());
        let away = match side {
            Side::Left => rect.left - work.left,
            Side::Right => work.right - rect.right,
            Side::Top => rect.top - work.top,
            Side::Bottom => work.bottom - rect.bottom,
        };
        away as f32 / self.contact.scale
    }

    /// Where its middle is along `side` now (physical px).
    fn middle_along(&self, side: Side) -> i32 {
        middle_along(self.rect(), side)
    }

    /// Where a surface `size` large (physical px) stuck to `side`, its middle `along` it, and to the side across it
    /// `also`, has its corner (see the function of that name).
    fn stuck_at(&self, side: Side, along: Option<i32>, also: Option<Side>, size: (i32, i32)) -> POINT {
        stuck_at(self.contact.work, self.at, side, along, also, size)
    }

    /// Shows where it would stick, as the skin draws it: `rect` (physical
    /// px), firmer once letting go would stick it there, `strength` as
    /// strong.
    fn show_hint(&mut self, rect: RECT, firm: bool, strength: f32, theme: &Theme, ground: Option<Ground>) {
        // (Its widget gone onto another device, a screen another adapter drives: made again on that one.)
        if self.hint.as_ref().is_some_and(|hint| !Rc::ptr_eq(&hint.gfx, &self.gfx)) {
            self.hide_hint();
        }
        if self.hint.is_none() {
            let Ok(mut window) = Window::new() else { return };
            window.set_click_through(true);
            // Never in what a widget sees behind it: it is where the widget
            // would go, not part of the desktop.
            crate::ui::scene::leave_out(window.hwnd, true);
            let surface = Surface::new(&self.gfx, window.hwnd).ok();
            self.hint = Some(Hint { window, surface, gfx: self.gfx.clone(), drawn: None });
        }
        let Some(hint) = self.hint.as_mut() else { return };
        let this = (rect, firm, strength.to_bits(), ground);
        if hint.drawn == Some(this) {
            return;
        }
        hint.drawn = Some(this);
        // Just under the widget in hand, as Windows' own snap preview is under the window moved: over it, the hint's
        // pane lay on the widget where the two met, which came out dimmed, lighter or darker, as it was picked up.
        hint.window.place_under(rect, Some(self.window.hwnd));
        hint.window.show_in_place();
        let Some(surface) = hint.surface.as_mut() else { return };
        let scale = self.contact.scale;
        let pixels = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        let (w, h) = (pixels.0 as f32 / scale, pixels.1 as f32 / scale);
        let _ = surface.draw(&self.gfx, pixels, scale, |frame| {
            frame.origin(0.0, 0.0);
            paint_hint(frame, theme, (w, h), firm, strength, ground);
        });
    }

    fn hide_hint(&mut self) {
        if let Some(hint) = self.hint.take() {
            crate::ui::scene::leave_out(hint.window.hwnd, false);
            hint.window.destroy();
        }
    }

    /// Let go somewhere else on its screen: the desktop taken behind it
    /// serves still (it was taken without the widget); only how light it
    /// is where the widget now is, which the theme follows, is read again.
    fn retone(&mut self) {
        // Measured once it is where it comes to rest, on what is taken
        // there (see `frame`).
        self.tone_due = true;
        // What is behind it now is looked at once, the desktop having
        // changed meanwhile, or not; not when it was a moment ago (as it was
        // torn off).
        let fresh = self.behind.as_ref().is_some_and(|behind| behind.taken.elapsed() < RETAKE_AFTER);
        self.retake |= !fresh;
    }

    /// Whether it is merged into what plays, drawn on its own ground (as `music` says it is).
    fn on_music(&self, music: MusicGround) -> bool {
        music != MusicGround::Skin && matches!(&self.content, Content::Merged(merged) if merged.host_plays())
    }

    /// Whether it is kept under every other window.
    fn low(&self) -> bool {
        self.on_desktop && !self.game_only
    }

    fn margin_px(&self) -> i32 {
        (self.margin * self.contact.scale * self.zoom).ceil() as i32
    }

    /// How far past its surface its buttons' row reaches, above, under, left and right of it (physical px): none but
    /// on the side they are at, and there only while they show (the pointer over it).
    fn reach(&self) -> (i32, i32, i32, i32) {
        // (Its buttons' room taken where they show: the pointer over it, or its settings' button lit.)
        let reach = if self.hover || self.settings_lit().is_some() { ((BUTTON + 2.0 * BUTTON_INSET) * self.contact.scale * self.zoom) as i32 } else { 0 };
        match self.rack() {
            Rack::Above => (reach, 0, 0, 0),
            Rack::Under => (0, reach, 0, 0),
            Rack::Left => (0, 0, reach, 0),
            Rack::Right => (0, 0, 0, reach),
        }
    }

    /// Its window on the screen (physical px): its surface and the room round it for its shadow and its buttons.
    fn window_rect(&self) -> RECT {
        let (r, m) = (self.rect(), self.margin_px());
        RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m }
    }

    /// Where it takes the pointer (physical px): its surface, the band round its edge that sizes it (see `edges_at`)
    /// and, where they show, its buttons.
    fn taking(&self) -> RECT {
        let band = (EDGE * self.contact.scale * self.zoom).ceil() as i32;
        let (above, under, left, right) = self.reach();
        let r = self.rect();
        RECT { left: r.left - band.max(left), top: r.top - band.max(above), right: r.right + band.max(right), bottom: r.bottom + band.max(under) }
    }

    /// A note's step of `seconds` on: lifted in hand, leaning as fast as the hand moves it across, and once put down,
    /// coming to rest; pulled off the panel (see `Pulled`), turning between its card and a note as the pointer is over
    /// the panel or not, going back to the panel on its way, and once put down a note at rest, an ordinary one. Whether
    /// it is still moving.
    fn carry(&mut self, seconds: f32) -> bool {
        if self.content.stuck_note().is_none() {
            return false;
        }
        let in_hand = !self.pinned && matches!(self.drag, Some(Drag::Move { moved: true, .. }));
        let scale = self.contact.scale;
        // How fast it goes across (DIPs a millisecond), as the hand moved it lately.
        let across = match &self.drag {
            Some(Drag::Move { trail, moved: true, .. }) => {
                let recent: Vec<&(Instant, POINT)> = trail.iter().filter(|(at, _)| at.elapsed() <= LEAN_SPAN).collect();
                match (recent.first(), recent.last()) {
                    (Some(first), Some(last)) if last.0 > first.0 => (last.1.x - first.1.x) as f32 / scale / (last.0.duration_since(first.0).as_secs_f32() * 1000.0),
                    _ => 0.0,
                }
            }
            _ => 0.0,
        };
        let (most, per) = if self.pulled.is_some() { LEAN_PULLED } else { LEAN_MOVED };
        self.lift.target = if in_hand { 1.0 } else { 0.0 };
        self.lean.target = if in_hand { (across * per).clamp(-most, most) } else { 0.0 };
        // Lifted and leaning about where it is held; at rest, about its middle (asked for, it is lifted there: see
        // `Widgets::raise_note`). Pulled off the panel, held as `Pulled::grab_at` says (see `frame`).
        if self.pulled.is_none() {
            match &self.drag {
                Some(Drag::Move { grab, .. }) if in_hand => {
                    let px = scale * self.zoom;
                    self.about = (grab.x as f32 / px, grab.y as f32 / px);
                }
                _ if self.lift.rests(0.002) && self.lean.rests(0.01) => self.about = (self.surface_size.0 / 2.0, self.surface_size.1 / 2.0),
                _ => {}
            }
        }
        self.lift.step(seconds);
        self.lean.step(seconds);
        let mut moving = !self.lift.rests(0.002) || !self.lean.rests(0.01);
        let cursor = cursor_position();
        if let Some(pulled) = &mut self.pulled {
            if in_hand {
                pulled.morph.target = pulled.wants(cursor);
            }
            pulled.morph.step(seconds);
            if let Some((x, y)) = &mut pulled.back {
                x.step(seconds);
                y.step(seconds);
            }
            moving |= !pulled.morph.rests(0.002) || pulled.back.is_some();
            if !in_hand && pulled.back.is_none() && !moving {
                self.pulled = None;
                self.unsaved = true;
            }
        }
        moving || in_hand
    }

    /// Lays it out, places its window and draws it.
    fn frame(&mut self, now: Instant, look: &Look, history: &[Arc<Sample>], seen: &Seen, controller: &crate::panel::Controller) -> Frame {
        // (What it draws and takes from behind it is sized as it is: see `gfx::room`.)
        // (Settling to its size after it is let go, too.)
        let sizing = matches!(self.drag, Some(Drag::Size { .. })) || self.settle.is_some();
        let _sizing = gfx::sizing(sizing);
        // (What another window drew on this thread before forgotten: see `view::gliding_drawn`.)
        crate::ui::view::gliding_drawn();
        let dt = now.duration_since(self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = now;
        // Each part of a frame timed, and a slow one noted.
        let started = Instant::now();
        let mut moving = self.drag.is_some() || self.settle.is_some();
        self.follow_settings();
        // (Frames while its settings' button bounces.)
        moving |= self.settings_lit().is_some_and(|grown| grown != 1.0);
        // The knob under the pointer, and the one held (the slider in hand's). (A copy drawn with: the widget is
        // drawn meanwhile.)
        let sliding = match self.drag {
            Some(Drag::Slide { slider, .. }) => Some(slider),
            _ => None,
        };
        let held = sliding.and_then(|sliding| self.hits.iter().map(|hit| hit.4).find(|hit| hit.slider() == Some(sliding)));
        moving |= self.thumbs.step(now, self.hit_hover.filter(|hit| hit.eases()), held);
        // (What is chosen of several, its mark sliding to it as it changes: the station's page, the notes' view.)
        // (What it shows of them alone: what is chosen, its mark sliding as it changes; what is kept at hand, going to its
        // places as its order changes. A widget showing none of them asks no frames for them.)
        let shows = |kind: fn(&view::Hit) -> bool| self.hits.iter().any(|hit| kind(&hit.4));
        if shows(|hit| matches!(hit, view::Hit::Tab(_) | view::Hit::NotesShow(_))) {
            moving |= self.thumbs.choose(now, &view::choices(self.content.tab()));
        }
        // What was just copied from what it shows of the station, marked so a moment.
        if shows(|hit| matches!(hit, view::Hit::Shelf(_) | view::Hit::Copied(_) | view::Hit::Recent(_))) {
            moving |= self.thumbs.mark_copied(now, view::copied(), crate::station::run::COPIED_FOR);
            moving |= self.thumbs.mark_arrived(now, view::arrived(), crate::station::run::ARRIVED_FOR);
        }
        // What is chosen together on what it shows of the station, marked so.
        self.thumbs.mark_chosen(crate::station::select::marked(self.window.hwnd.0 as isize));
        if shows(|hit| matches!(hit, view::Hit::App(_) | view::Hit::AppsArea)) {
            let order: Vec<String> = crate::station::apps::shown().iter().map(|app| app.target.clone()).collect();
            moving |= self.thumbs.apps(now, &order, None);
        }
        let thumbs = self.thumbs.clone();
        // The preview's shortcut said on what the pointer rests on that may be looked at larger: not while a look is up
        // from it, or it is written in where it is (its keys are its own then).
        let says = crate::ui::look::hint_says(look.lang).filter(|_| !crate::ui::look::up_for(self.window.hwnd) && self.in_place.is_none());
        let hint = self.preview_hint.step(self.hit_hover.filter(|_| says.is_some()), now);
        let hint = hint.zip(says.as_deref()).map(|((hit, shown), says)| (hit, shown, says));
        // Its notes turned through by pages: kept on the note they show as their order changes, on one opened beside.
        let note_pages = shows(|hit| matches!(hit, view::Hit::Pages(view::List::Notes, _)));
        let writing = if note_pages { crate::ui::beside::writing_for(self.window.hwnd) } else { None };
        // Its process list turned on toward where the wheel turned it, as the panel's is.
        let readings: Vec<&mut Readings> = match &mut self.content {
            Content::Readings(readings) => vec![readings],
            Content::Merged(merged) => merged.parts.iter_mut().filter_map(|part| if let Content::Readings(readings) = part { Some(readings) } else { None }).collect(),
            Content::Music(_) => Vec::new(),
        };
        for readings in readings {
            readings.scrolls.follow(view::lists_show(readings.tab));
            if note_pages {
                view::keep_note(&mut readings.scrolls, &mut readings.note_shown, &crate::notes::listed(), writing.as_deref());
            }
            moving |= readings.scrolls.ease(dt / 1000.0, crate::panel::SCROLL_EASE);
        }
        // Settling this frame (its last too): its far edges held.
        let settling = self.settle.is_some();
        // Flung: on along its way, slowing; gone once it has left the
        // screens, or at rest where it came to a stop.
        if let Some((vx, vy)) = self.fling {
            let scale = self.contact.scale;
            // As far as it goes slowing all the while (however long since the
            // last frame: a menu up meanwhile): v (1 - kᵗ) / -ln k.
            let keep = FRICTION.powf(dt);
            let travel = (1.0 - keep) / -FRICTION.ln();
            self.at.x += (vx * travel * scale) as i32;
            self.at.y += (vy * travel * scale) as i32;
            // Onto another screen on its way: on that one now.
            let r = self.rect();
            if let Some(contact) = monitor_at(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }) {
                self.moved_to(contact);
            }
            let (vx, vy) = (vx * keep, vy * keep);
            self.fling = Some((vx, vy));
            let screens = unsafe {
                let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
                RECT { left: x, top: y, right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN), bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN) }
            };
            let r = self.rect();
            let (w, h) = ((r.right - r.left).max(1) as f32, (r.bottom - r.top).max(1) as f32);
            let outside = (screens.left - r.left).max(r.right - screens.right).max(0) as f32 / w + (screens.top - r.top).max(r.bottom - screens.bottom).max(0) as f32 / h;
            self.opacity = (1.0 - outside * 1.4).clamp(0.0, 1.0);
            if self.opacity <= 0.0 {
                return Frame::Gone;
            }
            // Slowed to a stop: put away if it is mostly gone, else at rest,
            // all of it back on its screen.
            if vx.hypot(vy) < 0.15 {
                if self.opacity <= 0.5 {
                    return Frame::Gone;
                }
                self.fling = None;
                self.unsaved = true;
                self.opacity = 1.0;
                if let Some(contact) = monitor_at(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }) {
                    self.moved_to(contact);
                }
                // Come to rest: light or dark as the desktop is there.
                self.retone();
            }
            moving = true;
        }
        // On what plays' own ground (merged into it), dark, as that ground is.
        let on_music = self.on_music(look.prefs.music.ground);
        let dark = on_music || theme::is_dark(look.prefs.theme, self.behind.as_ref().map(|b| b.tone.0).filter(|_| look.skin.sees_backdrop()));
        let theme = self.content.dressed(Theme::new(look.skin, dark));
        self.drawn_look = (look.skin, dark);
        // Room round it for its shadow (the skin's; what plays, its own), and above it for its buttons.
        let shadow = if !self.content.skinned(look.prefs.music.ground) { MUSIC_SHADOW } else { theme.margin };
        self.margin = shadow.max(BUTTON + 2.0 * BUTTON_INSET);
        let scrolls = self.content.scrolls();
        let tab = self.content.tab();
        let music_apart = self.content.music_apart();
        // Merged with what plays, the readings without the music lane (what plays shows itself).
        let prefs = self.content.prefs(look.prefs);
        let prefs = prefs.as_ref().unwrap_or(look.prefs);
        let info = controller.info();
        let scene = view::Scene { hover: self.hover_shown(), hint, thumbs: &thumbs, scrolls, tab, music_apart, ..crate::panel::scene_for(controller, &info, prefs, &theme, look.lang, history, seen) };
        // Laid out as it is wanted: what fills the room it is dragged to, else
        // what it shows.
        let work = self.contact.work;
        let work = ((work.right - work.left) as f32 / self.contact.scale, (work.bottom - work.top) as f32 / self.contact.scale);
        // What plays and widgets merged are as large as they are made, within the screen they are on, the room round
        // them for their shadow and buttons too (made larger on a larger screen, or kept so, they were larger than another, and part of them, their
        // buttons with it, was past its edge).
        let margin = self.margin;
        let within = |size: (f32, f32), least: (f32, f32)| {
            let most = ((work.0 - 2.0 * margin).max(least.0), (work.1 - 2.0 * margin).max(least.1));
            (size.0.clamp(least.0, most.0), size.1.clamp(least.1, most.1))
        };
        // Moved or flung, it shows what it showed (laid out again once at
        // rest, as the readings step); else laid out for what it shows, or
        // for the room it is sized to.
        let only_moved = matches!(self.drag, Some(Drag::Move { .. })) || self.fling.is_some();
        // A note stuck on the desktop alone: its page, made as large as it is made (see `view::STUCK_NOTE`).
        let page = self.content.bare();
        let sized_to = self.room.filter(|_| matches!(self.drag, Some(Drag::Size { .. })));
        match &mut self.content {
            Content::Readings(readings) => {
                let shown = match readings.last.clone().filter(|last| only_moved && last.form == readings.choice.form) {
                    Some(last) => last,
                    // The workbench's lanes: laid out again for the size it is made (its columns as wide, its lists as
                    // tall), not scaled to it, down to its least (see `view::LANE_LEAST`, `view::gives`).
                    None if readings.workbench => {
                        let heights = view::heights(&scene);
                        let columns = match readings.choice.form {
                            Form::Lanes { columns, .. } => columns.min(heights.len()).max(1),
                            Form::Small(_) => 1,
                        };
                        // (Made shorter, its lists give way; its other lanes are as designed: see `view::Layout::giving`.)
                        let designed = view::Layout::with_columns(heights.iter().map(|(_, h)| h[0]).collect(), columns, &theme).giving(view::gives(&scene));
                        let (designed, least, shape_least) = match page {
                            true => (designed.clone().fitted(view::STUCK_NOTE), view::STUCK_NOTE_LEAST, view::STUCK_NOTE_LEAST),
                            false => {
                                let least = (designed.width() - columns as f32 * (view::COLUMN_WIDTH - view::LANE_LEAST), designed.least_height());
                                (designed, least, SHAPE_LEAST)
                            }
                        };
                        let wanted = sized_to.unwrap_or((designed.width() + readings.choice.extra.0, designed.height() + readings.choice.extra.1));
                        let size = within(wanted, shape_least);
                        let extra = (size.0 - designed.width(), size.1 - designed.height());
                        // Smaller than all of it fits in: each lane in a shape of its own (see `view::Shape`), across
                        // side by side, else one over another.
                        let (detail, layout) = if size.0 >= least.0 && size.1 >= least.1 {
                            (view::Detail::Full, designed.fitted(size))
                        } else {
                            let shape = view::Shape::of(size);
                            let detail = view::Detail::Shape(shape);
                            let shaped: Vec<f32> = heights.iter().filter_map(|(id, _)| view::lane_height(&scene, id, scene.seen, detail)).collect();
                            let columns = shape.columns(shaped.len());
                            (detail, view::Layout::with_columns(shaped, columns, &theme).fitted(size))
                        };
                        // (Its columns kept as chosen, whatever its shape: sized up again, it is laid out as it was.)
                        let form = Form::Lanes { detail, columns };
                        readings.choice = Choice { form, scale: 1.0, extra };
                        Shown { form, zoom: 1.0, size: (layout.width(), layout.height()), layout, small: None, detail }
                    }
                    None => {
                        let heights = view::heights(&scene);
                        let lanes: Vec<(&str, arrange::Heights)> = heights.iter().map(|(id, h)| (id.as_str(), *h)).collect();
                        let readings_count = forms::count(&scene);
                        let want = match sized_to {
                            Some(room) => Want::Room(room, Some(readings.taken.filter(|taken| taken.form == readings.choice.form).unwrap_or(arrange::Shown { form: readings.choice.form, taken: room }))),
                            None => Want::Kept(readings.choice),
                        };
                        let opening = arrange::Opening::new(&theme, Edge::Right, &lanes, readings_count, work, None, 1.0, want, seen.clone());
                        let Some(choice) = opening.choice else { return Frame::Rest };
                        if let Some(room) = sized_to {
                            readings.taken = Some(arrange::Shown::after(readings.taken, choice.form, room));
                        }
                        readings.choice = choice;
                        Shown { form: choice.form, zoom: opening.zoom, size: opening.size, layout: opening.layout.clone(), small: opening.small(), detail: opening.detail() }
                    }
                };
                self.zoom = shown.zoom;
                self.shown = shown.size;
                // The workbench's sized by hand while still turning: the turn ends at once (see `turns_into`).
                if readings.workbench && sized_to.is_some() && readings.morph.take().is_some() {
                    readings.drawn.clear();
                    readings.morph_layer.release();
                }
                // Another layout: what showed turns into it, from where it is now if
                // it was still turning (see `turns_into`).
                if let Some(last) = readings.last.take().filter(|last| turns_into(last, &shown, readings.workbench, sized_to.is_some(), &theme)) {
                    note(format!("widget: {} -> {}", last.form.key(), shown.form.key()));
                    let before = if readings.morph.is_some() { Before::Drawn(std::mem::take(&mut readings.drawn)) } else { Before::Shown(last) };
                    readings.morph = Some((now, before));
                }
                readings.last = Some(shown);
                if readings.morph.as_ref().is_some_and(|(since, _)| now.duration_since(*since) >= MORPH) {
                    readings.morph = None;
                    readings.drawn.clear();
                    readings.morph_layer.release();
                }
                moving |= readings.morph.is_some();
            }
            // What plays is laid out for the room it is given (see `music::Layout`), as large as the hand makes it while
            // it is sized (as a widget of readings follows the hand), and let go, it settles within the range its layout
            // is right in (see `release`). (Held to that range as it was sized, it followed the hand only part of the way
            // past it, and between one layout's range and the next's it held back.)
            Content::Music(shown) => {
                if let Some(room) = sized_to {
                    shown.size = room;
                }
                shown.size = within(shown.size, music::LEAST);
                self.zoom = 1.0;
                self.shown = shown.size;
                moving |= shown.busy;
            }
            // Merged: laid out in the room it is given, no less than its parts need at their least.
            Content::Merged(merged) => {
                if let Some(room) = sized_to {
                    merged.size = room;
                }
                if !merged.lay_out(&scene, &theme, work, seen, now) {
                    return Frame::Rest;
                }
                if sized_to.is_some() {
                    merged.size = (merged.size.0.max(merged.least.0), merged.size.1.max(merged.least.1));
                }
                merged.size = within(merged.size, merged.least);
                self.zoom = 1.0;
                self.shown = merged.size;
                moving |= merged.busy();
            }
        }
        // A note lifted and leaning in hand, a note pulled off the panel turning between its card and a note: each a step
        // on (see `carry`).
        moving |= self.carry(dt / 1000.0);
        // Gone back all the way: its card on the panel again.
        let home = self.pulled.as_ref().is_some_and(|pulled| pulled.morph.rests(0.002) && pulled.back.as_ref().is_some_and(|(x, y)| x.rests(0.5) && y.rests(0.5)));
        if home && self.lift.rests(0.002) && self.pulled.as_mut().is_some_and(Pulled::land) {
            return Frame::Back;
        }
        // A note pulled off the panel: as large as it is between its card and a note.
        let fill = match &self.pulled {
            Some(pulled) => {
                let px = self.contact.scale * self.zoom;
                let card = ((pulled.card.right - pulled.card.left) as f32 / px, (pulled.card.bottom - pulled.card.top) as f32 / px);
                let m = pulled.morph.value;
                (card.0 + (self.shown.0 - card.0) * m, card.1 + (self.shown.1 - card.1) * m)
            }
            None => self.shown,
        };
        // Its surface: the room the hand gives it, past what it shows only
        // part of the way; settling from that once let go; else what it shows.
        let give = |have: f32, fill: f32| if have <= fill { fill } else { fill + (have - fill) * GIVE };
        self.surface_size = match (self.room, self.settle) {
            (Some(room), _) => (give(room.0 / self.zoom, self.shown.0), give(room.1 / self.zoom, self.shown.1)),
            (None, Some((since, from))) => {
                let t = (now.saturating_duration_since(since).as_secs_f32() / SETTLE.as_secs_f32()).min(1.0);
                let eased = morph::ease(t);
                if t >= 1.0 {
                    self.settle = None;
                    self.unsaved = true;
                }
                (from.0 + (fill.0 - from.0) * eased, from.1 + (fill.1 - from.1) * eased)
            }
            _ => fill,
        };
        // Carried: held where it is held as it turns (see `Pulled::grab_at`), in hand under the pointer, going back on its
        // way, put down where it was let go.
        let px = self.contact.scale * self.zoom;
        if let Some(pulled) = &self.pulled {
            let held = pulled.grab_at(self.surface_size);
            let grab = POINT { x: (held.0 * px).round() as i32, y: (held.1 * px).round() as i32 };
            let was = POINT { x: (self.about.0 * px).round() as i32, y: (self.about.1 * px).round() as i32 };
            match (&mut self.drag, &pulled.back) {
                (_, Some((x, y))) => self.at = POINT { x: x.value.round() as i32, y: y.value.round() as i32 },
                (Some(Drag::Move { grab: hand, .. }), None) => {
                    *hand = grab;
                    if let Some(cursor) = cursor_position() {
                        self.at = POINT { x: cursor.x - grab.x, y: cursor.y - grab.y };
                    }
                }
                _ => self.at = POINT { x: self.at.x + was.x - grab.x, y: self.at.y + was.y - grab.y },
            }
            self.about = held;
        }
        // Its room round it for its shadow and buttons, and for it lifted and leaning.
        if self.lift.value.abs() > 0.001 || self.lean.value.abs() > 0.001 {
            let reach = carried_reach(self.surface_size, self.about, 1.0 + LIFTED_BY * self.lift.value.max(0.0), self.lean.value, lifted_shadow(self.lift.value).reach());
            self.margin = self.margin.max(reach.ceil());
        }
        if let Some(Drag::Size { edges: (l, _, t, _), from: _, room, corner }) = &self.drag {
            let start = ((room.0 * self.contact.scale) as i32, (room.1 * self.contact.scale) as i32);
            let size = ((self.surface_size.0 * px) as i32, (self.surface_size.1 * px) as i32);
            self.at = POINT { x: if *l { corner.x + start.0 - size.0 } else { corner.x }, y: if *t { corner.y + start.1 - size.1 } else { corner.y } };
        }
        // Settling after it was sized by its left or top edge: the other
        // edges held where they were let go.
        let size = ((self.surface_size.0 * px) as i32, (self.surface_size.1 * px) as i32);
        if let (None, true, Some(anchor), None) = (&self.drag, settling, self.anchor, self.stuck) {
            if self.keeps_to.0 {
                self.at.x = anchor.x - size.0;
            }
            if self.keeps_to.1 {
                self.at.y = anchor.y - size.1;
            }
        }
        // Stuck to an edge: flush with it, at its stop along it. Else, at
        // rest, all of it on its screen.
        if let (Some(side), None) = (self.stuck, &self.drag) {
            self.at = self.stuck_at(side, self.along, self.also, size);
        } else if self.drag.is_none() && self.fling.is_none() {
            let work = self.contact.work;
            self.at.x = self.at.x.clamp(work.left, (work.right - size.0).max(work.left));
            self.at.y = self.at.y.clamp(work.top, (work.bottom - size.1).max(work.top));
        }
        // The pointer over it, or (its buttons showing) over them.
        let (above, under, left, right) = self.reach();
        let cursor = cursor_position();
        let hovered = cursor.is_some_and(|c| {
            let r = self.rect();
            (r.left - left..r.right + right).contains(&c.x) && (r.top - above..r.bottom + under).contains(&c.y)
        });
        // Its buttons come or go: a widget that took the desktop with them
        // there takes it again.
        if hovered != self.hover {
            let (r, m) = (self.rect(), self.margin_px());
            own_window_changed(RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m });
        }
        self.hover = hovered;
        // Letting clicks through, it hears nothing of the pointer, which is
        // looked for here: over it a while, the button that ends it shows,
        // and with the pointer on that the window takes the click.
        self.lingered = if hovered && self.click_through { self.lingered.or(Some(now)) } else { None };
        self.reached = self.click_through && cursor.is_some_and(|c| self.button_at((c.x - self.at.x) as f32 / px, (c.y - self.at.y) as f32 / px) == Some(Button::Through));
        // (What it said under the pointer goes with it: a button's, a drive's row, a name cut short.)
        if !hovered && (self.tip.take().is_some() | std::mem::take(&mut self.saying)) {
            crate::ui::tip::hide();
        }
        if !hovered && self.hit_hover.take().is_some() {
            self.next_frame = now;
        }
        // Moved near an edge, not stuck: where it would stick, as it would
        // be there, nearer and firmer the nearer it is.
        let near = match (&self.drag, self.stuck) {
            // Torn off the panel and still at its edge, it is put down where it is let go.
            (Some(Drag::Move { moved: true, torn: false, .. }), None) => self.nearest_side().filter(|(_, away)| *away < HINT_SEEN),
            _ => None,
        };
        match near {
            Some((side, away)) => {
                // As large as it is: it sticks as it is (in the corner, near enough the edge across it to stick
                // there too).
                let rect = self.rect();
                let size = (rect.right - rect.left, rect.bottom - rect.top);
                let also = corner_side(self.contact.work, rect, self.contact.scale, side, STICK);
                let POINT { x, y } = self.stuck_at(side, Some(self.middle_along(side)), also, size);
                let firm = away < STICK;
                // In steps: drawn again as it changes visibly, not each frame.
                let strength = if firm { 1.0 } else { ((0.25 + 0.5 * (1.0 - (away - STICK) / (HINT_SEEN - STICK))) * 20.0).round() / 20.0 };
                let ground = self.content.ground(self.in_place.as_ref().map(|in_place| in_place.writing.colour), look.prefs.music.ground);
                self.show_hint(RECT { left: x, top: y, right: x + size.0, bottom: y + size.1 }, firm, strength, &theme, ground);
            }
            None => self.hide_hint(),
        }
        let laid_out = started.elapsed();
        // The desktop behind, for the skins that see it, without the widget:
        // what is behind it put together where no one sees it (see `scene`),
        // before it first shows (the theme follows it), and arranged anew as
        // it moves past it, goes onto another screen, or the windows there
        // change; taken as it changes (see `collect`).
        // (On the device that draws now, before what is behind is awaited: one waited for on a device gone never
        // comes.)
        self.follow_device();
        let sees = look.skin.sees_backdrop() && self.content.skinned(look.prefs.music.ground);
        let live = look.live && sees;
        if !sees {
            self.unsee();
        }
        // A frame of it, come.
        self.collect(live, px);
        // Moved past what was taken round it (that part of it on its screen:
        // beyond, the capture is mirrored in its edge), or onto another screen;
        // in hand, near the edge of it already, so that the next is there by
        // the time it gets past (taken on another thread, the widget kept out
        // of it, nothing of it shows).
        let on_screen = clip(self.rect(), self.contact.monitor);
        // Not while it is sized by hand (or settles to its size after): what was taken serves meanwhile (twice its
        // size each way, mirrored past its edges), and it is taken again once, as it comes to rest. Taken again at
        // each size, all that a capture is read through and kept in is made again at each size too.
        let elsewhere = !sizing && self.behind.as_ref().is_some_and(|behind| {
            let taken = behind.capture.rect;
            let ahead = if self.drag.is_some() { (on_screen.right - on_screen.left).max(on_screen.bottom - on_screen.top) / 6 } else { 0 };
            // (Ahead of it as far as its screen goes: what was taken goes no further, see `Scene::arrange`.)
            let screen = self.contact.monitor;
            let past = (on_screen.left - ahead).max(screen.left) < taken.left || (on_screen.top - ahead).max(screen.top) < taken.top || (on_screen.right + ahead).min(screen.right) > taken.right || (on_screen.bottom + ahead).min(screen.bottom) > taken.bottom;
            // (What was taken may reach past its screen, onto others or where there is none: only its being on
            // another screen than it was on when it was taken counts.)
            let other_screen = self.arranged_on.is_some_and(|on| on != self.contact.monitor);
            other_screen || past
        });
        let retake = self.retake && self.drag.is_none() && !sizing;
        let moved_behind = self.windows_moved_behind(now, live);
        let due = self.behind.is_none() || self.replace.is_some() || elsewhere || retake || moved_behind;
        // None could be taken last time (the lock screen): not again at once.
        let rested = self.failed.is_none_or(|at| now.saturating_duration_since(at) >= RETAKE_EVERY);
        // Giving way to the settings, the screen copied as it is would take them in (they are in front of it); what is
        // behind put together from the windows under it never has them (see `scene`), and is taken all the same: after
        // the device was lost, the widget has nothing behind it till it is.
        let in_front_kept_out = crate::ui::scene::supported() || !self.window.yielding();
        if sees && due && rested && self.pending.is_none() && in_front_kept_out {
            // Why, for the journal (not each live refresh).
            let why = [(self.behind.is_none(), "first"), (self.replace.is_some(), "replacing what it had"), (elsewhere, "moved past what was taken"), (retake, "let go"), (moved_behind, "windows behind changed")]
                .iter()
                .filter(|(is, _)| *is)
                .map(|(_, why)| *why)
                .collect::<Vec<_>>()
                .join(", ");
            if !why.is_empty() {
                note_sparingly("retake", || format!("widget: the desktop taken again ({why})"));
            }
            self.retake = false;
            let area = self.backdrop_area();
            // Its scene made once, and kept for its life (see `life`): arranged anew, as here, it brings a frame of
            // what is there now.
            if self.backdrop.is_none() && crate::ui::scene::supported() {
                let hwnd = self.window.hwnd.0 as isize;
                let wake = std::sync::Arc::new(move || {
                    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), CAPTURED, WPARAM(0), LPARAM(0)) };
                });
                self.backdrop = crate::ui::scene::Scene::new(area, wake, &self.gfx);
            }
            self.arranged = Some(now);
            self.arranged_on = Some(self.contact.monitor);
            // In hand, light or dark holds (measured again as it is let go:
            // see `retone`).
            let moved = (elsewhere && self.drag.is_none()) || self.replace == Some(true);
            match &mut self.backdrop {
                // Without the system's capture (Windows 10 before 1903): the screen copied as it is, once, for a
                // widget that has nothing behind it yet (not shown yet, it is not in it); after, what it has stands.
                None if !crate::ui::scene::supported() => {
                    if self.behind.is_none() {
                        self.took(Capture::take(area), moved, now, px);
                    } else if let Some(behind) = &mut self.behind {
                        behind.taken = now;
                        self.replace = None;
                    }
                }
                // (It could not be put together: as when the screen cannot be taken, tried again in a while.)
                None => self.took(None, moved, now, px),
                // A frame of it as it is now awaited; nothing changed, what it has stands.
                Some(scene) => {
                    if scene.arrange(area, self.window.hwnd) || self.behind.is_none() {
                        self.pending = Some(Request { area, generation: self.generation, moved, since: now });
                        // (Its first frame looked past as the last were.)
                        self.look_again = None;
                    } else if let Some(behind) = &mut self.behind {
                        behind.taken = now;
                        self.replace = None;
                    }
                }
            }
            // Watched from now (see `Widgets::tick`): what changes there is
            // seen to.
            WIDGET_AREAS.lock().unwrap().push(area);
        }
        // Let go or come to rest: light or dark as the desktop is where it
        // is now, measured on what was taken there (none under way).
        if self.tone_due && self.drag.is_none() && self.fling.is_none() && self.settle.is_none() && self.pending.is_none() {
            self.tone_due = false;
            let rect = self.rect();
            if let Some(behind) = &mut self.behind {
                behind.tone = behind.capture.luminance(rect, px);
            }
        }
        // Nothing yet behind a skin that sees it: drawn once it is there
        // (as it shows for the first time, it shows on it).
        if sees && self.behind.is_none() {
            return Frame::Waiting;
        }
        // Drawn light or dark as the desktop behind it is, now that it has
        // been taken: a first frame drawn before that would show the other.
        let dark = theme::is_dark(look.prefs.theme, self.behind.as_ref().map(|b| b.tone.0).filter(|_| sees));
        let dark = dark || on_music;
        let theme = if dark == theme.dark { theme } else { self.content.dressed(Theme::new(look.skin, dark)) };
        let info = controller.info();
        let scene = view::Scene { hover: self.hover_shown(), hint, thumbs: &thumbs, scrolls, tab, music_apart, ..crate::panel::scene_for(controller, &info, prefs, &theme, look.lang, history, seen) };
        let taken = started.elapsed();
        self.draw(&theme, &scene, look.prefs.music.ground, now);
        // Sized by hand, and now drawn at rest: what was made for it as it was sized is let go (see `Gfx::settle`).
        if sizing {
            self.was_sized = true;
        } else if std::mem::take(&mut self.was_sized) {
            self.gfx.settle();
        }
        let whole = started.elapsed();
        if whole > SLOW_FRAME {
            let ms = |d: Duration| d.as_secs_f32() * 1000.0;
            let form = self.content.key();
            let took = self.frame_took;
            note_sparingly("slow", || {
                format!("widget: a frame took {:.1} ms: laid out {:.1}, the desktop taken {:.1}, drawn {:.1} (its surface given in {:.1}, painted {:.1}, handed to the compositor {:.1}; {form})", ms(whole), ms(laid_out), ms(taken - laid_out), ms(whole - taken), ms(took.given), ms(took.painted), ms(took.handed))
            });
        }
        // (The lyrics gliding to the next line are drawn every frame too, and the glass easing to how frosted it is.)
        let easing = self.frost_shown.is_some_and(|(shown, target, _)| shown != target);
        // (And its readings turning from as they were as a thing was carried off its station, begun as they are drawn:
        // see `render::Away`.)
        let turning = match &self.content {
            Content::Readings(readings) => readings.morph.is_some(),
            Content::Merged(merged) => merged.busy(),
            _ => false,
        };
        if moving || easing || turning || crate::ui::view::gliding_drawn() { Frame::Moving } else { Frame::Rest }
    }

    /// On the device for the screen it is on, if it is not on it (a driver reset, a device lost, or it was moved to a
    /// screen another adapter drives: see `gfx::on_screen`): its surface made anew, what it kept on the one before let
    /// go of, the desktop behind taken again (what is behind put together on the new one). Whether it has a surface.
    fn follow_device(&mut self) -> bool {
        let Ok(current) = gfx::for_area(self.rect()) else { return false };
        if !Rc::ptr_eq(&current, &self.gfx) || self.surface.is_none() {
            if self.behind.is_some() {
                note("widget: the drawing device changed; the desktop taken again");
            }
            // All it drew with, and kept, was the other device's (see `life`). (A window has one composition target:
            // the old one goes first.)
            self.gone();
            self.surface = match Surface::new(&current, self.window.hwnd) {
                Ok(surface) => Some(surface),
                Err(error) => {
                    gfx::failed("widget", &current, Some(&error));
                    None
                }
            };
            self.gfx = current;
        }
        self.surface.is_some()
    }

    /// Hidden (one shown only while a game is played, as none is), on a device no screen would be drawn for any longer
    /// (see `gfx::needed`: a laptop's discrete card, its screen unplugged meanwhile): on the one for the screen it is on
    /// now, all it held on the one before let go of (made again on this one as it shows), so that it keeps no card
    /// awake. On one still needed, it stays as it is, to show at once (it follows its screen's as it next draws).
    fn follow_device_hidden(&mut self) {
        if gfx::needed(&self.gfx) {
            return;
        }
        let Ok(current) = gfx::for_area(self.rect()) else { return };
        self.gone();
        self.gfx = current;
    }

    fn draw(&mut self, theme: &Theme, scene: &Scene, music_ground: MusicGround, now: Instant) {
        // (A to-do written on it, as it is written.)
        let (field, _) = crate::ui::todo::field(self.window.hwnd, now);
        let _timed = crate::perf::Timed::start("widget frame", || format!("{:?}", self.surface_size));
        if matches!(&self.content, Content::Readings(Readings { last: None, .. })) {
            return;
        }
        if !self.follow_device() {
            return;
        }
        self.shown();
        let px = self.contact.scale * self.zoom;
        let size = self.surface_size;
        // Its margin as whole pixels, as its window and what the pointer is over reckon it (see `margin_px`): what it
        // shows drawn from there, not from the margin in DIPs, put on the pixel nearest that (a pixel off from where
        // presses were looked for, at some scales).
        let m = self.margin_px();
        let margin = m as f32 / px;
        let window = {
            let r = self.rect();
            RECT { left: r.left - m, top: r.top - m, right: r.right + m, bottom: r.bottom + m }
        };
        // What it shows keeps to the sides not dragged; so does what it
        // showed, turning into it, at its own zoom.
        let keeps_to = self.keeps_to;
        let zoom = self.zoom;
        let placed = |shown: &Shown| {
            let k = shown.zoom / zoom;
            (k, (if keeps_to.0 { size.0 - shown.size.0 * k } else { 0.0 }, if keeps_to.1 { size.1 - shown.size.1 * k } else { 0.0 }))
        };
        let halo = (theme.skin == Skin::Glass).then_some(theme.legibility);
        let (opacity, pinned, through, gfx) = (self.opacity, self.pinned, self.click_through, self.gfx.clone());
        let drop_target = self.drop_target;
        let rough = matches!(self.drag, Some(Drag::Move { moved: true, .. })) || self.fling.is_some();
        let contact_scale = self.contact.scale;
        // (None while a note is carried: it is in hand, or settling as it was let go.)
        let carrying = self.pulled.is_some() || self.lift.value > 0.01;
        let offered = if carrying { Vec::new() } else { self.offered() };
        // A note lifted and leaning (see `carry`): drawn larger about where it is held and turned there, its shadow
        // under it; pulled off the panel, its card under it as much as it is still its card.
        let carried = self.content.stuck_note().is_some().then(|| {
            let card = self.pulled.as_ref().map(|pulled| (pulled.morph.value.clamp(0.0, 1.0), pulled.number, pulled.card_px / px));
            (card, self.lift.value, self.lean.value, self.about)
        });
        let settings_lit = self.settings_lit();
        let frost = if self.content.skinned(music_ground) { self.frost(now, theme) } else { 0.0 };
        // What plays: as its switches say, the pointer where it is over it (letting clicks through, it is not).
        let shows = (!matches!(self.content, Content::Readings(_))).then(|| Shows::from(&crate::app().settings.lock().unwrap().view));
        let pointer = cursor_position().filter(|_| self.hover && !self.click_through).map(|c| ((c.x - self.at.x) as f32 / px, (c.y - self.at.y) as f32 / px));
        // A note written in where it is: its card where it was last drawn, should none be drawn now.
        let was_card = self.in_place.as_ref().and_then(|in_place| card_in(&self.hits, self.hits_from, in_place.number, margin));
        // A thing on its station dragged off it: lifted off its room, as it was drawn there (see `carry`).
        // (With what is carried with it, those of them whose rooms show, each from its own room: see `carry::Tile`.)
        let lifting = self.lifting.take().filter(|(hit, ..)| self.hits.iter().any(|one| one.4 == *hit)).map(|(_, carried, drag, from)| {
            let boxes: Vec<(view::Hit, (f32, f32))> = carried.iter().filter_map(|one| self.hits.iter().find(|box_| box_.4 == *one).map(|&(hx, hy, ..)| (*one, (hx, hy)))).take(crate::carry::FLOWN).collect();
            (carried, drag, from, boxes)
        });
        let mut lifted = None;
        let in_place = self.in_place.as_mut();
        let mut card_gone = false;
        let behind = &mut self.behind;
        let content = &mut self.content;
        let Some(surface) = self.surface.as_mut() else { return };
        let pixels = ((window.right - window.left) as u32, (window.bottom - window.top) as u32);
        // Every layer of it drawn (see below).
        let mut whole = true;
        let mut pressed: Option<(Vec<view::HitBox>, (f32, f32))> = None;
        let drawn = surface.draw(&gfx, pixels, px, |frame| {
            frame.origin(0.0, 0.0);
            if opacity < 1.0 {
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
            }
            let held = match carried {
                Some((_, lift, lean, about)) if lift.abs() > 0.001 || lean.abs() > 0.001 => {
                    let grown = 1.0 + LIFTED_BY * lift.max(0.0);
                    Matrix3x2::translation(-about.0, -about.1) * Matrix3x2::scale(grown, grown) * Matrix3x2::rotation_around(lean, Vector2 { X: 0.0, Y: 0.0 }) * Matrix3x2::translation(about.0, about.1)
                }
                _ => Matrix3x2::identity(),
            };
            let local = held * Matrix3x2::translation(margin, margin);
            // A music widget's own ground's colour along its edges: its buttons in it.
            let mut button_ground = None;
            match content {
                Content::Readings(readings) => {
                    let Some(shown) = readings.last.clone() else { return };
                    let mut lanes = view::lanes_at(scene, |_| shown.detail);
                    view::todo::write_in(&mut lanes, &field);
                    let backdrop = behind.as_mut().and_then(|behind| behind.backdrop(&frame.dc, contact_scale, window, px, margin));
                    let at = (if keeps_to.0 { size.0 - shown.size.0 } else { 0.0 }, if keeps_to.1 { size.1 - shown.size.1 } else { 0.0 });
                    let layout = shown.drawn_at(size, carried.is_some_and(|(card, ..)| card.is_some()));
                    let picture = render::Picture { scene, lanes: &lanes, layout: &layout, small: shown.small.map(|small| (small, shown.size)), size, at, edge: None, backdrop, frost, rough };
                    if let Some((carried, _, _, boxes)) = &lifting {
                        lifted = Some(render::record_tiles(frame, scene, &lanes, &shown.layout, picture.small, 1.0, (0.0, 0.0), boxes)).filter(|tiles| tiles.first().is_some_and(|(first, ..)| *first == boxes[0].0));
                        if lifted.is_some() {
                            view::carry_tiles(carried.clone());
                        }
                    }
                    // Carried: its shadow lifted with it, and pulled off the panel, its card turning into the note over it
                    // (its card seen till it is half a note, the note from nothing to whole by then).
                    let mut page = 1.0;
                    if let Some((card, lift, ..)) = carried {
                        drawn_in(frame, local, || {
                            let turned = card.map_or(1.0, |(m, ..)| m);
                            let shadow = lifted_shadow(lift);
                            if shadow.dark > 0.0 {
                                let radius = view::note_radius(scene.theme, true) + (scene.theme.radius - view::note_radius(scene.theme, true)) * turned;
                                frame.shadow(Color::hex(0, shadow.dark), 0.0, 0.0, size.0, size.1, radius, shadow.blur, shadow.drop);
                            }
                            if let Some((m, number, k)) = card {
                                let seen = (2.0 * (1.0 - m)).min(1.0);
                                if seen > 0.0 {
                                    // (In the panel's DIPs, as large as it is.)
                                    drawn_in(frame, Matrix3x2::scale(k, k), || faded(frame, seen, || view::paint_carried_card(frame, scene, number, (0.0, 0.0, size.0 / k, size.1 / k))));
                                }
                            }
                        });
                        if let Some((m, ..)) = card {
                            page = (2.0 * m).min(1.0);
                        }
                    }
                    if page > 0.0 {
                        let mut painted = (None, true);
                        faded(frame, page, || painted = paint_readings(frame, readings, scene, &picture, &placed, local, px, halo, Some(&[]), now));
                        let (hits, drawn_whole) = painted;
                        whole &= drawn_whole;
                        pressed = hits.map(|hits| (hits, at)).or(pressed.take());
                    }
                }
                // What plays, drawn anew in its layer (round it, the room for its shadow).
                Content::Music(shown) => {
                    let MusicShown { music, layer, frames, busy, skin_ground, .. } = shown;
                    *frames += 1;
                    let shows = shows.unwrap_or_else(|| Shows::from(&serde_json::Value::Null));
                    let ground = music::Ground::of(&shows, theme.dark);
                    // On the skin: its ground under it, as every widget's is (its buttons on it, as theirs are).
                    if let music::Ground::Skin { .. } = ground {
                        let backdrop = behind.as_mut().and_then(|behind| behind.backdrop(&frame.dc, contact_scale, window, px, margin));
                        if let Err(error) = skin_ground.draw_whole(frame, theme, size, backdrop, frost, rough, local, px) {
                            note(format!("widget: what plays' ground not drawn: {error}"));
                            whole = false;
                        }
                    }
                    let area = (-MUSIC_SHADOW, -MUSIC_SHADOW, size.0 + 2.0 * MUSIC_SHADOW, size.1 + 2.0 * MUSIC_SHADOW);
                    let mut painted = None;
                    let drew = layer.draw(frame, *frames, local, area, px, None, |frame| {
                        painted = Some(music.paint(frame, size, &shows, ground, theme.body.family, px, pointer, now));
                        Ok(())
                    });
                    if let Err(error) = drew {
                        note(format!("widget: what plays not drawn: {error}"));
                        whole = false;
                        frame.fail(error);
                    }
                    if let Some(painted) = painted {
                        *busy = painted.busy;
                        pressed = Some((painted.hits, (0.0, 0.0)));
                    }
                    button_ground = (!matches!(ground, music::Ground::Skin { .. })).then(|| music.ground_colour(size));
                }
                Content::Merged(merged) => {
                    let backdrop = behind.as_mut().and_then(|behind| behind.backdrop(&frame.dc, contact_scale, window, px, margin));
                    let shows = shows.unwrap_or_else(|| Shows::from(&serde_json::Value::Null));
                    let (hits, drawn_whole) = draw_merged(frame, merged, scene, local, px, backdrop, frost, rough, pointer, &shows, now, lifting.as_ref().map(|(carried, _, _, boxes)| (carried.as_slice(), boxes.as_slice())), &mut lifted, &field);
                    whole &= drawn_whole;
                    pressed = Some((hits, (0.0, 0.0)));
                }
            }
            // A note written in where it is: its card drawn by what writes it, where the card is now (its words' box
            // placed there, written as large as its scale makes them). Its card gone (the note taken off it, a part
            // torn out), given back.
            if let Some(in_place) = in_place {
                match pressed.as_ref().map_or(was_card, |(hits, from)| card_in(hits, *from, in_place.number, margin)) {
                    Some(card) => {
                        // (Placed first: restyled, its tables are laid out in the room it has.)
                        in_place.writing.place(words_rect(card, px));
                        if in_place.styled != (px, theme.skin, theme.dark) {
                            in_place.styled = (px, theme.skin, theme.dark);
                            in_place.writing.restyle(theme, crate::ui::note::WORDS * px);
                        }
                        in_place.card = card;
                        frame.origin(0.0, 0.0);
                        // (Lifted and leaning as the rest of it is, carried: its card is placed from the window's
                        // corner, the rest from inside its margin.)
                        drawn_in(frame, Matrix3x2::translation(-margin, -margin) * local, || {
                            in_place.writing.paint(frame, theme, scene.lang, card, (0.0, 0.0), px, in_place.taking && in_place.caret_on(now));
                            if in_place.taking {
                                let place = in_place.menu_button();
                                let lit = cursor_position().is_some_and(|c| {
                                    let (x, y) = ((c.x - window.left) as f32 / px, (c.y - window.top) as f32 / px);
                                    x >= place.0 && x < place.0 + place.2 && y >= place.1 && y < place.1 + place.3
                                });
                                crate::ui::note::paint_button(frame, theme, place, Icon::Ellipsis, lit);
                            }
                        });
                    }
                    None => card_gone = true,
                }
            }
            frame.place(Matrix3x2::translation(margin, margin));
            if let Some(beside) = drop_target {
                let ring = theme.text.alpha(0.85);
                frame.stroke_rounded(Fill::Solid(ring), -5.25, -5.25, size.0 + 10.5, size.1 + 10.5, 19.25, 2.5);
                if beside {
                    frame.fill_rounded(ring.alpha(0.5), size.0 - 10.0, 10.0, 5.0, size.1 - 20.0, 2.5);
                } else {
                    frame.fill_rounded(ring.alpha(0.5), 10.0, size.1 - 10.0, size.0 - 20.0, 5.0, 2.5);
                }
            }
            // The pointer over it: its buttons (letting clicks through, the one that ends that).
            let front_is_game = crate::ui::view::game_shown(scene.history);
            for (centre, button) in offered {
                // Lit while on: pinned, letting clicks through, the window in front a game.
                // (Its settings' button, while the settings asked from it are open.)
                let lit = (button == Button::Pin && pinned) || (button == Button::Through && through) || (button == Button::Game && front_is_game) || (button == Button::Settings && settings_lit.is_some());
                let grown = if button == Button::Settings { settings_lit.unwrap_or(1.0) } else { 1.0 };
                match button_ground {
                    Some(ground) => buttons::paint_on(frame, centre, button, lit, ground, grown),
                    None => buttons::paint(frame, centre, button, lit, theme.dark, theme.text3.alpha(0.5), grown),
                }
            }
            frame.origin(0.0, 0.0);
            if opacity < 1.0 {
                unsafe { frame.dc.PopLayer() };
            }
        });
        self.frame_took = surface.took;
        if card_gone {
            self.stop_writing(false);
            self.in_place = None;
        }
        if let Some((hits, from)) = pressed {
            (self.hits, self.hits_from) = (hits, from);
            self.show_stations();
        }
        if let (Some((carried, drag, from, _)), Some(recorded)) = (lifting, lifted) {
            // (Each in its room on the screen, its box as large as it is drawn there. Its surface's corner is at `at`,
            // the window's margin before it.)
            let tiles: Vec<crate::carry::Tile> = recorded
                .into_iter()
                .filter_map(|(hit, parts, home)| {
                    let &(hx, hy, hw, hh, _) = self.hits.iter().find(|one| one.4 == hit)?;
                    let at = POINT { x: (self.at.x as f32 + (self.hits_from.0 + hx) * px).round() as i32, y: (self.at.y as f32 + (self.hits_from.1 + hy) * px).round() as i32 };
                    Some(crate::carry::Tile { hit, parts, home, size: (hw, hh), at })
                })
                .collect();
            if !tiles.is_empty() {
                let count = crate::station::select::landing(&carried);
                let lifted = crate::carry::Lifted { carried, count, drag, tiles, from, px, over: self.rect(), window: self.window.hwnd, look: self.menu_look() };
                crate::carry::begin(&self.gfx, lifted);
            }
        }
        // (Not drawn: drawn again on its surface; the device gone, on one made anew on the next.)
        if let Err(error) = &drawn {
            if gfx::failed("widget", &self.gfx, Some(error)) {
                self.surface = None;
            }
        }
        gfx.sweep();
        // Placed and shown once drawn: never the last picture in the new
        // place, nor an empty window, on the screen (not shown at all till
        // a frame was drawn whole).
        let whole = drawn.is_ok() && whole;
        if !self.shown_once && !whole {
            return;
        }
        self.window.place_keeping(window);
        self.window.show_in_place();
        if self.tip.is_some() {
            crate::ui::tip::above();
        }
        // (A note stuck from its page shown: the page goes, now that it is here.)
        if !self.shown_once && whole {
            if let Some(name) = self.content.stuck_note() {
                crate::ui::beside::stuck_shown(name);
            }
        }
        self.shown_once |= whole;
    }

    /// How frosted its glass is drawn: as the desktop where it is now asks (its theme holds, as the panel's does),
    /// measured again as it moves a step; eased to it, not stepped (moved a little onto a busier or calmer part of
    /// the desktop, the glass frosts or clears over a moment rather than at once).
    fn frost(&mut self, now: Instant, theme: &Theme) -> f32 {
        let contact_scale = self.contact.scale;
        let here = self.rect();
        let step = (16.0 * contact_scale) as i32;
        let target = match (&self.behind, self.frost_at) {
            (None, _) => 0.0,
            (Some(_), Some((at, frost))) if (at.left - here.left).abs() < step && (at.top - here.top).abs() < step && (at.right - here.right).abs() < step && (at.bottom - here.bottom).abs() < step => frost,
            (Some(behind), _) => {
                let (mean, spread) = behind.capture.luminance(here, contact_scale);
                let frost = (skins::frost(mean, spread, theme.dark) * 20.0).round() / 20.0;
                self.frost_at = Some((here, frost));
                frost
            }
        };
        let frost = match self.frost_shown {
            Some((shown, _, at)) => {
                let step = now.saturating_duration_since(at).as_secs_f32() / FROST_EASE.as_secs_f32();
                if target > shown { (shown + step).min(target) } else { (shown - step).max(target) }
            }
            None => target,
        };
        self.frost_shown = Some((frost, target, now));
        frost
    }

    /// Its menu, up (see `menu_look`).
    fn menu(&self, lang: Lang, over_the_panel: bool) -> Option<crate::ui::menu::Menu> {
        crate::ui::menu::Menu::open(menu_items(lang, self.pinned, self.click_through, !self.on_desktop, self.shows_full()), self.menu_look(), over_the_panel)
    }

    /// How a menu opened on it looks: in its skin, light or dark, as frosted as it was last drawn and as large.
    fn menu_look(&self) -> crate::ui::menu::Look {
        let frost = self.frost_shown.map_or(1.0, |(shown, _, _)| shown);
        crate::ui::menu::Look::of(self.drawn_look.0, self.drawn_look.1).frosted(frost, self.behind.is_some()).zoomed(self.zoom)
    }

    /// Whether it shows what plays, alone or merged with others (and so offers it over the whole screen). (Not by its
    /// buttons: merged, it takes those of the readings, which have no full screen.)
    fn shows_full(&self) -> bool {
        match &self.content {
            Content::Music(_) => true,
            Content::Merged(merged) => merged.parts.iter().any(|part| matches!(part, Content::Music(_))),
            Content::Readings(_) => false,
        }
    }
}

/// Where a widget let go would stick, `size` DIPs from the canvas's origin,
/// as `theme`'s skin draws it (or, a widget with a ground of its own, in that
/// ground's colour and corners, one with a colour of its own, as Windows 11's
/// is drawn but in that colour): firmer once letting go would stick it there,
/// `strength` as strong.
pub(crate) fn paint_hint(canvas: &dyn Canvas, theme: &Theme, (w, h): (f32, f32), firm: bool, strength: f32, ground: Option<Ground>) {
    if let Some(Ground::Own([r, g, b])) = ground {
        let (fill, rim) = (Color { r, g, b, a: (if firm { 0.85 } else { 0.5 }) * strength }, Color::hex(0xFFFFFF, (if firm { 0.3 } else { 0.18 }) * strength));
        let radius = crate::ui::music::RADIUS;
        canvas.fill_rounded(fill, 0.5, 0.5, w - 1.0, h - 1.0, radius);
        canvas.stroke_rounded(Fill::Solid(rim), 0.0, 0.0, w, h, radius + 0.5, 1.0);
        return;
    }
    let (fill, line, width, dashed) = match (theme.skin, ground) {
        // Its own colour's wash under a hairline of it.
        (_, Some(Ground::Tint(own))) => (own.alpha(if firm { 0.26 } else { 0.14 }), own.alpha(if firm { 1.0 } else { 0.45 }), 1.0, false),
        // A pencilled outline on the paper's own colour.
        (Skin::Paper, _) => (theme.paper.alpha(if firm { 0.75 } else { 0.45 }), if firm { theme.text2 } else { theme.text3 }, 1.5, !firm),
        // As Windows' own snap preview: the accent's wash under a hairline.
        (Skin::Fluent, _) => {
            let accent = theme.ink("cpu").trace;
            (accent.alpha(if firm { 0.26 } else { 0.14 }), accent.alpha(if firm { 1.0 } else { 0.45 }), 1.0, false)
        }
        // A pane of glass, its rim catching the light.
        (Skin::Glass, _) => {
            let rim = if theme.dark { Color::hex(0xFFFFFF, 0.2) } else { Color::hex(0xFFFFFF, 0.7) };
            (theme.glass.alpha(if firm { 0.6 } else { 0.32 }), rim, 1.0, false)
        }
    };
    let (fill, line) = (fill.alpha(strength), line.alpha(strength));
    let inset = width / 2.0;
    let radius = theme.radius;
    canvas.fill_rounded(fill, inset, inset, w - width, h - width, radius);
    if dashed {
        canvas.stroke_dashed(&rounded(inset, inset, w - width, h - width, radius), line, width);
    } else {
        canvas.stroke_rounded(Fill::Solid(line), 0.0, 0.0, w, h, radius + width / 2.0, width);
    }
}

/// What a widget's menu offers, row by row (`full`: it shows what plays, and offers it over the whole screen).
/// Whether a widget of readings that showed `last` turns into `shown` (see `morph`), rather than showing it at once: as
/// it shows another form, or other lanes. The workbench's lanes, laid out for the size it is made, also as their
/// things flow anew across its columns (cards and quick launch's things a row more or less) or its lanes take other
/// shapes (see `view::Shape`), but not while it is sized by hand (`by_hand`): laid out again for each size the hand
/// makes, it shows that at once, as the hand makes it (turning behind the hand, each turn from one still turning, what
/// it drew showed through faded twice over, and its cards' grounds were drawn lighter and darker as they faded).
fn turns_into(last: &Shown, shown: &Shown, workbench: bool, by_hand: bool, theme: &Theme) -> bool {
    if workbench && by_hand {
        return false;
    }
    let flowed = workbench && (last.layout.flow(theme) != shown.layout.flow(theme) || last.layout.shapes(theme) != shown.layout.shapes(theme));
    last.form != shown.form || last.layout.heights.len() != shown.layout.heights.len() || flowed
}

fn menu_choices(full: bool) -> Vec<WidgetChoice> {
    let mut choices = vec![WidgetChoice::Pin, WidgetChoice::ClickThrough, WidgetChoice::AboveOthers];
    if full {
        choices.push(WidgetChoice::Full);
    }
    choices.extend([WidgetChoice::Close, WidgetChoice::Settings]);
    choices
}

/// A widget's menu, as `menu_choices` orders it: pinned or not, letting clicks through or not, above the others or
/// not; what plays over the whole screen, where it shows that.
pub(crate) fn menu_items(lang: Lang, pinned: bool, through: bool, above: bool, full: bool) -> Vec<crate::ui::menu::Item<'static>> {
    use crate::ui::menu::Item;
    let item = |zh, en, icon, checked, rule_before| Item { label: lang.pick(zh, en), mark: Some(crate::ui::menu::Mark::Icon(icon)), checked, rule_before };
    menu_choices(full)
        .into_iter()
        .map(|choice| match choice {
            WidgetChoice::Pin => item("固定位置和大小", "Pin where it is", Icon::Pin, pinned, false),
            WidgetChoice::ClickThrough => item("鼠标穿透", "Let clicks through", Icon::Pointer, through, false),
            WidgetChoice::AboveOthers => item("置于其他窗口之上", "Keep above other windows", Icon::Layers, above, false),
            WidgetChoice::Full => item("全屏播放", "Full screen", Icon::Maximize, false, false),
            WidgetChoice::Close => item("收起", "Put away", Icon::Close, false, false),
            WidgetChoice::Settings => item("设置…", "Settings…", Icon::Settings, false, true),
        })
        .collect()
}

/// The least rectangle holding both.
fn union(a: RECT, b: RECT) -> RECT {
    RECT { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) }
}

/// `r` within `bounds`.
fn clip(r: RECT, bounds: RECT) -> RECT {
    let left = r.left.clamp(bounds.left, bounds.right);
    let top = r.top.clamp(bounds.top, bounds.bottom);
    RECT { left, top, right: r.right.clamp(left, bounds.right), bottom: r.bottom.clamp(top, bounds.bottom) }
}

/// The game button pressed (a widget's or the panel's): showing a game (found or asked for), off; else on.
pub(crate) fn press_game() {
    let latest = crate::app().controller.history.latest();
    let shown = crate::ui::view::game_shown(latest.as_ref().map(std::slice::from_ref).unwrap_or_default());
    crate::source::set_game_mode(if shown { crate::presents::GameMode::Off } else { crate::presents::GameMode::On });
}

/// The outline of a rounded rectangle, as points round it, closed.
fn rounded(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Vec<Point> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let corners = [(x + w - r, y + r, -0.25), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 0.25), (x + r, y + r, 0.5)];
    let mut points: Vec<Point> = corners
        .iter()
        .flat_map(|&(cx, cy, from)| {
            (0..=8).map(move |i| {
                let angle = std::f32::consts::TAU * (from + 0.25 * i as f32 / 8.0);
                Point { x: cx + r * angle.cos(), y: cy + r * angle.sin() }
            })
        })
        .collect();
    points.push(points[0]);
    points
}

/// What a widget's buttons do: the window in front taken for a game (one
/// played in a window, that would not be found), the settings, pinned,
/// put away.
#[cfg(test)]
mod tests {
    use super::*;

    /// A note pulled off the panel is its card while the pointer is over the panel, and a note off it (none to be had:
    /// off it). Held where it was pressed on its card, across by share as it grows; down, as far as on its card while it
    /// is one, drawn up into a note's head as it becomes one (pressed high on its card already, no further).
    #[test]
    fn a_note_pulled_off_the_panel_is_a_note_only_off_it_held_by_its_head() {
        let card = RECT { left: 120, top: 400, right: 290, bottom: 504 };
        let mut pulled = Pulled { number: 1, over: RECT { left: 100, top: 100, right: 500, bottom: 900 }, card, card_px: 1.5, grab: (0.25, 80.0), morph: Spring::at(0.0, CARD_TO_NOTE.0, CARD_TO_NOTE.1), back: None, landed: false };
        let at = |x| Some(POINT { x, y: 450 });
        assert_eq!(pulled.wants(at(300)), 0.0);
        assert_eq!(pulled.wants(at(499)), 0.0);
        assert_eq!(pulled.wants(at(500)), 1.0);
        assert_eq!(pulled.wants(None), 1.0);
        assert_eq!(pulled.grab_at((170.0, 104.0)), (42.5, 80.0));
        pulled.morph.value = 1.0;
        assert_eq!(pulled.grab_at((360.0, 320.0)), (90.0, crate::ui::note::HEAD / 2.0));
        pulled.morph.value = 0.5;
        assert_eq!(pulled.grab_at((360.0, 320.0)).1, (80.0 + crate::ui::note::HEAD / 2.0) / 2.0);
        pulled.grab.1 = 10.0;
        pulled.morph.value = 1.0;
        assert_eq!(pulled.grab_at((360.0, 320.0)).1, 10.0);
    }

    /// What is drawn under a carried note (its shadow, its card) is placed where the note is, and the frame left placed
    /// as it was for the note drawn over it, which places itself from there: a note on the desktop was drawn its
    /// window's margin too far right and down, away from its buttons.
    #[test]
    fn what_is_drawn_under_a_carried_note_leaves_the_frame_as_it_was() {
        let _device = gfx::held().unwrap();
        let gfx = Gfx::new().expect("graphics");
        let (mut was, mut within, mut after) = (Matrix3x2::default(), Matrix3x2::default(), Matrix3x2::default());
        gfx.draw_offscreen((64, 64), 1.0, |frame| {
            frame.origin(0.0, 0.0);
            unsafe { frame.dc.GetTransform(&mut was) };
            drawn_in(frame, Matrix3x2::translation(12.0, 7.0), || unsafe { frame.dc.GetTransform(&mut within) });
            unsafe { frame.dc.GetTransform(&mut after) };
        })
        .unwrap();
        // (The device let go before anything is asserted: held by a test that fails, it holds the process.)
        drop(gfx);
        assert_eq!((within, after), (Matrix3x2::translation(12.0, 7.0) * was, was));
    }

    /// A note neither lifted nor leaning reaches no further than its shadow; lifted, as far as it grows about where it
    /// is held (none on the side it is held at); leaning, as far as its farthest corner turns.
    #[test]
    fn a_note_carried_has_room_for_how_it_is_drawn() {
        let size = (200.0, 100.0);
        assert_eq!(carried_reach(size, (50.0, 20.0), 1.0, 0.0, 6.0), 6.0);
        // Grown by a tenth about its top left corner: past its right edge by a tenth of its width, nothing elsewhere.
        assert!((carried_reach(size, (0.0, 0.0), 1.1, 0.0, 0.0) - 20.0).abs() < 1e-3);
        // Turned a quarter about its middle: its long sides stand up, half of its width less half its height past.
        assert!((carried_reach(size, (100.0, 50.0), 1.0, 90.0, 0.0) - 50.0).abs() < 1e-3);
    }

    /// A widget hidden (shown only while a game is played) on a device no screen would be drawn for any longer (a
    /// laptop's discrete card, its screen unplugged) goes onto its screen's as the screens change, all it held on that
    /// card let go of, and the card's device can be: kept, it kept the card awake till a game was played. One on a
    /// device a screen needs stays as it is, to show at once.
    #[test]
    fn a_hidden_widget_lets_go_of_a_card_no_screen_needs() {
        let _device = crate::ui::gfx::held().unwrap();
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let contact = Contact { monitor: RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 }, scale: 1.5 };
        let mut widget = Widget::new(contact, POINT { x: 400, y: 300 }, Content::Readings(Readings::new(choice))).unwrap();
        let screens = Rc::downgrade(&widget.gfx);
        widget.follow_device_hidden();
        assert!(std::ptr::eq(Rc::as_ptr(&widget.gfx), screens.as_ptr()) && widget.surface.is_some(), "a hidden widget on a device a screen needs moved off it");
        let held = gfx::in_software();
        let stray = Rc::downgrade(&held);
        widget.gone();
        widget.gfx = held;
        widget.follow_device_hidden();
        gfx::let_go_of_strays();
        assert!(gfx::needed(&widget.gfx), "a hidden widget left on a device no screen needs");
        assert!(stray.upgrade().is_none(), "a device no screen needs kept");
        drop(widget);
    }

    /// A widget's settings' button shows while the settings asked from it are open, the pointer away from it, lit: they
    /// pour back into it as they close, moved with it where the widget is moved meanwhile. Asked from elsewhere, or
    /// closed, it shows no more.
    #[test]
    fn its_settings_button_shows_while_the_settings_asked_from_it_are_open() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let contact = Contact { monitor: RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 }, scale: 1.5 };
        let mut widget = Widget::new(contact, POINT { x: 400, y: 300 }, Content::Readings(Readings::new(choice))).unwrap();
        widget.surface_size = (300.0, 200.0);
        widget.hover = false;
        let gear = widget.button_rect(Button::Settings).unwrap();
        let settings_shown = |widget: &Widget| widget.offered().iter().any(|(_, button)| *button == Button::Settings);
        // (Pressed where it shows: the pointer taken there, not let through to what is under it.)
        let takes_it = |widget: &Widget, gear: RECT| {
            let (taking, x, y) = (widget.taking(), (gear.left + gear.right) / 2, (gear.top + gear.bottom) / 2);
            (taking.left..taking.right).contains(&x) && (taking.top..taking.bottom).contains(&y)
        };
        let closed = (settings_shown(&widget), takes_it(&widget, gear));
        crate::ui::settings_window::pretend_asked_from(Some(gear));
        let asked_here = (settings_shown(&widget), widget.button_on_screen(Button::Settings), takes_it(&widget, gear));
        widget.follow_settings();
        widget.at.x += 200;
        widget.at.y -= 50;
        widget.follow_settings();
        let moved_gear = widget.button_rect(Button::Settings).unwrap();
        let moved = (settings_shown(&widget), crate::ui::settings_window::asked_from(), takes_it(&widget, moved_gear));
        crate::ui::settings_window::pretend_asked_from(Some(RECT { left: 10, top: 10, right: 40, bottom: 40 }));
        let asked_elsewhere = settings_shown(&widget);
        crate::ui::settings_window::pretend_asked_from(None);
        let closed_again = settings_shown(&widget);
        // (Its window and device let go of before anything is asserted: a thread ending with them held hangs.)
        drop(widget);
        gfx::let_go();
        assert_eq!(closed, (false, false));
        assert_eq!(asked_here, (true, Some(gear), true));
        assert_ne!(moved_gear, gear);
        assert_eq!(moved, (true, Some(moved_gear), true));
        assert!(!asked_elsewhere && !closed_again);
    }

    /// A note pulled off the panel is carried while it is in hand and while it goes back, its room outlined there; gone
    /// back all the way, it lands, carried no more (its card drawn on the panel again), and goes only after that.
    #[test]
    fn a_note_gone_back_lands_before_it_goes() {
        let card = RECT { left: 120, top: 400, right: 290, bottom: 504 };
        let mut pulled = Pulled { number: 1, over: RECT { left: 100, top: 100, right: 500, bottom: 900 }, card, card_px: 1.5, grab: (0.25, 80.0), morph: Spring::at(0.0, CARD_TO_NOTE.0, CARD_TO_NOTE.1), back: None, landed: false };
        assert!(pulled.carried(true) && !pulled.carried(false));
        pulled.back = Some((Spring::at(0.0, GOING_BACK.0, GOING_BACK.1), Spring::at(0.0, GOING_BACK.0, GOING_BACK.1)));
        assert!(pulled.carried(false));
        assert!(!pulled.land());
        assert!(!pulled.carried(false));
        assert!(pulled.land());
    }

    /// A note pulled off the panel and let go over it again goes back to its card over the panel, the panel come to
    /// the top meanwhile (as it does while it is open): kept under it as a widget is, it went back behind it, seen
    /// through the panel's glass.
    #[test]
    fn a_note_let_go_over_the_panel_goes_back_over_it() {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_HWNDNEXT};
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let contact = Contact { monitor: RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 }, scale: 1.5 };
        let content = Content::Readings(Readings { workbench: true, only: Some(format!("{}a.md", view::STUCK_NOTE_LANE)), ..Readings::new(choice) });
        let mut widget = Widget::new(contact, POINT { x: 400, y: 300 }, content).unwrap();
        // (The panel over everywhere the pointer may be: let go, it is over the panel.)
        let everywhere = RECT { left: -100_000, top: -100_000, right: 100_000, bottom: 100_000 };
        let card = RECT { left: 120, top: 400, right: 290, bottom: 504 };
        widget.pulled = Some(Pulled { number: 1, over: everywhere, card, card_px: 1.5, grab: (0.25, 80.0), morph: Spring::at(0.0, CARD_TO_NOTE.0, CARD_TO_NOTE.1), back: None, landed: false });
        widget.drag = Some(Drag::Move { grab: POINT::default(), trail: Vec::new(), torn: false, moved: true });
        widget.window.lift(true);
        let panel = Window::new().unwrap();
        let place = RECT { left: 0, top: 0, right: 600, bottom: 900 };
        panel.place(place);
        widget.release(Instant::now(), false);
        // (Its capture's end, after it let go, as Windows tells it.)
        widget.release(Instant::now(), true);
        let going_back = widget.pulled.as_ref().is_some_and(|pulled| pulled.back.is_some());
        panel.place(place);
        // Over the panel: the panel found under it.
        let mut under = unsafe { GetWindow(widget.window.hwnd, GW_HWNDNEXT) }.ok();
        let mut over_it = false;
        while let Some(window) = under {
            if window == panel.hwnd {
                over_it = true;
                break;
            }
            under = unsafe { GetWindow(window, GW_HWNDNEXT) }.ok();
        }
        // (Its window and device let go of before anything is asserted: a thread ending with them held hangs.)
        drop(panel);
        drop(widget);
        gfx::let_go();
        assert!(going_back);
        assert!(over_it);
    }

    /// A note stuck on the desktop found where it is alone, and where it was merged with what plays (which part); one
    /// not there not found.
    #[test]
    fn finds_a_stuck_note_merged_or_alone() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let note = |name: &str| Content::Readings(Readings { workbench: true, only: Some(format!("{}{name}", view::STUCK_NOTE_LANE)), ..Readings::new(choice.clone()) });
        assert_eq!(note("a.md").holding("note:a.md"), Some(None));
        let merged = Content::Merged(Box::new(Merged::new(vec![Content::Music(MusicShown::new(music::LEAST)), note("a.md")], true, music::LEAST)));
        assert_eq!(merged.holding("note:a.md"), Some(Some(1)));
        assert_eq!(merged.holding("note:b.md"), None);
    }

    /// Where a widget would stick, it is drawn in its own colour: a note's in the note's (as it is being written in, the
    /// colour chosen there), what plays in its ground's (on the skin, the skin's), readings in none of their own (the
    /// skin's).
    #[test]
    #[cfg(feature = "studio")]
    fn its_hint_is_drawn_in_its_own_colour() {
        use crate::notes::{Colour, Note};
        let _one = crate::pretending();
        crate::notes::pretend(vec![Note { name: "a.md".into(), text: "a".into(), colour: Colour::Pink, stamp: Default::default() }]);
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let note = |name: &str| Content::Readings(Readings { workbench: true, only: Some(format!("{}{name}", view::STUCK_NOTE_LANE)), ..Readings::new(choice.clone()) });
        let cover = MusicGround::Cover;
        assert_eq!(note("a.md").ground(None, cover), Some(Ground::Tint(view::note_colour(Colour::Pink))));
        assert_eq!(note("a.md").ground(Some(Colour::Green), cover), Some(Ground::Tint(view::note_colour(Colour::Green))));
        assert_eq!(Content::Readings(Readings::new(choice.clone())).ground(None, cover), None);
        let music = Content::Music(MusicShown::new(music::LEAST));
        assert!(matches!(music.ground(None, cover), Some(Ground::Own(_))) && matches!(music.ground(None, MusicGround::Clear), Some(Ground::Own(_))));
        assert_eq!(music.ground(None, MusicGround::Skin), None);
    }

    /// A note stuck on the desktop (its card the whole widget, to its edges): sized by its edges and corners, the pointer
    /// there saying so (it was the hand of the card under it, and the note seemed not to be sized at all), its words
    /// pressed to be written in, the I-beam over them, and elsewhere moved, or its card pressed, the hand. Pinned, its card
    /// pressed, never sized.
    #[test]
    fn a_stuck_note_is_sized_by_its_edges_and_says_so() {
        use windows::Win32::UI::WindowsAndMessaging::{IDC_HAND, IDC_IBEAM, IDC_SIZENWSE, IDC_SIZEWE};
        let (w, h) = view::STUCK_NOTE;
        let (wx, wy, ww, wh) = crate::ui::note::words_in(w, h);
        let hits = vec![(wx, wy, ww, wh, view::Hit::Words(7)), (0.0, 0.0, w, h, view::Hit::Note(7))];
        let hit_at = |(x, y): (f32, f32)| hits.iter().find(|hit| x >= hit.0 && x < hit.0 + hit.2 && y >= hit.1 && y < hit.1 + hit.3).map(|hit| hit.4);
        let at = |pinned: bool, point: (f32, f32)| {
            let taken = takes(&hits, (0.0, 0.0), (w, h), pinned, false, point);
            (pointer_for(&taken, false, hit_at(point), false), taken)
        };
        let (corner, side, words, head) = ((w - 2.0, h - 2.0), (1.0, h / 2.0), (wx + 40.0, wy + 30.0), (w / 2.0, 20.0));
        assert!(matches!(at(false, corner), (shape, Takes::Edges((false, true, false, true))) if shape == IDC_SIZENWSE));
        assert!(matches!(at(false, side), (shape, Takes::Edges((true, false, false, false))) if shape == IDC_SIZEWE));
        assert!(matches!(at(false, words), (shape, Takes::Whole) if shape == IDC_IBEAM));
        assert!(matches!(at(false, head), (shape, Takes::Whole) if shape == IDC_HAND));
        assert!(matches!(at(true, corner), (shape, Takes::Pressed) if shape == IDC_HAND));
    }

    /// A note stuck on the desktop, its card found among what a widget drew (from its content's corner, in its window
    /// past its margin) where its words are there to be written in, and none where they are not (a note's card on the
    /// workbench, opened beside), its words' box placed where its card draws them, at any scale.
    #[test]
    fn a_stuck_note_s_card_is_found_and_written_where_its_words_are() {
        let (wx, wy, ww, wh) = crate::ui::note::words_in(300.0, 220.0);
        let hits = vec![(wx, wy, ww, wh, view::Hit::Words(7)), (0.0, 0.0, 300.0, 220.0, view::Hit::Note(7)), (0.0, 230.0, 300.0, 40.0, view::Hit::Note(8))];
        let card = (14.0, 16.0, 300.0, 220.0);
        assert_eq!(card_in(&hits, (4.0, 6.0), 7, 10.0), Some(card));
        assert_eq!(card_in(&hits, (4.0, 6.0), 8, 10.0), None);
        for px in [1.0, 1.25, 1.5, 2.0] {
            let words = words_rect(card, px);
            let drawn = [(card.0 + wx) * px, (card.1 + wy) * px, (card.0 + wx + ww) * px, (card.1 + wy + wh) * px];
            let placed = [words.left, words.top, words.right, words.bottom].map(|edge| edge as f32);
            assert!(drawn.iter().zip(placed).all(|(drawn, placed)| (drawn - placed).abs() <= 1.0), "{px}: {drawn:?} {placed:?}");
        }
    }

    /// A press on a long list's bar takes its thumb, to be dragged along it, pinned or not (as a track is), each list
    /// with a bar in each of its views, the hand over it, as over a track. (A press there moved the widget, or, pinned,
    /// did nothing.)
    #[test]
    fn a_press_on_a_list_s_bar_takes_its_thumb() {
        use windows::Win32::UI::WindowsAndMessaging::IDC_HAND;
        for (what, _, hits) in view::tests::lists_with_bars(0.0) {
            let &(x, y, w, h, _) = hits.iter().find(|hit| matches!(hit.4, view::Hit::List(..))).unwrap();
            let point = (x + w + 7.0, y + h / 2.0);
            for pinned in [false, true] {
                let taken = takes(&hits, (0.0, 0.0), (1000.0, 1000.0), pinned, false, point);
                assert!(matches!(taken, Takes::Bar((.., view::Hit::Bar(..)))), "{what}, pinned {pinned}");
                let over = view::under(&hits, point).next().map(|hit| hit.4).filter(|hit| hit.takes_press());
                assert!(pointer_for(&taken, false, over, false) == IDC_HAND, "{what}, pinned {pinned}");
            }
        }
    }

    /// Readings merged with what plays, drawn again from their layers as they were (nothing of them changed, as in most
    /// frames while it plays, what plays drawn anew beside them): a press, the pointer and the wheel find on them what
    /// they found as they were drawn anew, the wheel over their station's long list scrolling it. (Such a frame gave
    /// what plays' alone: nothing on the readings was found till they were next drawn anew.) Drawn as `draw_merged`
    /// draws them (what plays is drawn only with Glance running).
    #[test]
    fn readings_merged_with_what_plays_are_found_as_it_plays() {
        let _device = gfx::held().unwrap();
        let _pretending = crate::pretending();
        let item = |id: u64| crate::station::shelf::Item { id, what: crate::station::shelf::What::Text { text: format!("{id}") }, added_ms: 0, pinned: false, copied: false, stays_here: false, own_bytes: 0, from: None };
        crate::station::run::pretend((1..=40).map(item).collect(), crate::settings::ShelfView::List);
        let theme = workbench_dressed(Theme::new(Skin::Fluent, false));
        let (info, prefs) = (view::tests::info(), Prefs::resolve(&serde_json::Value::Null, &["station".to_string()]));
        let history: &'static [Arc<Sample>] = Box::leak(Box::new([Arc::new(view::tests::sample(None, None, None))]));
        let seen = Seen::of(history);
        let scene = Scene { music_apart: true, hint: None, info: &info, prefs: &prefs, theme: &theme, lang: Lang::En, history, seen: &seen, pen_ms: 0.0, scrolls: view::Scrolls::default(), tab: view::Tab::Shelf, hover: None, thumbs: &view::RESTING, pinned: false, overlay: false, buttons: true, newer: false, settings_lit: None };
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let readings = Content::Readings(Readings { workbench: true, ..Readings::new(choice) });
        let mut merged = Merged::new(vec![readings, Content::Music(MusicShown::new(music::LEAST))], false, (600.0, 700.0));
        let now = Instant::now();
        assert!(merged.lay_out(&scene, &theme, (1920.0, 1080.0), &seen, now));
        let size = merged.size;
        let Content::Readings(readings) = &mut merged.parts[0] else { unreachable!() };
        let shown = readings.last.clone().unwrap();
        let lanes = view::lanes_at(&scene, |_| shown.detail);
        let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &shown.layout, small: None, size: (size.0 / shown.zoom, size.1 / shown.zoom), at: (0.0, 0.0), edge: None, backdrop: None, frost: 0.0, rough: false };
        let gfx = Gfx::new().expect("graphics");
        let mut frames = Vec::new();
        for _ in 0..2 {
            gfx.draw_offscreen((800, 900), 1.0, |frame| {
                let keep = |old: &Shown| (old.zoom / shown.zoom, (0.0, 0.0));
                let (drawn, _) = paint_readings(frame, readings, &scene, &picture, &keep, Matrix3x2::identity(), shown.zoom, None, Some(&[][..]), now);
                frames.push((drawn.is_some(), readings.found(drawn)));
            })
            .expect("drawn");
        }
        // (The device let go before anything is asserted: held by a test that fails, it holds the process.)
        drop(merged);
        drop(gfx);
        assert_eq!(frames.iter().map(|(anew, _)| *anew).collect::<Vec<_>>(), [true, false], "drawn anew, then as it was");
        for (frame, (_, hits)) in frames.iter().enumerate() {
            let Some(&(x, y, w, h, _)) = hits.iter().find(|hit| matches!(hit.4, view::Hit::List(view::List::Shelf, _))) else { panic!("frame {frame}: no list found on the readings in {hits:?}") };
            let under: Vec<view::Hit> = view::under(hits, (x + w / 2.0, y + h / 2.0)).map(|hit| hit.4).collect();
            let mut scrolls = view::Scrolls::default();
            assert!(view::wheel(&mut scrolls, &under, -120, false) && scrolls.to(view::List::Shelf) > 0.0, "frame {frame}");
        }
        assert_eq!(frames[0].1, frames[1].1);
    }

    /// A widget of one of the panel's lanes alone has no bar under it (how long the PC has been up is not about that
    /// lane), in every skin; one of all its lanes keeps the panel's.
    #[test]
    fn a_lane_alone_has_no_bar_under_it() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let alone = Content::Readings(Readings { only: Some("cpu".into()), ..Readings::new(choice) });
        let all = Content::Readings(Readings::new(choice));
        for skin in [Skin::Paper, Skin::Glass, Skin::Fluent] {
            let theme = alone.dressed(Theme::new(skin, false));
            assert_eq!((theme.bar_gap, theme.bar_height), (0.0, 0.0), "{skin:?}");
            assert_eq!(all.dressed(Theme::new(skin, false)).bar_height, Theme::new(skin, false).bar_height, "{skin:?}");
        }
    }

    /// A note stuck on the desktop is its page alone, in every skin: nothing ruled off under it as chart paper rules a
    /// lane (a line ran under the page, past its rounded corners).
    #[test]
    fn a_stuck_note_has_no_rule_under_it() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let note = Content::Readings(Readings { workbench: true, only: Some(format!("{}a.md", view::STUCK_NOTE_LANE)), ..Readings::new(choice) });
        for skin in [Skin::Paper, Skin::Glass, Skin::Fluent] {
            assert!(!note.dressed(Theme::new(skin, false)).ruled, "{skin:?}");
        }
    }

    /// A note pulled off the panel is drawn as large as it is between its card and a note, in every skin: its page all
    /// of it at each size it passes through (laid out as the note it turns into, its page was that note's size from the
    /// first, its colour's band and its foot reaching past its card). Not turning, it is drawn as it was laid out.
    #[test]
    fn a_note_pulled_off_the_panel_is_laid_out_as_large_as_it_is() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let note = Content::Readings(Readings { workbench: true, only: Some(format!("{}a.md", view::STUCK_NOTE_LANE)), ..Readings::new(choice) });
        for skin in [Skin::Paper, Skin::Glass, Skin::Fluent] {
            let theme = note.dressed(Theme::new(skin, false));
            let layout = view::Layout::with_columns(vec![view::STUCK_NOTE.1], 1, &theme).fitted(view::STUCK_NOTE);
            let shown = Shown { form: choice.form, zoom: 1.0, size: view::STUCK_NOTE, layout, small: None, detail: view::Detail::Full };
            let page = |layout: &view::Layout| layout.lanes().iter().map(|lane| (lane.x, lane.y, lane.w, lane.h)).collect::<Vec<_>>();
            assert_eq!(page(&shown.drawn_at(view::STUCK_NOTE, false)), vec![(0.0, 0.0, view::STUCK_NOTE.0, view::STUCK_NOTE.1)], "{skin:?}");
            for size in [(148.0, 104.0), (204.0, 197.0), view::STUCK_NOTE, (372.0, 331.0)] {
                assert_eq!(page(&shown.drawn_at(size, true)), vec![(0.0, 0.0, size.0, size.1)], "{skin:?} {size:?}");
                assert_eq!(page(&shown.drawn_at(size, false)), page(&shown.layout), "{skin:?} {size:?}");
            }
        }
    }

    /// A workbench widget sized by hand shows each layout the hand makes at once (turning behind the hand, its cards
    /// were seen twice over, fading, lighter and darker); let go, or laid out anew otherwise, as its things flow anew
    /// it turns. A widget of readings turns into another form while sized too.
    #[test]
    fn a_workbench_sized_by_hand_shows_its_layout_at_once() {
        let theme = Theme::new(Skin::Paper, false);
        let shown = |width: f32, form: Form| {
            let layout = view::Layout::with_columns(vec![200.0, 300.0], 1, &theme).fitted((width, 560.0));
            Shown { form, zoom: 1.0, size: (layout.width(), layout.height()), layout, small: None, detail: view::Detail::Full }
        };
        let full = Form::Lanes { detail: view::Detail::Full, columns: 1 };
        let (wide, narrow) = (shown(356.0, full), shown(240.0, full));
        assert_ne!(wide.layout.flow(&theme), narrow.layout.flow(&theme), "the two sizes flow alike: no test");
        assert!(!turns_into(&wide, &narrow, true, true, &theme));
        assert!(turns_into(&wide, &narrow, true, false, &theme));
        let other = shown(240.0, Form::Lanes { detail: view::Detail::Shape(view::Shape::Down), columns: 1 });
        assert!(!turns_into(&wide, &other, true, true, &theme));
        assert!(turns_into(&wide, &other, false, true, &theme));
    }

    /// A widget of the workbench's (a note, the station) has no game button, a game shown changing nothing in it; one of
    /// the readings' has, and so has one merged with readings, whatever else is in it.
    #[test]
    fn only_the_readings_have_a_game_button() {
        let choice = Choice { form: Form::Lanes { detail: view::Detail::Full, columns: 1 }, scale: 1.0, extra: (0.0, 0.0) };
        let lanes = |workbench: bool| Content::Readings(Readings { workbench, ..Readings::new(choice.clone()) });
        let game = |content: &Content| content.buttons().contains(&Button::Game);
        assert!(!game(&lanes(true)));
        assert!(game(&lanes(false)));
        let merged = |parts: Vec<Content>| Content::Merged(Box::new(Merged::new(parts, true, music::LEAST)));
        assert!(!game(&merged(vec![Content::Music(MusicShown::new(music::LEAST)), lanes(true)])));
        assert!(game(&merged(vec![lanes(true), lanes(false)])));
    }

    /// One lane alone kept under a form of its own, read back as that lane and its form; a whole panel's form as it
    /// was; and the form an older Glance reads it as is none it knows (it leaves the widget out).
    #[test]
    fn keeps_one_lane_apart() {
        let form = Form::Lanes { detail: view::Detail::Full, columns: 1 }.key();
        let kept = form_of(Some("station"), form.clone());
        assert_eq!(only_of(&kept), (Some("station".to_string()), form.clone()));
        assert_eq!(only_of(&form_of(Some("note:周会: 议题.md"), form.clone())), (Some("note:周会: 议题.md".to_string()), form.clone()));
        assert_eq!(only_of(&form_of(None, form.clone())), (None, form));
        assert!(Form::from_key(&kept).is_none());
    }

    /// A widget stuck to an edge and slid along it stays where it is let go, stuck to that edge still (it went back to
    /// where it was first stuck); let go near another edge, it is stuck to that one.
    #[test]
    fn a_stuck_widget_slid_along_its_edge_stays_where_it_is_let_go() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1032 };
        let size = (400, 200);
        // Stuck to the left edge, its middle at 300 down it; slid down it to 500.
        let at = POINT { x: 0, y: 500 };
        let rect = RECT { left: at.x, top: at.y, right: at.x + size.0, bottom: at.y + size.1 };
        let (side, along) = stuck_after(None, Some((Side::Left, 300)), rect).unwrap();
        assert_eq!(side, Side::Left);
        assert_eq!(stuck_at(work, at, side, Some(along), None, size), at);
        // Slid up into the top left corner, nearer the top: stuck to the top, where it is.
        let at = POINT { x: 0, y: 0 };
        let rect = RECT { left: at.x, top: at.y, right: at.x + size.0, bottom: at.y + size.1 };
        let (side, along) = stuck_after(Some(Side::Top), Some((Side::Left, 300)), rect).unwrap();
        assert_eq!(side, Side::Top);
        assert_eq!(stuck_at(work, at, side, Some(along), None, size), at);
        // Stuck to none, let go near none: stuck to none.
        assert_eq!(stuck_after(None, None, rect), None);
    }

    /// A widget let go near two edges that meet is stuck in their corner, flush with both, however near each it was;
    /// near one alone, or the other only past where it sticks, to that edge alone.
    #[test]
    fn a_widget_let_go_near_a_corner_is_stuck_in_it() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1032 };
        let size = (400, 200);
        let rect_at = |x: i32, y: i32| RECT { left: x, top: y, right: x + size.0, bottom: y + size.1 };
        // 20 px from the right, 30 from the bottom, at 1.5: near both (a little over 13 and 20 DIPs).
        let rect = rect_at(1920 - 400 - 20, 1032 - 200 - 30);
        let (side, along) = stuck_after(Some(Side::Right), None, rect).unwrap();
        let also = corner_side(work, rect, 1.5, side, STICK);
        assert_eq!(also, Some(Side::Bottom));
        assert_eq!(stuck_at(work, POINT { x: rect.left, y: rect.top }, side, Some(along), also, size), POINT { x: 1520, y: 832 });
        // The top left corner, the top nearer.
        let rect = rect_at(30, 4);
        assert_eq!(corner_side(work, rect, 1.0, Side::Top, STICK), Some(Side::Left));
        assert_eq!(stuck_at(work, POINT { x: 30, y: 4 }, Side::Top, Some(230), Some(Side::Left), size), POINT { x: 0, y: 0 });
        // Near the right edge alone: 100 DIPs from the bottom, it is stuck to the right, where it is along it.
        let rect = rect_at(1520, 1032 - 200 - 100);
        assert_eq!(corner_side(work, rect, 1.0, Side::Right, STICK), None);
        // Nearer both the top and the bottom than it sticks (a screen as short as it is tall): the nearer.
        let short = RECT { left: 0, top: 0, right: 1920, bottom: 230 };
        assert_eq!(corner_side(short, rect_at(1520, 20), 1.0, Side::Right, STICK), Some(Side::Bottom));
    }

    /// A widget's window lets a click over its shadow through to what is under it, as the panel's does, but not one on
    /// it, on the band round its edge that sizes it or on its buttons, nor any while it is in hand; set to let clicks
    /// through, it lets all through but on the button that ends that.
    #[test]
    fn a_click_on_a_widgets_shadow_goes_through() {
        let window = RECT { left: 76, top: 76, right: 524, bottom: 324 };
        let taking = RECT { left: 92, top: 60, right: 508, bottom: 308 };
        let at = |x, y| Some(POINT { x, y });
        // Over its shadow, left of it and under it.
        assert!(clicks_through(false, false, at(80, 200), window, taking, false));
        assert!(clicks_through(false, false, at(300, 316), window, taking, false));
        // On it, on its edge's band, on its buttons' row (above it, past its window's top), and in hand.
        assert!(!clicks_through(false, false, at(300, 200), window, taking, false));
        assert!(!clicks_through(false, false, at(94, 200), window, taking, false));
        assert!(!clicks_through(false, false, at(300, 62), window, taking, false));
        assert!(!clicks_through(false, false, at(80, 200), window, taking, true));
        // Away from its window: none of its.
        assert!(!clicks_through(false, false, at(900, 900), window, taking, false));
        // Set to let clicks through: all, but on the button that ends that.
        assert!(clicks_through(true, false, at(300, 200), window, taking, false));
        assert!(!clicks_through(true, true, at(300, 200), window, taking, false));
    }

    #[test]
    fn a_corner_is_taken_along_either_edge() {
        let (w, h) = (300.0, 200.0);
        // On the right edge near the top, and on the top edge near the right.
        assert_eq!(edges_at(w - 2.0, 15.0, w, h), (false, true, true, false));
        assert_eq!(edges_at(w - 15.0, 2.0, w, h), (false, true, true, false));
        assert_eq!(edges_at(2.0, h - 15.0, w, h), (true, false, false, true));
        // Away from the corners, one edge; inside, none.
        assert_eq!(edges_at(w - 2.0, 100.0, w, h), (false, true, false, false));
        assert_eq!(edges_at(150.0, h - 2.0, w, h), (false, false, false, true));
        assert_eq!(edges_at(15.0, 15.0, w, h), (false, false, false, false));
        // Over its shadow, past the band round its edge: none.
        assert_eq!(edges_at(-EDGE - 1.0, 100.0, w, h), (false, false, false, false));
        assert_eq!(edges_at(w + EDGE + 1.0, h + EDGE + 1.0, w, h), (false, false, false, false));
        assert_eq!(edges_at(-EDGE + 1.0, 100.0, w, h), (true, false, false, false));
    }
}
