//! The disk organizer: a page of the settings window (see `settings_window`, `Page::Disk`) showing what a drive holds
//! as a map of blocks, each file its kind's colour, each folder what it holds, two levels at a time (see `disk::map`);
//! beside it the largest files, the files by type and by time; under them a place to gather what is to go, moved to
//! the Recycle Bin together. The drives are its tabs. Opened from the settings' pages, from a folder found in search,
//! or by its name in search (a drive's row in the panel opens the drive in Explorer). Everything it shows is read from
//! the drive's search index (see `worker`): no drive is scanned.

mod input;
mod paint;
mod worker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PostMessageW, WM_APP};

use crate::disk::map::{Block, Item, Rect};
use crate::disk::{Filter, Kind, Light, What};
use crate::ui::gfx::Frame;
use crate::ui::motion::Easing;
use crate::ui::text::Lang;
use worker::{Ask, Info, Shown, Told, Worker};

/// What the window the organizer is in is told: a reading come (see `worker`), an act done (see `Acted`). Taken there
/// by `took_shown` and `took_acts`.
pub const SHOWN: u32 = WM_APP + 52;
pub const ACTED: u32 = WM_APP + 53;

/// How long what is gathered waits, once it is to go, for its going to be undone (see `Pending`); how long the word
/// that it went stays; how long a word said stays (see `Said`).
const UNDO_FOR: Duration = Duration::from_secs(5);
const WENT_FOR: Duration = Duration::from_secs(9);
const SAID_FOR: Duration = Duration::from_millis(2600);
/// How long the map takes to move from one layout to the next: going in or out of a folder, the drive changing under
/// it, another drive.
const ZOOM: Duration = Duration::from_millis(380);
const FOLD: Duration = Duration::from_millis(420);
const FADE: Duration = Duration::from_millis(260);

/// The curves things move on, as the mockup's: CSS's own ease (a hover's fade), Windows 11's for what comes into
/// place, the one things slide and pop on, and what eases out alone.
const EASE: Easing = Easing(0.25, 0.1, 0.25, 1.0);
const SETTLE: Easing = Easing(0.1, 0.9, 0.2, 1.0);
const SLIDE: Easing = Easing(0.2, 0.8, 0.3, 1.0);
const EASE_OUT: Easing = Easing(0.0, 0.0, 0.58, 1.0);

/// A value going to another over `lasts` seconds on `curve`, from where it was when told where to go.
#[derive(Clone, Copy)]
struct Tween {
    from: f64,
    to: f64,
    since: Instant,
    lasts: f32,
    curve: Easing,
}

impl Tween {
    fn new(at: f64, lasts: f32, curve: Easing) -> Tween {
        Tween { from: at, to: at, since: Instant::now(), lasts, curve }
    }

    fn at(&self, now: Instant) -> f64 {
        let t = (now.saturating_duration_since(self.since).as_secs_f32() / self.lasts).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * self.curve.at(t) as f64
    }

    /// Going to `to` from where it is now (as it was going, if it was told so before).
    fn go(&mut self, to: f64, now: Instant) {
        if to != self.to {
            self.from = self.at(now);
            self.to = to;
            self.since = now;
        }
    }

    /// At `to` at once.
    fn set(&mut self, to: f64) {
        self.from = to;
        self.to = to;
    }

    fn moving(&self, now: Instant) -> bool {
        self.from != self.to && now < self.since + Duration::from_secs_f32(self.lasts)
    }
}

/// The bar under the tab shown, going to another chosen as the settings' own tabs' does: its end toward it first,
/// stretching it, its other end after, drawing it in again.
#[derive(Clone, Copy)]
struct Mark {
    left: Tween,
    right: Tween,
}

impl Mark {
    /// Under the middle `middle`.
    fn new(middle: f32) -> Mark {
        let (left, right) = Mark::ends(middle);
        Mark { left: Tween::new(left, 1.0, EASE), right: Tween::new(right, 1.0, EASE) }
    }

    fn ends(middle: f32) -> (f64, f64) {
        let half = (crate::ui::settings_window::TAB_MARK / 2.0) as f64;
        (middle as f64 - half, middle as f64 + half)
    }

    /// Going under the middle `middle` (there at once, with less motion asked for).
    fn go(&mut self, middle: f32, now: Instant) {
        let (left, right) = Mark::ends(middle);
        if crate::panel::reduced_motion() {
            self.left.set(left);
            self.right.set(right);
            return;
        }
        if left == self.left.to {
            return;
        }
        use crate::ui::settings_window::{TAB_MARK_LEADS as LEADS, TAB_MARK_TRAILS as TRAILS};
        let rightward = left > self.left.to;
        let (left_how, right_how) = if rightward { (TRAILS, LEADS) } else { (LEADS, TRAILS) };
        for (end, to, (lasts, curve)) in [(&mut self.left, left, left_how), (&mut self.right, right, right_how)] {
            end.from = end.at(now);
            end.to = to;
            end.since = now;
            end.lasts = lasts.as_secs_f32();
            end.curve = curve;
        }
    }

    /// Where its ends are now.
    fn at(&self, now: Instant) -> (f32, f32) {
        (self.left.at(now) as f32, self.right.at(now) as f32)
    }

    fn moving(&self, now: Instant) -> bool {
        self.left.moving(now) || self.right.moving(now)
    }
}

/// Something over the page as it comes and goes (a tag, a card, a menu, what is dragged): what it is, when it came,
/// when it went (drawn going till it has faded).
#[derive(Clone, Debug)]
struct Coming<T> {
    what: T,
    came: Instant,
    went: Option<Instant>,
}

impl<T> Coming<T> {
    fn new(what: T, now: Instant) -> Coming<T> {
        Coming { what, came: now, went: None }
    }

    /// How far it has come over `lasts` seconds (0 to 1, on `curve`), and gone again since it went.
    fn shown(&self, now: Instant, lasts: f32, curve: Easing) -> f32 {
        let part = |from: Instant, to: Instant| (to.saturating_duration_since(from).as_secs_f32() / lasts).clamp(0.0, 1.0);
        match self.went {
            None => curve.at(part(self.came, now)),
            Some(went) => {
                let had = part(self.came, went);
                curve.at((had - part(went, now)).max(0.0))
            }
        }
    }

    /// Whether it is done going (or coming) over `lasts` seconds.
    fn settled(&self, now: Instant, lasts: f32) -> bool {
        let since = self.went.unwrap_or(self.came);
        now.saturating_duration_since(since).as_secs_f32() >= lasts
    }
}

/// `coming` kept in step with what is to be shown (`now_is`): come anew where that is new (or another), gone where
/// there is none, let go of once gone; `same` telling whether two are the one shown still (else what it is now is
/// taken in as it stays). Whether it moves still over `lasts` seconds.
fn keep<T: Clone>(coming: &mut Option<Coming<T>>, now_is: Option<&T>, same: impl Fn(&T, &T) -> bool, now: Instant, lasts: f32) -> bool {
    match (coming.as_mut(), now_is) {
        (Some(shown), Some(is)) if shown.went.is_none() && same(&shown.what, is) => shown.what = is.clone(),
        (_, Some(is)) => *coming = Some(Coming::new(is.clone(), now)),
        (Some(shown), None) => {
            if shown.went.is_none() {
                shown.went = Some(now);
            } else if shown.settled(now, lasts) {
                *coming = None;
            }
        }
        (None, None) => {}
    }
    coming.as_ref().is_some_and(|shown| !shown.settled(now, lasts))
}

/// What is dragged, drawn by the pointer: its icon, its name and size, or the reason it may not go; where the pointer
/// is.
#[derive(Clone, Debug, PartialEq)]
struct Ghost {
    icon: crate::ui::icons::Icon,
    name: String,
    size: String,
    why_not: Option<String>,
    at: (f32, f32),
}

/// What the organizer was last asked to show, till the settings window takes it (see `take_asked`).
static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
/// What a shell thread did (see `act`), for the organizer to take in as its window is told (`ACTED`).
static ACTS: Mutex<Vec<Acted>> = Mutex::new(Vec::new());

/// What the organizer is asked to show: a drive (none: the one shown last, or the system's), a folder on it, and where
/// it was asked from (physical px: the settings pour out of there, see `genie`).
#[derive(Clone, Debug, Default)]
pub struct Asked {
    pub root: Option<String>,
    pub folder: Option<PathBuf>,
    pub place: Option<RECT>,
}

/// The organizer opened (or, open, turned) to the folder at `path`, on its drive.
pub fn open_folder(path: &Path, place: Option<RECT>) {
    let root = path.to_string_lossy().get(..3).filter(|root| root.ends_with(":\\")).map(|root| root.to_ascii_uppercase());
    open(Asked { root, folder: Some(path.to_path_buf()), place });
}

/// The organizer opened on the drive it showed last, else the system's.
pub fn open_last(place: Option<RECT>) {
    open(Asked { root: None, folder: None, place });
}

/// Whether the drive at `root` is one the organizer can show: one search knows every file of.
pub fn shows(root: &str) -> bool {
    crate::search::standing().iter().any(|standing| standing.root.eq_ignore_ascii_case(root))
}

/// The settings window opened (or raised) at the organizer's page, to show what `asked` says.
fn open(asked: Asked) {
    let place = asked.place;
    *ASKED.lock().unwrap() = Some(asked);
    crate::ui::settings_window::open_at_disk(place);
}

/// What the organizer was asked to show and has not yet: none, asked for by its page alone.
pub fn take_asked() -> Option<Asked> {
    ASKED.lock().unwrap().take()
}

/// How the organizer draws: Windows 11's colours, as the settings window takes them (see `settings_window::Palette`),
/// and the map's own (each kind's, after Explorer's icons and Fluent's).
#[derive(Clone)]
struct Palette {
    dark: bool,
    text: crate::ui::canvas::Color,
    text2: crate::ui::canvas::Color,
    text3: crate::ui::canvas::Color,
    hover: crate::ui::canvas::Color,
    control: crate::ui::canvas::Color,
    /// A hairline's (a share's track, the list's scroll thumb).
    rule: crate::ui::canvas::Color,
    card: crate::ui::canvas::Color,
    card_stroke: crate::ui::canvas::Color,
    /// A first-level folder's ground; a chosen row's; a meter's track.
    ground: crate::ui::canvas::Color,
    chosen: crate::ui::canvas::Color,
    meter: crate::ui::canvas::Color,
    /// A flyout's fill and its hairline.
    fly: crate::ui::canvas::Color,
    fly_stroke: crate::ui::canvas::Color,
    /// Windows 11's standard button: its fill, under the pointer, its hairline and its lower edge.
    button: crate::ui::canvas::Color,
    button_hover: crate::ui::canvas::Color,
    button_stroke: crate::ui::canvas::Color,
    button_edge: crate::ui::canvas::Color,
    accent: crate::ui::canvas::Color,
    on_accent: crate::ui::canvas::Color,
    /// What warns (a drive nearly full).
    signal: crate::ui::canvas::Color,
    /// A block's outline under the pointer; the ring inside a chosen block's (the card's colour).
    edge: crate::ui::canvas::Color,
    ring: crate::ui::canvas::Color,
    /// The stripes over what is gathered to go.
    stripe: crate::ui::canvas::Color,
    /// Each kind's colour, as its icons are drawn, and as a block of it is filled (mixed into the card's), and filled
    /// under the pointer.
    hue: [crate::ui::canvas::Color; 9],
    fill: [crate::ui::canvas::Color; 9],
    fill_hot: [crate::ui::canvas::Color; 9],
}

impl Palette {
    fn new(dark: bool) -> Palette {
        use crate::ui::canvas::Color;
        let white = |a| Color::hex(0xFFFFFF, a);
        let black = |a| Color::hex(0x000000, a);
        let hues: [u32; 9] = if dark {
            [0xB48CF5, 0x4BA9EE, 0xEE9384, 0x9DB3CC, 0xE3B84A, 0x3FB8B8, 0x6CC56C, 0xA0A0A0, 0xB4B4B4]
        } else {
            [0x8A4FE0, 0x1488D8, 0xD9705E, 0x6F8CAD, 0xC8961A, 0x0F8A8A, 0x2E8B2E, 0x7A7A7A, 0x9A9A9A]
        };
        let card = if dark { (43.0, 43.0, 43.0) } else { (255.0, 255.0, 255.0) };
        let mix = |hue: u32, share: f32| {
            let part = |shift: u32, of: f32| ((of + (((hue >> shift) & 0xFF) as f32 - of) * share).round() as u32).min(255);
            Color::hex(part(16, card.0) << 16 | part(8, card.1) << 8 | part(0, card.2), 1.0)
        };
        let (soft, strong) = if dark { (0.6, 0.72) } else { (0.44, 0.56) };
        let accent = crate::ui::theme::accent(dark);
        if dark {
            Palette {
                dark,
                text: white(1.0),
                text2: white(0.76),
                text3: white(0.5),
                hover: white(0.06),
                control: white(0.06),
                rule: white(0.08),
                card: white(0.05),
                card_stroke: black(0.2),
                ground: white(0.035),
                chosen: white(0.06),
                meter: white(0.14),
                fly: Color::hex(0x2C2C2C, 1.0),
                fly_stroke: white(0.09),
                button: white(0.06),
                button_hover: white(0.084),
                button_stroke: white(0.07),
                button_edge: white(0.07),
                accent,
                on_accent: black(1.0),
                signal: Color::hex(0xFF99A4, 1.0),
                edge: white(0.75),
                ring: Color::hex(0x1C1C1C, 1.0),
                stripe: white(0.28),
                hue: hues.map(|hue| Color::hex(hue, 1.0)),
                fill: hues.map(|hue| mix(hue, soft)),
                fill_hot: hues.map(|hue| mix(hue, strong)),
            }
        } else {
            Palette {
                dark,
                text: black(0.9),
                text2: black(0.6),
                text3: black(0.44),
                hover: black(0.04),
                control: black(0.05),
                rule: black(0.08),
                card: white(0.7),
                card_stroke: black(0.06),
                ground: black(0.035),
                chosen: black(0.05),
                meter: black(0.1),
                fly: Color::hex(0xF9F9F9, 1.0),
                fly_stroke: black(0.1),
                button: white(0.7),
                button_hover: Color::hex(0xF9F9F9, 0.5),
                button_stroke: black(0.0578),
                button_edge: black(0.1622),
                accent,
                on_accent: white(1.0),
                signal: Color::hex(0xC42B1C, 1.0),
                edge: black(0.5),
                ring: white(1.0),
                stripe: black(0.16),
                hue: hues.map(|hue| Color::hex(hue, 1.0)),
                fill: hues.map(|hue| mix(hue, soft)),
                fill_hot: hues.map(|hue| mix(hue, strong)),
            }
        }
    }
}

/// A block, or a piece of one, as it is known from one layout to the next: by its place in the index (a file's or a
/// folder's), the folder whose small things are together in it, or what is in no file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Place(u32),
    Many(u32),
    Unlisted,
}

/// A block of the map as laid out, with the reading it was laid out from (a block moving away is drawn from the one it
/// came from).
#[derive(Clone)]
struct Placed {
    key: Key,
    block: Block,
    shown: Rc<Shown>,
}

/// A block moving from one layout to the next (see `Anim`): from where, how seen, to where; how much a ground it is
/// from and to (a block becoming a folder's ground going in, or a ground a block going out, crosses from one look to
/// the other), and what it was before (drawn as its pieces as it crosses); going (not in the next layout).
#[derive(Clone)]
struct Moving {
    placed: Placed,
    from: Rect,
    to: Rect,
    from_alpha: f32,
    to_alpha: f32,
    from_ground: f32,
    to_ground: f32,
    was: Option<Placed>,
    gone: bool,
}

/// A block as it is seen now (moving, as far as it has): its layout, where it is, how clear, how much a ground it is
/// (0 to 1), and what it was before it moved.
#[derive(Clone)]
struct Seen {
    placed: Placed,
    at: Rect,
    alpha: f32,
    ground: f32,
    was: Option<Placed>,
}

struct Anim {
    moving: Vec<Moving>,
    began: Instant,
    lasts: Duration,
}

/// What the map's layout moves to from the last: the map going into a folder (the block at `Rect` growing to fill it),
/// out of one (into the block at `Rect`), each block from where it was or into its middle (the drive changed under it),
/// or fading (another drive).
#[derive(Clone, Copy, Debug)]
enum Change {
    In(Rect),
    Out(Rect),
    Fold,
    Fade,
}

/// The tabs of the list beside the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Largest,
    Types,
    Ages,
}

/// Something gathered to go.
#[derive(Clone, Debug, PartialEq)]
struct Gathered {
    path: PathBuf,
    name: String,
    bytes: u64,
    folder: bool,
    kind: Kind,
}

/// What was gathered, about to go to the Recycle Bin: its going undoable till `UNDO_FOR` has passed.
#[derive(Clone)]
struct Pending {
    items: Vec<Gathered>,
    since: Instant,
}

/// What the foot tells once what was gathered went: how many, how much; till when.
struct Went {
    count: usize,
    bytes: u64,
    until: Instant,
}

/// A word said over the foot for a while (see `SAID_FOR`), with its icon.
#[derive(Clone, Debug, PartialEq)]
struct Said {
    words: String,
    icon: crate::ui::icons::Icon,
    since: Instant,
}

/// What a card over the map tells of (see `Plate`).
#[derive(Clone, Debug, PartialEq)]
enum PlateOf {
    /// A system item: what it is (none: any of the system's), its name, its path, its size and files.
    System { info: Option<Info>, name: String, path: PathBuf, bytes: u64, files: u64 },
    /// Hibernation's turning off, as told of from its file's card.
    Hibernation { bytes: u64 },
    /// The Recycle Bin to be emptied: asked first.
    Empty,
}

/// A card over the window telling of a system item, its acts on it, at a point (above it, where `above`).
#[derive(Clone, Debug, PartialEq)]
struct Plate {
    of: PlateOf,
    at: (f32, f32),
    above: bool,
}

/// A menu over the window (see `paint::menu`): of what, under (or over) which box.
#[derive(Clone, Debug, PartialEq)]
enum MenuOf {
    /// The sizes the files shown may be filtered by.
    Sizes,
    /// The folders on the way to the one shown not shown in the path: each's name and path, the nearest first.
    Crumbs(Vec<(String, PathBuf)>),
    /// The drives whose tabs do not fit (by their place among the drives).
    Drives(Vec<usize>),
    /// What is gathered and not shown in the foot.
    Gathered(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq)]
struct Menu {
    of: MenuOf,
    under: Rect,
    above: bool,
}

/// A press held on the map or a row of the list, to be a click as it is let go, or dragged off.
#[derive(Clone, Debug)]
struct Press {
    at: (f32, f32),
    on: Pressed,
}

#[derive(Clone, Debug)]
enum Pressed {
    /// A block of the map (by its place in the layout).
    Block(usize),
    /// A file's row in the list (by its place among the rows).
    Row(usize),
    /// Any other spot (see `Target`): what it does, done as it is let go of over it.
    Spot(Target),
    /// The list's scroll thumb, held this far down from its top.
    Thumb(f32),
}

/// Which look of a spot fades (see `paint::fade`): under the pointer; on (pressed, lit, there to be used); its tools
/// shown (a row's, under the pointer).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Hot,
    On,
    Tools,
}

/// Something dragged to the foot: what it is, the reason it may not go (none: it may), where the pointer is.
struct Drag {
    what: Draggable,
    why_not: Option<String>,
    at: (f32, f32),
}

#[derive(Clone, Debug)]
enum Draggable {
    Block(usize),
    Row(usize),
}

/// What a press on the window's spots does (see `input`).
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// A drive's tab, the menu of those that do not fit, a drive in it.
    Drive(usize),
    DriveMore(Vec<usize>),
    MenuDrive(usize),
    Back,
    Crumb(PathBuf),
    CrumbMore(Vec<(String, PathBuf)>),
    FilterOld,
    FilterSize,
    FilterPreset,
    Map,
    /// The hover tools on the map's block under the pointer: gathered (or told of, a system item's), freed to the cloud.
    AddTool(usize),
    ReleaseTool(usize),
    Legend(Kind),
    Tab(Tab),
    AddAll,
    Row(usize),
    RowAdd(usize),
    RowRelease(usize),
    TypeRow(String),
    AgeRow(usize),
    /// A gathered thing's way out, the menu of those not shown, all of them out, their going, its undoing, the bin
    /// emptied.
    Ungather(usize),
    GatheredMore(Vec<usize>),
    UngatherAll,
    Commit,
    Undo,
    EmptyNow,
    Foot,
    MenuSize(u64),
    MenuCrumb(PathBuf),
    MenuUngather(usize),
    PlateAct(PlateAct),
    /// A card or menu, over what is under it.
    Over,
    /// The list's scroll thumb.
    Thumb,
}

/// What a card's buttons and links do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PlateAct {
    Open(crate::disk::act::Page),
    Hibernation,
    CopyCommand,
    AskEmpty,
    Empty,
    OpenBin,
    Close,
}

/// What a shell thread did, told to the window.
enum Acted {
    /// Gathered things moved to the Recycle Bin: those that went, those that did not.
    Recycled { went: Vec<Gathered>, kept: Vec<Gathered> },
    /// A OneDrive file or folder left to the cloud alone: its name, how much it frees, whether any was marked.
    Released { bytes: u64, marked: bool },
    /// A drive's Recycle Bin emptied: how much it freed, whether it was.
    Emptied { bytes: u64, done: bool },
}

/// The organizer's state, on the settings window's thread: kept while that window is open, whichever of its pages is
/// shown (what is gathered stays gathered, what waits to go goes as its time is up).
pub struct Organizer {
    /// The window it is drawn in, told what its threads read and did.
    hwnd: HWND,
    lang: Lang,
    palette: Palette,
    next_frame: Instant,
    last_frame: Instant,
    worker: Worker,
    serial: u64,
    /// The drives offered, the one shown, the folder shown (as asked: shown as soon as it is read).
    drives: Vec<crate::disk::act::Drive>,
    root: String,
    focus: PathBuf,
    /// What was read last, and its drive's state: its files being found still, or it gone.
    shown: Option<Rc<Shown>>,
    listing: bool,
    /// What was asked last (its serial aside), and what was read of the folder shown and of those above it as they
    /// were asked so: going up to one is shown from it at once, as it is read anew.
    asked: Option<Ask>,
    read_above: Vec<(Ask, Rc<Shown>)>,
    /// The map as laid out (its size), moving from the last, and what each block and piece is dimmed to (eased).
    laid: Vec<Placed>,
    laid_size: (f32, f32),
    anim: Option<Anim>,
    change: Option<Change>,
    dims: HashMap<Key, f32>,
    /// The block under the pointer (its place in the layout); the file chosen (by its path).
    hover: Option<usize>,
    chosen: Option<PathBuf>,
    filter: Filter,
    light: Option<Light>,
    peek: Option<Light>,
    tab: Tab,
    /// Where the mark under the list's tab shown is (its left, sliding to it), and the drive whose tab is shown and
    /// since when (its mark growing in under it).
    tab_mark: Option<Mark>,
    drive_mark: Option<Mark>,
    scroll: f32,
    scroll_to: f32,
    gathered: Vec<Gathered>,
    pending: Option<Pending>,
    /// What is being moved to the Recycle Bin now (its undoing past), till the shell's thread tells how it went.
    going: Vec<Gathered>,
    went: Option<Went>,
    said: Option<Said>,
    plate: Option<Plate>,
    menu: Option<Menu>,
    press: Option<Press>,
    /// What the last click on the map was on (a file's or folder's path): what a double-click shows in Explorer.
    clicked: Option<PathBuf>,
    drag: Option<Drag>,
    pointer: Option<(f32, f32)>,
    targets: Vec<(Rect, Target)>,
    /// The drive's use as its words tell it (what is used, counted to what it is) and as its meter shows it (what is
    /// used but for the Recycle Bin, what is gathered at its end, the Recycle Bin's after it), for which drive.
    used_told: Tween,
    meter: [Tween; 3],
    meter_of: String,
    /// The time this frame is drawn at; each spot's look under the pointer (or pressed, or shown) as it fades to it,
    /// and whether any fades still.
    now: Instant,
    fades: Vec<(Target, Look, Tween, bool)>,
    fading: bool,
    /// What is over the page as it comes and goes: the tag of the block under the pointer (its place in the layout,
    /// where the pointer is), the tools on it, a card, a menu, what is dragged.
    tag_shown: Option<Coming<(usize, (f32, f32))>>,
    drive_tag: Option<Coming<usize>>,
    tools_shown: Option<Coming<paint::BlockTools>>,
    plate_shown: Option<Coming<Plate>>,
    menu_shown: Option<Coming<Menu>>,
    ghost: Option<Coming<Ghost>>,
    said_shown: Option<Coming<Said>>,
    /// What the foot shows (told by a sum of it), and since when: it comes up anew as that changes. And whether the
    /// menu of what is gathered and not shown is to open again once the foot is drawn anew (one taken out of it).
    foot_seen: (u64, Instant),
    reopen_gathered: bool,
    /// Where the map was last drawn (DIPs in the window), and the foot.
    map_at: Rect,
    foot_at: Rect,
    /// This PC's OneDrive folders: what is in them may be left in the cloud alone.
    clouds: Vec<PathBuf>,
}

impl Drop for Organizer {
    /// Its window closed: what waits to go goes now.
    fn drop(&mut self) {
        self.commit_now();
    }
}

/// The date a file time (100 ns since 1601) falls on by this PC's clock: its year, month and day.
fn local_date(written: i64) -> (u16, u16, u16) {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let file = FILETIME { dwLowDateTime: written as u32, dwHighDateTime: (written >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    let read = unsafe { FileTimeToSystemTime(&file, &mut utc).ok().and_then(|_| SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()) };
    match read {
        Some(()) => (local.wYear, local.wMonth, local.wDay),
        None => (utc.wYear, utc.wMonth, utc.wDay),
    }
}

impl Organizer {
    /// The organizer, drawn in the window `hwnd` (told there what its threads read and did: `SHOWN`, `ACTED`), light or
    /// `dark`, in `lang`. Nothing shown till it is turned to a drive (see `turn`).
    pub fn new(hwnd: HWND, dark: bool, lang: Lang) -> Organizer {
        Organizer {
            hwnd,
            lang,
            palette: Palette::new(dark),
            next_frame: Instant::now(),
            last_frame: Instant::now(),
            worker: Worker::start(hwnd, SHOWN),
            serial: 0,
            drives: crate::disk::act::drives(),
            root: String::new(),
            focus: PathBuf::new(),
            shown: None,
            listing: false,
            asked: None,
            read_above: Vec::new(),
            laid: Vec::new(),
            laid_size: (0.0, 0.0),
            anim: None,
            change: None,
            dims: HashMap::new(),
            hover: None,
            chosen: None,
            filter: Filter::default(),
            light: None,
            peek: None,
            tab: Tab::Largest,
            tab_mark: None,
            drive_mark: None,
            scroll: 0.0,
            scroll_to: 0.0,
            gathered: Vec::new(),
            pending: None,
            going: Vec::new(),
            went: None,
            said: None,
            plate: None,
            menu: None,
            press: None,
            clicked: None,
            drag: None,
            pointer: None,
            targets: Vec::new(),
            used_told: Tween::new(0.0, 0.6, SETTLE),
            meter: [Tween::new(0.0, 0.5, SLIDE); 3],
            meter_of: String::new(),
            now: Instant::now(),
            fades: Vec::new(),
            fading: false,
            tag_shown: None,
            drive_tag: None,
            tools_shown: None,
            plate_shown: None,
            menu_shown: None,
            ghost: None,
            said_shown: None,
            foot_seen: (0, Instant::now()),
            reopen_gathered: false,
            map_at: Rect::default(),
            foot_at: Rect::default(),
            clouds: crate::disk::Places::of_this_pc().onedrive,
        }
    }

    /// Light or `dark`, in `lang`, as its window is now.
    pub fn restyle(&mut self, dark: bool, lang: Lang) {
        if self.palette.dark != dark {
            self.palette = Palette::new(dark);
        }
        self.lang = lang;
        self.next_frame = Instant::now();
    }

    /// When it is to be drawn next: at once while something moves or the pointer did, else as a timer is due.
    pub fn next_frame(&self) -> Instant {
        self.next_frame
    }

    /// Turned to what `asked` says: a drive (its root, a folder on it), as it is now; with nothing asked, to the drive
    /// shown last (as it was left, if it is shown still).
    pub fn turn(&mut self, asked: &Asked) {
        self.drives = crate::disk::act::drives();
        let last = crate::app().settings.lock().unwrap().disk_drive.clone();
        let system = std::env::var("SystemDrive").map(|drive| format!("{}\\", drive.to_ascii_uppercase())).unwrap_or_else(|_| "C:\\".into());
        let root = asked
            .root
            .clone()
            .or((!self.root.is_empty()).then(|| self.root.clone()))
            .or(last)
            .filter(|root| self.drives.iter().any(|drive| drive.root.eq_ignore_ascii_case(root)))
            .or_else(|| self.drives.iter().find(|drive| drive.root.eq_ignore_ascii_case(&system)).map(|drive| drive.root.clone()))
            .or_else(|| self.drives.first().map(|drive| drive.root.clone()));
        let Some(root) = root else {
            self.root.clear();
            self.next_frame = Instant::now();
            return;
        };
        if !root.eq_ignore_ascii_case(&self.root) {
            self.to_drive(&root);
        }
        match (asked.folder.clone(), &asked.root) {
            (Some(folder), _) => self.go_to(folder),
            // (Turned to with no drive asked: as it was left, what it showed read anew.)
            (None, None) if self.shown.is_some() => self.ask(),
            (None, _) => self.go_to(PathBuf::from(&root)),
        }
    }

    /// The drive at `root` shown, from its root: what was gathered on the one before goes (what waits to go, now).
    fn to_drive(&mut self, root: &str) {
        self.commit_now();
        self.gathered.clear();
        self.went = None;
        self.root = root.to_string();
        self.focus = PathBuf::from(root);
        self.chosen = None;
        self.light = None;
        self.peek = None;
        self.plate = None;
        self.menu = None;
        self.change = Some(Change::Fade);
        self.listing = false;
        let root = root.to_string();
        crate::app().change(|settings| settings.disk_drive = Some(root));
        self.ask();
    }

    /// The folder at `path` shown (as soon as it is read).
    fn go_to(&mut self, path: PathBuf) {
        self.plate = None;
        self.menu = None;
        if path == self.focus && self.shown.as_ref().is_some_and(|shown| shown.focus == path) {
            return;
        }
        // In or out: from the block it is drawn as, or into the one the folder shown was.
        if self.change.is_none() {
            let map = Rect { x: 0.0, y: 0.0, w: self.laid_size.0, h: self.laid_size.1 };
            let drawn = |target: &Path| self.drawn_rect(target);
            self.change = Some(if path.starts_with(&self.focus) && path != self.focus {
                drawn(&path).map_or(Change::Fade, Change::In)
            } else if self.focus.starts_with(&path) {
                Change::Out(Rect { w: map.w.max(1.0), h: map.h.max(1.0), ..map })
            } else {
                Change::Fade
            });
        }
        self.focus = path;
        self.scroll = 0.0;
        self.scroll_to = 0.0;
        // A folder above, read before as it is asked now: shown from that at once (moving as it was to), the reading
        // anew shown as it comes.
        let read = self.read_above.iter().find(|(asked, _)| *asked == self.asking(0)).map(|(_, shown)| shown.clone());
        if let Some(read) = read {
            self.shown = Some(read);
            let change = self.change.take();
            self.relay(change);
        }
        self.ask();
    }

    /// The folder above the one shown, shown.
    pub fn up_a_folder(&mut self) {
        if let Some(parent) = self.focus.parent().filter(|_| !self.at_root()) {
            self.go_to(parent.to_path_buf());
        }
    }

    fn at_root(&self) -> bool {
        self.focus.as_os_str().len() <= self.root.len()
    }

    /// Esc: a menu or a card closed, else the folder above shown; at the drive's root, whether the window is to close.
    pub fn escape(&mut self) -> bool {
        self.next_frame = Instant::now();
        if self.menu.take().is_some() || self.plate.take().is_some() {
            return false;
        }
        if self.at_root() {
            return true;
        }
        self.up_a_folder();
        false
    }

    /// The reading asked for what is to be shown now.
    fn ask(&mut self) {
        if self.root.is_empty() {
            return;
        }
        self.serial += 1;
        let ask = self.asking(self.serial);
        self.asked = Some(Ask { serial: 0, ..ask.clone() });
        self.worker.ask(ask);
        self.next_frame = Instant::now();
    }

    /// What is to be asked now, as `serial`.
    fn asking(&self, serial: u64) -> Ask {
        // (As large as the map can be: the tree is copied out as far down as that could draw.)
        let (width, height) = self.client();
        let card = paint::geo(width, height).card;
        let area = ((card.w - 24.0) * (card.h - 60.0)).max(10_000.0);
        Ask { root: self.root.clone(), focus: self.focus.clone(), filter: self.filter, light: self.light.clone(), area, serial }
    }

    /// Its window's size (DIPs).
    fn client(&self) -> (f32, f32) {
        let mut rect = RECT::default();
        let scale = unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
            GetDpiForWindow(self.hwnd)
        } as f32
            / 96.0;
        let scale = if scale > 0.0 { scale } else { 1.0 };
        ((rect.right - rect.left) as f32 / scale, (rect.bottom - rect.top) as f32 / scale)
    }

    /// What the reading thread read, taken in: the map laid out anew from it, moving from the last.
    pub fn take_shown(&mut self) {
        let Some(told) = self.worker.take() else { return };
        match told {
            // (Of what was asked before: what is asked now comes after.)
            Told::Listing(serial) | Told::Nowhere(serial) if serial != self.serial => {}
            Told::Listing(_) => {
                self.listing = true;
            }
            Told::Nowhere(_) => {
                self.listing = false;
                self.drives = crate::disk::act::drives();
            }
            Told::Shown(shown) => {
                self.listing = false;
                // (A reading of what was asked before: what is asked now comes after.)
                if shown.serial != self.serial && !shown.changed {
                    return;
                }
                let replaced = self.shown.as_ref().is_some_and(|before| before.replaced != shown.replaced || before.root != shown.root);
                let change = match self.change.take() {
                    Some(change) => change,
                    None => Change::Fold,
                };
                let change = if replaced && !matches!(change, Change::Fade) { Change::Fold } else { change };
                if shown.focus != self.focus && shown.serial == self.serial {
                    self.focus = shown.focus.clone();
                }
                // (What went is no longer gathered.)
                self.gathered.retain(|gathered| gathered.path.exists());
                if let Some(drive) = self.drives.iter_mut().find(|drive| drive.root.eq_ignore_ascii_case(&shown.root)) {
                    drive.total = shown.total;
                    drive.free = shown.free;
                }
                let shown = Rc::new(*shown);
                // Kept as read for what was asked, with what was read of the folders above it asked as it was.
                if let Some(asked) = self.asked.clone().filter(|_| shown.serial == self.serial) {
                    self.read_above.retain(|(above, _)| above.focus != asked.focus && asked.focus.starts_with(&above.focus) && Ask { focus: asked.focus.clone(), ..above.clone() } == asked);
                    self.read_above.push((asked, shown.clone()));
                }
                self.shown = Some(shown);
                self.relay(Some(change));
            }
        }
        self.next_frame = Instant::now();
    }

    /// The map laid out anew for the reading shown, at the map's size, moving from the last as `change` says (none:
    /// at once).
    fn relay(&mut self, change: Option<Change>) {
        let Some(shown) = self.shown.clone() else { return };
        let (width, height) = self.laid_size;
        if width < 1.0 || height < 1.0 {
            return;
        }
        let blocks = crate::disk::map::lay_out(&shown.tree, width, height);
        let next: Vec<Placed> = blocks.into_iter().map(|block| Placed { key: key_of(&shown, &block.item), block, shown: shown.clone() }).collect();
        // Laid out as it was (the drive changed, not what the map shows of it): nothing starts moving, what moves keeps
        // on as it was, the tools and the tag kept.
        let same = next.len() == self.laid.len() && next.iter().zip(&self.laid).all(|(next, was)| next.key == was.key && same_rect(next.block.at, was.block.at));
        if same {
            self.laid = next;
            return;
        }
        // Going out: into where the folder shown before is drawn now, else the drive changed under it.
        let change = match change {
            Some(Change::Out(_)) => {
                let before = self.laid.first().map(|placed| Key::Place(placed.shown.tree.nodes[0].place));
                Some(before.and_then(|key| drawn_as(&next, key)).map_or(Change::Fold, Change::Out))
            }
            change => change,
        };
        match change {
            Some(change) if !self.laid.is_empty() && !crate::panel::reduced_motion() => {
                let now = self.now_rects(Instant::now());
                self.anim = Some(moving(now, &next, change, width, height));
            }
            _ => self.anim = None,
        }
        self.laid = next;
        // The block under the pointer, as laid out now (its tag and tools gone: they come again once the map is still).
        self.hover = self.pointer.and_then(|(x, y)| self.block_at(x, y));
        self.tag_shown = None;
        self.tools_shown = None;
    }

    /// The block of the map at (`x`, `y`) in the window, the deepest there: its place in the layout.
    fn block_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.map_at.contains(x, y) {
            return None;
        }
        let (x, y) = (x - self.map_at.x, y - self.map_at.y);
        let mut found: Option<usize> = None;
        for (at, placed) in self.laid.iter().enumerate() {
            if placed.block.at.contains(x, y) && found.is_none_or(|found| self.laid[found].block.level < placed.block.level) {
                found = Some(at);
            }
        }
        found
    }

    /// The blocks as they are seen now (moving, as far as they have).
    fn now_rects(&self, now: Instant) -> Vec<Seen> {
        match &self.anim {
            Some(anim) => {
                let t = anim_t(anim, now);
                anim.moving
                    .iter()
                    .map(|moving| Seen {
                        placed: moving.placed.clone(),
                        at: lerp_rect(moving.from, moving.to, t),
                        alpha: moving.from_alpha + (moving.to_alpha - moving.from_alpha) * t,
                        ground: moving.from_ground + (moving.to_ground - moving.from_ground) * t,
                        was: moving.was.clone(),
                    })
                    .collect()
            }
            None => self.laid.iter().map(|placed| Seen { placed: placed.clone(), at: placed.block.at, alpha: 1.0, ground: if placed.block.group { 1.0 } else { 0.0 }, was: None }).collect(),
        }
    }

    /// Where the file at `path` itself is drawn on the map now: its block, or the piece of a block it is (told so).
    fn exact_rect(&self, path: &Path) -> Option<(Rect, bool)> {
        for placed in &self.laid {
            let shown = &placed.shown;
            let is = |item: &Item| matches!(item, Item::Node(node) if matches!(shown.tree.nodes[*node].what, What::File { .. }) && shown.paths[*node] == path);
            if is(&placed.block.item) {
                return Some((placed.block.at, false));
            }
            let at = placed.block.at;
            if let Some(piece) = placed.block.pieces.iter().find(|piece| is(&piece.item)) {
                return Some((Rect { x: at.x + piece.at.x * at.w, y: at.y + piece.at.y * at.h, w: piece.at.w * at.w, h: piece.at.h * at.h }, true));
            }
        }
        None
    }

    /// Where the file or folder at `path` is drawn on the map now: its block, or the piece of a block it is, or the
    /// block it is in.
    fn drawn_rect(&self, path: &Path) -> Option<Rect> {
        let mut found: Option<(usize, Rect)> = None;
        for placed in &self.laid {
            let shown = &placed.shown;
            let mut look = |item: &Item, at: Rect| {
                let node = match item {
                    Item::Node(node) => *node,
                    Item::Many { folder, .. } => *folder,
                };
                let of = &shown.paths[node];
                if path.starts_with(of) {
                    let depth = of.components().count() + usize::from(matches!(item, Item::Node(_)));
                    if found.is_none_or(|(deepest, _)| depth > deepest) {
                        found = Some((depth, at));
                    }
                }
            };
            look(&placed.block.item, placed.block.at);
            for piece in &placed.block.pieces {
                let at = placed.block.at;
                look(&piece.item, Rect { x: at.x + piece.at.x * at.w, y: at.y + piece.at.y * at.h, w: piece.at.w * at.w, h: piece.at.h * at.h });
            }
        }
        found.map(|(_, at)| at).filter(|at| at.w > 0.0 && at.h > 0.0)
    }

    /// Timers: what waits to go, going once its time to be undone is past; what was said, and that it went, unsaid.
    pub fn timers(&mut self, now: Instant) {
        if self.pending.as_ref().is_some_and(|pending| now >= pending.since + UNDO_FOR) {
            self.commit_now();
        }
        if self.said.as_ref().is_some_and(|said| now >= said.since + SAID_FOR) {
            self.said = None;
            self.next_frame = now;
        }
        if self.went.as_ref().is_some_and(|went| now >= went.until) {
            self.went = None;
            self.next_frame = now;
        }
    }

    fn say(&mut self, words: String, icon: crate::ui::icons::Icon) {
        self.said = Some(Said { words, icon, since: Instant::now() });
        self.next_frame = Instant::now();
    }

    /// What was gathered set to go, undoable for a while.
    fn start_commit(&mut self) {
        if self.gathered.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.gathered);
        self.pending = Some(Pending { items, since: Instant::now() });
        self.went = None;
        self.next_frame = Instant::now();
    }

    fn undo(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.gathered = pending.items;
        }
        self.next_frame = Instant::now();
    }

    /// What waits to go moved to the Recycle Bin now, on a thread of the shell's.
    fn commit_now(&mut self) {
        let Some(pending) = self.pending.take() else { return };
        self.going.extend(pending.items.iter().cloned());
        let hwnd = self.hwnd;
        act(hwnd, move || {
            let (went, kept): (Vec<Gathered>, Vec<Gathered>) = pending.items.into_iter().partition(|item| crate::disk::act::recycle(&item.path));
            Acted::Recycled { went, kept }
        });
        self.next_frame = Instant::now();
    }

    /// What a shell thread did, taken in.
    fn acted(&mut self, acted: Acted) {
        let lang = self.lang;
        match acted {
            Acted::Recycled { went, kept } => {
                self.going.retain(|item| !went.contains(item) && !kept.contains(item));
                if !went.is_empty() {
                    self.went = Some(Went { count: went.len(), bytes: went.iter().map(|item| item.bytes).sum(), until: Instant::now() + WENT_FOR });
                }
                if !kept.is_empty() {
                    let count = kept.len();
                    for item in kept {
                        if !self.gathered.contains(&item) {
                            self.gathered.push(item);
                        }
                    }
                    self.say(format!("{} {}", count_text(count as u64), lang.pick("项未能移至回收站，可能没有权限或正在使用", "items could not be moved to the Recycle Bin: no permission, or in use")), crate::ui::icons::Icon::Info);
                }
                // (What is shown at once as it is now: the drive's change comes in a moment.)
                if let Some(chosen) = &self.chosen {
                    if !chosen.exists() {
                        self.chosen = None;
                    }
                }
            }
            Acted::Released { bytes, marked } => {
                if marked {
                    self.say(format!("{} {}{}", lang.pick("将释放", "Freeing"), size_text(bytes, lang), lang.pick("，文件只保留在 OneDrive 中", ", the files kept in OneDrive alone")), crate::ui::icons::Icon::Cloud);
                } else {
                    self.say(lang.pick("未能释放空间", "Space could not be freed").into(), crate::ui::icons::Icon::Info);
                }
            }
            Acted::Emptied { bytes, done } => {
                if done {
                    self.went = None;
                    self.say(format!("{}{}", lang.pick("已清空回收站，释放 ", "Recycle Bin emptied, freeing "), size_text(bytes, lang)), crate::ui::icons::Icon::CircleCheck);
                } else {
                    self.say(lang.pick("未能清空回收站", "The Recycle Bin could not be emptied").into(), crate::ui::icons::Icon::Info);
                }
            }
        }
        self.drives = crate::disk::act::drives();
        self.next_frame = Instant::now();
    }

    /// Drawn on `frame` (the window's, from its corner, the settings' pages' list left of it) as it is now: whether
    /// anything moves still. When it is to be drawn next is kept (see `next_frame`).
    pub fn draw(&mut self, frame: &Frame, now: Instant) -> bool {
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.064);
        self.last_frame = now;
        let moving = self.paint(frame, now, dt);
        // Moving: the next frame at once (the screen's next); else when a timer is due, or a while.
        let due = [self.pending.as_ref().map(|pending| pending.since + UNDO_FOR), self.said.as_ref().map(|said| said.since + SAID_FOR), self.went.as_ref().map(|went| went.until)].into_iter().flatten().min();
        self.next_frame = if moving || self.pending.is_some() { now + Duration::from_millis(8) } else { due.unwrap_or(now + Duration::from_secs(3600)).max(now + Duration::from_millis(8)) };
        moving
    }

    /// What shell threads did, taken in, as its window is told (`ACTED`).
    pub fn took_acts(&mut self) {
        let acts = std::mem::take(&mut *ACTS.lock().unwrap());
        for acted in acts {
            self.acted(acted);
        }
    }

    /// What was pressed or dragged let go of where it is: the pointer taken by another window.
    pub fn pointer_lost(&mut self) {
        self.press = None;
        self.drag = None;
        self.next_frame = Instant::now();
    }
}

/// `act` run on a shell thread, the window told what it did.
fn act(hwnd: HWND, act: impl FnOnce() -> Acted + Send + 'static) {
    let window = hwnd.0 as isize;
    crate::disk::act::by_the_shell(move || {
        let acted = act();
        ACTS.lock().unwrap().push(acted);
        let _ = unsafe { PostMessageW(Some(HWND(window as *mut _)), ACTED, WPARAM(0), LPARAM(0)) };
    });
}

/// How a block's item is known from one layout to the next.
fn key_of(shown: &Shown, item: &Item) -> Key {
    match item {
        Item::Node(node) if shown.tree.nodes[*node].what == What::Unlisted => Key::Unlisted,
        Item::Node(node) => Key::Place(shown.tree.nodes[*node].place),
        Item::Many { folder, .. } => Key::Many(shown.tree.nodes[*folder].place),
    }
}

/// Where the block or piece known as `key` is in `blocks`.
fn drawn_as(blocks: &[Placed], key: Key) -> Option<Rect> {
    for placed in blocks {
        if placed.key == key {
            return Some(placed.block.at);
        }
        let at = placed.block.at;
        for piece in &placed.block.pieces {
            if key_of(&placed.shown, &piece.item) == key {
                return Some(Rect { x: at.x + piece.at.x * at.w, y: at.y + piece.at.y * at.h, w: piece.at.w * at.w, h: piece.at.h * at.h });
            }
        }
    }
    None
}

/// How far `anim` has gone at `now`, eased (Windows 11's curve for what moves into place).
fn anim_t(anim: &Anim, now: Instant) -> f32 {
    let t = (now.duration_since(anim.began).as_secs_f32() / anim.lasts.as_secs_f32()).clamp(0.0, 1.0);
    crate::ui::motion::Easing(0.1, 0.9, 0.2, 1.0).at(t)
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5 && (a.w - b.w).abs() < 0.5 && (a.h - b.h).abs() < 0.5
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, w: a.w + (b.w - a.w) * t, h: a.h + (b.h - a.h) * t }
}

/// `r` in the box `from` taken to where it is in `to` (`from` stretched over `to`).
fn mapped(from: Rect, to: Rect, r: Rect) -> Rect {
    Rect { x: to.x + (r.x - from.x) * to.w / from.w, y: to.y + (r.y - from.y) * to.h / from.h, w: r.w * to.w / from.w, h: r.h * to.h / from.h }
}

fn centre(r: Rect) -> Rect {
    Rect { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0, w: 0.0, h: 0.0 }
}

/// The blocks moving from how they are seen now (`now`) to the layout `next` (a map `width` × `height`), as `change`
/// says.
fn moving(now: Vec<Seen>, next: &[Placed], change: Change, width: f32, height: f32) -> Anim {
    let map = Rect { x: 0.0, y: 0.0, w: width, h: height };
    let (enter, exit): (Box<dyn Fn(Rect) -> (Rect, f32)>, Box<dyn Fn(Rect) -> (Rect, f32)>) = match change {
        Change::In(at) => (Box::new(move |r| (mapped(map, at, r), 0.0)), Box::new(move |r| (mapped(at, map, r), 0.0))),
        // (Into the block the folder shown before is drawn as now: what comes grows out of it as what goes shrinks in.)
        Change::Out(at) => (Box::new(move |r| (mapped(at, map, r), 0.0)), Box::new(move |r| (mapped(map, at, r), 0.0))),
        Change::Fold => (Box::new(|r| (centre(r), 0.0)), Box::new(|r| (centre(r), 0.0))),
        Change::Fade => (Box::new(|r| (r, 0.0)), Box::new(|r| (r, 0.0))),
    };
    let lasts = match change {
        Change::In(_) | Change::Out(_) => ZOOM,
        Change::Fold => FOLD,
        Change::Fade => FADE,
    };
    let fading = matches!(change, Change::Fade);
    let ground_of = |placed: &Placed| if placed.block.group { 1.0 } else { 0.0 };
    let mut moves = Vec::new();
    for placed in next {
        let from = now.iter().find(|seen| !fading && seen.placed.key == placed.key && seen.alpha > 0.01);
        match from {
            Some(seen) => moves.push(Moving {
                placed: placed.clone(),
                from: seen.at,
                to: placed.block.at,
                from_alpha: seen.alpha,
                to_alpha: 1.0,
                from_ground: seen.ground,
                to_ground: ground_of(placed),
                was: Some(seen.placed.clone()),
                gone: false,
            }),
            None => {
                let (at, alpha) = enter(placed.block.at);
                moves.push(Moving { placed: placed.clone(), from: at, to: placed.block.at, from_alpha: alpha, to_alpha: 1.0, from_ground: ground_of(placed), to_ground: ground_of(placed), was: None, gone: false });
            }
        }
    }
    for seen in now {
        if seen.alpha > 0.01 && (fading || !next.iter().any(|placed| placed.key == seen.placed.key)) {
            let (to, to_alpha) = exit(seen.at);
            moves.push(Moving { placed: seen.placed, from: seen.at, to, from_alpha: seen.alpha, to_alpha, from_ground: seen.ground, to_ground: seen.ground, was: seen.was, gone: true });
        }
    }
    // (What goes drawn under what comes, as deep as each is.)
    moves.sort_by_key(|moving| (moving.placed.block.level, !moving.gone));
    Anim { moving: moves, began: Instant::now(), lasts }
}

/// A size as the organizer writes it: bytes, KB, MB, GB, a tenth where it is under a hundred.
fn size_text(bytes: u64, lang: Lang) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    // (The figure and its unit joined by a space no line breaks at.)
    if b < KB {
        return format!("{bytes}{NBSP}{}", lang.pick("字节", "bytes"));
    }
    if b < MB {
        return format!("{:.0}{NBSP}KB", b / KB);
    }
    let (value, unit) = if b < GB { (b / MB, "MB") } else { (b / GB, "GB") };
    if value >= 100.0 { format!("{value:.0}{NBSP}{unit}") } else { format!("{value:.1}{NBSP}{unit}") }
}

/// A space a line is not broken at.
const NBSP: char = '\u{a0}';

/// A count with its thousands set apart (18,400).
fn count_text(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A file time's date, as the organizer writes it.
fn date_text(written: i64, lang: Lang) -> String {
    if written <= 0 {
        return "—".into();
    }
    let (year, month, day) = local_date(written);
    match lang {
        Lang::Zh => format!("{year}年{month}月{day}日"),
        Lang::En => {
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {day}, {year}", MONTHS[(month.clamp(1, 12) - 1) as usize])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_counts_are_written_as_the_mockup_writes_them() {
        assert_eq!(size_text(0, Lang::Zh), "0\u{a0}字节");
        assert_eq!(size_text(512, Lang::Zh), "512\u{a0}字节");
        assert_eq!(size_text(20 * 1024, Lang::En), "20\u{a0}KB");
        assert_eq!(size_text(5 * 1024 * 1024 + 300 * 1024, Lang::En), "5.3\u{a0}MB");
        assert_eq!(size_text(250 * 1024 * 1024, Lang::En), "250\u{a0}MB");
        assert_eq!(size_text(25 * (1 << 30) + (1 << 29), Lang::En), "25.5\u{a0}GB");
        assert_eq!(size_text(150 * (1 << 30), Lang::En), "150\u{a0}GB");
        assert_eq!(count_text(7), "7");
        assert_eq!(count_text(18_400), "18,400");
        assert_eq!(count_text(1_234_567), "1,234,567");
    }

    #[test]
    fn a_block_is_mapped_from_one_box_into_another() {
        let from = Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let to = Rect { x: 10.0, y: 20.0, w: 50.0, h: 100.0 };
        assert_eq!(mapped(from, to, from), to);
        assert_eq!(mapped(from, to, Rect { x: 50.0, y: 25.0, w: 10.0, h: 5.0 }), Rect { x: 35.0, y: 70.0, w: 5.0, h: 10.0 });
    }

    #[test]
    fn what_is_under_a_folder_is_told_by_whole_names() {
        let under = |path: &str, top: &str| paint::under(Path::new(path), Path::new(top));
        assert!(under(r"D:\Research\why-drift\experiments\a.bin", r"D:\Research\why-drift\experiments"));
        assert!(under(r"D:\Research\why-drift\experiments", r"D:\Research\why-drift\experiments"));
        assert!(under(r"D:\Research", r"D:\"));
        assert!(!under(r"D:\Research\why-drift\experiments-old", r"D:\Research\why-drift\experiments"));
        assert!(!under(r"D:\Research", r"D:\Research\why-drift"));
    }
}
