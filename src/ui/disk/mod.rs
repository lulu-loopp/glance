//! The disk organizer: a window of its own (as the settings window is drawn: Mica, Windows 11's cards) showing what a
//! drive holds as a map of blocks, each file its kind's colour, each folder what it holds, two levels at a time (see
//! `disk::map`); beside it the largest files, the files by type and by time; under them a place to gather what is to
//! go, moved to the Recycle Bin together. Opened from a drive's row in the panel's Storage, from a folder found in
//! search, or by its name in search. Everything it shows is read from the drive's search index (see `worker`): no
//! drive is scanned.

mod input;
mod paint;
mod worker;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateWaitableTimerExW, SetWaitableTimer, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_BACK, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetSystemMetrics, IsWindowVisible, LoadCursorW, LoadImageW, MsgWaitForMultipleObjects, PeekMessageW, PostMessageW, RegisterClassExW,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, CS_DBLCLKS, HICON, HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, IMAGE_ICON, LR_SHARED, MINMAXINFO, MSG, PM_REMOVE, QS_ALLINPUT,
    SM_CXICON, SM_CXSMICON, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_OVERLAPPEDWINDOW,
};

use crate::disk::map::{Block, Item, Rect};
use crate::disk::{Filter, Kind, Light, What};
use crate::ui::gfx::{self, Gfx, Surface};
use crate::ui::text::Lang;
use worker::{Ask, Info, Shown, Told, Worker};

/// The window's size as it opens, and the least it may be sized to (DIPs).
const SIZE: (f32, f32) = (1100.0, 720.0);
const LEAST: (f32, f32) = (980.0, 600.0);

/// The window's own messages: asked for again (see `open`), a reading come (see `worker`), an act done (see `Acted`).
const AGAIN: u32 = WM_APP + 1;
const SHOWN: u32 = WM_APP + 2;
const ACTED: u32 = WM_APP + 3;

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

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
/// What a shell thread did (see `act`), for the window to take in as it is told (`ACTED`).
static ACTS: Mutex<Vec<Acted>> = Mutex::new(Vec::new());

/// What the window is asked to show: a drive (none: the one shown last, or the system's), a folder on it, and where it
/// was asked from (physical px: it pours out of there, see `genie`).
#[derive(Clone, Debug, Default)]
pub struct Asked {
    pub root: Option<String>,
    pub folder: Option<PathBuf>,
    pub place: Option<RECT>,
}

/// The organizer opened (or, open, turned) to the drive `root`'s root.
pub fn open_drive(root: &str, place: Option<RECT>) {
    open(Asked { root: Some(root.to_string()), folder: None, place });
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

fn open(asked: Asked) {
    *ASKED.lock().unwrap() = Some(asked);
    let window = WINDOW.load(Ordering::Acquire);
    if window != 0 {
        let _ = unsafe { PostMessageW(Some(HWND(window as *mut _)), AGAIN, WPARAM(0), LPARAM(0)) };
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    if std::thread::Builder::new().name("disk organizer".into()).spawn(run).is_err() {
        RUNNING.store(false, Ordering::Release);
    }
}

/// The window's thread: the window made as it is asked for, up till it is closed; again where it was asked for as it
/// closed.
fn run() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    loop {
        let asked = ASKED.lock().unwrap().take();
        if let Some(asked) = asked {
            up(asked);
        }
        RUNNING.store(false, Ordering::Release);
        if ASKED.lock().unwrap().is_some() && !RUNNING.swap(true, Ordering::AcqRel) {
            continue;
        }
        break;
    }
    UI.with(|ui| drop(ui.borrow_mut().take()));
    gfx::let_go();
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// The window destroyed: its state let go of once the message that told it is done.
    static GONE: Cell<bool> = const { Cell::new(false) };
}

/// Runs `act` on the window's state, unless it is in use further up this thread's stack.
fn with_ui<R>(act: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok().and_then(|mut ui| ui.as_mut().map(act)))
}

/// The window made as `asked`, and up till it is gone.
fn up(asked: Asked) {
    GONE.set(false);
    let Some(hwnd) = make(&asked) else { return };
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    let mut msg = MSG::default();
    let timer = unsafe {
        CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0).or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0)).ok().map(|timer| windows_core::Owned::new(timer))
    };
    loop {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if GONE.get() {
            WINDOW.store(0, Ordering::Release);
            UI.with(|ui| drop(ui.borrow_mut().take()));
            return;
        }
        let due = tick(Instant::now()).unwrap_or(Duration::from_secs(1));
        if !due.is_zero() {
            let ms = due.as_micros().div_ceil(1000).min(u32::MAX as u128) as u32;
            match timer.as_deref().copied().filter(|_| due < Duration::from_secs(1)) {
                Some(timer) => unsafe {
                    let _ = SetWaitableTimer(timer, &-((due.as_nanos() / 100) as i64).max(1), 0, None, None, false);
                    MsgWaitForMultipleObjects(Some(&[timer]), false, ms, QS_ALLINPUT);
                },
                None => unsafe {
                    MsgWaitForMultipleObjects(None, false, ms, QS_ALLINPUT);
                },
            }
        }
    }
}

/// The next frame drawn if it is due (the genie's, pouring, or the window's), and how long till the next.
fn tick(now: Instant) -> Option<Duration> {
    let poured = with_ui(|ui| {
        let genie = ui.genie.as_mut()?;
        if genie.draw(now) {
            return Some(false);
        }
        let opened = genie.opening();
        ui.genie = None;
        if opened {
            super::genie::reveal(ui.hwnd);
        }
        Some(!opened)
    })
    .flatten();
    match poured {
        Some(false) => return Some(Duration::ZERO),
        // Poured into its place: closed.
        Some(true) => {
            let hwnd = with_ui(|ui| ui.hwnd)?;
            let _ = unsafe { DestroyWindow(hwnd) };
            return Some(Duration::ZERO);
        }
        None => {}
    }
    with_ui(|ui| {
        ui.timers(now);
        if now >= ui.next_frame {
            ui.draw(now);
        }
        ui.next_frame.saturating_duration_since(Instant::now())
    })
}

/// How the window draws: Windows 11's colours, as the settings window takes them (see `settings_window::Palette`),
/// and the map's own (each kind's, after Explorer's icons and Fluent's).
#[derive(Clone)]
struct Palette {
    dark: bool,
    window: crate::ui::canvas::Color,
    text: crate::ui::canvas::Color,
    text2: crate::ui::canvas::Color,
    text3: crate::ui::canvas::Color,
    hover: crate::ui::canvas::Color,
    control: crate::ui::canvas::Color,
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
                window: Color::hex(0x202020, 1.0),
                text: white(1.0),
                text2: white(0.76),
                text3: white(0.5),
                hover: white(0.06),
                control: white(0.06),
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
                window: Color::hex(0xF3F3F3, 1.0),
                text: black(0.9),
                text2: black(0.6),
                text3: black(0.44),
                hover: black(0.04),
                control: black(0.05),
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

/// A block moving from one layout to the next (see `Anim`): from where, how seen, to where; going (not in the next
/// layout).
#[derive(Clone)]
struct Moving {
    placed: Placed,
    from: Rect,
    to: Rect,
    from_alpha: f32,
    to_alpha: f32,
    gone: bool,
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
    /// What is gathered and not shown in the foot.
    Gathered(Vec<usize>),
}

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
    Drive(usize),
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

/// The window's state, on its thread.
struct Ui {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    mica: bool,
    scale: f32,
    lang: Lang,
    palette: Palette,
    genie: Option<super::genie::Genie>,
    place: Option<RECT>,
    next_frame: Instant,
    last_frame: Instant,
    by_hand: bool,
    drawn_size: Option<(u32, u32)>,
    worker: Worker,
    serial: u64,
    /// The drives offered, the one shown, the folder shown (as asked: shown as soon as it is read).
    drives: Vec<crate::disk::act::Drive>,
    root: String,
    focus: PathBuf,
    /// What was read last, and its drive's state: its files being found still, or it gone.
    shown: Option<Rc<Shown>>,
    listing: bool,
    /// The map as laid out (its size), moving from the last, and what each block and piece is dimmed to (eased).
    laid: Vec<Placed>,
    laid_size: (f32, f32),
    anim: Option<Anim>,
    change: Option<Change>,
    dims: HashMap<Key, f32>,
    /// The block under the pointer (its place in the layout), the row; the file chosen (by its path).
    hover: Option<usize>,
    row_hover: Option<Target>,
    chosen: Option<PathBuf>,
    filter: Filter,
    light: Option<Light>,
    peek: Option<Light>,
    tab: Tab,
    tab_mark: (f32, f32),
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
    /// What the used part of the drive's meter shows, eased toward what it is (bytes), and for which drive.
    used_shown: (String, f64),
    /// Where the map was last drawn (DIPs in the window), and the foot.
    map_at: Rect,
    foot_at: Rect,
    /// This PC's OneDrive folders: what is in them may be left in the cloud alone.
    clouds: Vec<PathBuf>,
}

/// The window made, as `asked`, drawn and shown.
fn make(asked: &Asked) -> Option<HWND> {
    let (work, scale) = crate::panel::work_area_at_cursor()?;
    let scale = scale as f32;
    let gfx = gfx::for_area(work).ok()?;
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let icon = |size| LoadImageW(Some(instance.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_SHARED).ok().map(|h| HICON(h.0));
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("GlanceDisk"),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: icon(GetSystemMetrics(SM_CXICON)).unwrap_or_default(),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let (wide, high) = ((work.right - work.left) as f32, (work.bottom - work.top) as f32);
        let width = (SIZE.0 * scale).min(wide).round() as i32;
        let height = (SIZE.1 * scale).min(high).round() as i32;
        let (x, y) = (work.left + (wide as i32 - width) / 2, work.top + (high as i32 - height) / 2);
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, w!("GlanceDisk"), w!("Glance"), WS_OVERLAPPEDWINDOW, x, y, width, height, None, None, Some(instance.into()), None).ok()?;
        let mica = crate::ui::settings_window::windows_build() >= crate::ui::settings_window::FIRST_MICA_BUILD;
        if mica {
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_MAINWINDOW as *const _ as *const _, 4);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        }
        let surface = Surface::new(&gfx, hwnd).ok()?;
        let lang = crate::update::language();
        let worker = Worker::start(hwnd, SHOWN);
        let mut ui = Ui {
            hwnd,
            gfx,
            surface: Some(surface),
            mica,
            scale: GetDpiForWindow(hwnd) as f32 / 96.0,
            lang,
            palette: Palette::new(false),
            genie: None,
            place: asked.place,
            next_frame: Instant::now(),
            last_frame: Instant::now(),
            by_hand: false,
            drawn_size: None,
            worker,
            serial: 0,
            drives: crate::disk::act::drives(),
            root: String::new(),
            focus: PathBuf::new(),
            shown: None,
            listing: false,
            laid: Vec::new(),
            laid_size: (0.0, 0.0),
            anim: None,
            change: None,
            dims: HashMap::new(),
            hover: None,
            row_hover: None,
            chosen: None,
            filter: Filter::default(),
            light: None,
            peek: None,
            tab: Tab::Largest,
            tab_mark: (0.0, 0.0),
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
            used_shown: (String::new(), 0.0),
            map_at: Rect::default(),
            foot_at: Rect::default(),
            clouds: crate::disk::Places::of_this_pc().onedrive,
        };
        ui.restyle();
        ui.turn(asked);
        ui.draw(Instant::now());
        UI.with(|cell| *cell.borrow_mut() = Some(ui));
        let pours = super::genie::wanted(asked.place);
        if pours.is_some() {
            super::genie::hide_to_open(hwnd);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
        if let Some(place) = pours {
            match super::genie::opening(hwnd, place) {
                Some(genie) => {
                    with_ui(|ui| ui.genie = Some(genie));
                }
                None => super::genie::reveal(hwnd),
            }
        }
        Some(hwnd)
    }
}

fn point(lparam: LPARAM) -> (i32, i32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        AGAIN => {
            let asked = ASKED.lock().unwrap().take();
            if let Some(asked) = asked {
                with_ui(|ui| ui.turn(&asked));
                let _ = unsafe { SetForegroundWindow(hwnd) };
            }
            LRESULT(0)
        }
        SHOWN => {
            with_ui(Ui::take_shown);
            LRESULT(0)
        }
        ACTED => {
            let acts = std::mem::take(&mut *ACTS.lock().unwrap());
            with_ui(|ui| acts.into_iter().for_each(|acted| ui.acted(acted)));
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT { x: (LEAST.0 * scale).round() as i32, y: (LEAST.1 * scale).round() as i32 };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            with_ui(|ui| ui.scale = ((wparam.0 & 0xFFFF) as f32) / 96.0);
            let _ = unsafe { SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE) };
            LRESULT(0)
        }
        WM_ENTERSIZEMOVE => {
            with_ui(|ui| ui.by_hand = true);
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            with_ui(|ui| {
                ui.by_hand = false;
                ui.draw(Instant::now());
            });
            LRESULT(0)
        }
        WM_SIZE => {
            let size = ((lparam.0 & 0xFFFF) as u32, ((lparam.0 >> 16) & 0xFFFF) as u32);
            if wparam.0 != windows::Win32::UI::WindowsAndMessaging::SIZE_MINIMIZED as usize {
                with_ui(|ui| {
                    if ui.drawn_size != Some(size) {
                        ui.draw(Instant::now());
                    }
                });
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            with_ui(Ui::restyle);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with_ui(|ui| {
                if ui.pointer.is_none() {
                    let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                }
                ui.moved(x as f32 / ui.scale, y as f32 / ui.scale);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_ui(Ui::left);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            unsafe { SetCapture(hwnd) };
            with_ui(|ui| ui.pressed(x as f32 / ui.scale, y as f32 / ui.scale, false));
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let (x, y) = point(lparam);
            unsafe { SetCapture(hwnd) };
            with_ui(|ui| ui.pressed(x as f32 / ui.scale, y as f32 / ui.scale, true));
            LRESULT(0)
        }
        // (Let go of before the capture is: letting go of it tells the window at once, which ends the press.)
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            with_ui(|ui| ui.released(x as f32 / ui.scale, y as f32 / ui.scale));
            let _ = unsafe { ReleaseCapture() };
            LRESULT(0)
        }
        // The capture taken by another window: what was pressed or dragged is let go of.
        WM_CAPTURECHANGED if lparam.0 != hwnd.0 as isize => {
            with_ui(|ui| {
                ui.press = None;
                ui.drag = None;
                ui.next_frame = Instant::now();
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16;
            with_ui(|ui| ui.wheel(delta));
            LRESULT(0)
        }
        WM_KEYDOWN => match VIRTUAL_KEY(wparam.0 as u16) {
            VK_ESCAPE => {
                if with_ui(Ui::escape).unwrap_or(false) {
                    let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
                }
                LRESULT(0)
            }
            VK_BACK => {
                with_ui(Ui::up_a_folder);
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        },
        WM_ACTIVATE => {
            let front = wparam.0 & 0xFFFF != WA_INACTIVE as usize;
            unsafe {
                let _ = SetWindowPos(hwnd, Some(if front { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
            crate::app().controller.disk_front(front.then_some(hwnd));
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // Closed: what waits to go goes now, and the window pours into the place it was asked from, if it can.
        WM_CLOSE => {
            if with_ui(Ui::pour_away).unwrap_or(false) {
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DESTROY => {
            crate::app().controller.disk_front(None);
            GONE.set(true);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
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

impl Ui {
    /// Follows the system's light or dark, and its language.
    fn restyle(&mut self) {
        let app = crate::app();
        let theme = crate::ui::prefs::Prefs::resolve(&app.settings.lock().unwrap().view, &app.controller.known_modules()).theme;
        let dark = match theme {
            crate::ui::prefs::ThemePref::Light => false,
            crate::ui::prefs::ThemePref::Dark => true,
            _ => crate::os::apps_dark(),
        };
        self.palette = Palette::new(dark);
        self.lang = crate::update::language();
        unsafe {
            let dark = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(self.lang.pick("磁盘整理", "Disk Organizer")));
        }
        self.next_frame = Instant::now();
    }

    /// Turned to what `asked` says: a drive (its root, a folder on it), as it is now.
    fn turn(&mut self, asked: &Asked) {
        self.drives = crate::disk::act::drives();
        let last = crate::app().settings.lock().unwrap().disk_drive.clone();
        let system = std::env::var("SystemDrive").map(|drive| format!("{}\\", drive.to_ascii_uppercase())).unwrap_or_else(|_| "C:\\".into());
        let root = asked
            .root
            .clone()
            .or(last)
            .filter(|root| self.drives.iter().any(|drive| drive.root.eq_ignore_ascii_case(root)))
            .or_else(|| self.drives.iter().find(|drive| drive.root.eq_ignore_ascii_case(&system)).map(|drive| drive.root.clone()))
            .or_else(|| self.drives.first().map(|drive| drive.root.clone()));
        let Some(root) = root else {
            self.root.clear();
            self.next_frame = Instant::now();
            return;
        };
        let folder = asked.folder.clone().unwrap_or_else(|| PathBuf::from(&root));
        if asked.place.is_some() {
            self.place = asked.place;
        }
        if !root.eq_ignore_ascii_case(&self.root) {
            self.to_drive(&root);
        }
        self.go_to(folder);
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
        self.ask();
    }

    /// The folder above the one shown, shown.
    fn up_a_folder(&mut self) {
        if let Some(parent) = self.focus.parent().filter(|_| !self.at_root()) {
            self.go_to(parent.to_path_buf());
        }
    }

    fn at_root(&self) -> bool {
        self.focus.as_os_str().len() <= self.root.len()
    }

    /// Esc: a menu or a card closed, else the folder above shown; at the drive's root, whether the window is to close.
    fn escape(&mut self) -> bool {
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
        // (As large as the map can be: the tree is copied out as far down as that could draw.)
        let (width, height) = self.client();
        let card = paint::geo(width, height).card;
        let area = ((card.w - 24.0) * (card.h - 60.0)).max(10_000.0);
        self.worker.ask(Ask { root: self.root.clone(), focus: self.focus.clone(), filter: self.filter, light: self.light.clone(), area, serial: self.serial });
        self.next_frame = Instant::now();
    }

    fn client(&self) -> (f32, f32) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        ((rect.right - rect.left) as f32 / self.scale, (rect.bottom - rect.top) as f32 / self.scale)
    }

    /// What the reading thread read, taken in: the map laid out anew from it, moving from the last.
    fn take_shown(&mut self) {
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
                self.shown = Some(Rc::new(*shown));
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
        // Laid out as it was (the drive changed, not what the map shows of it): nothing moves.
        let same = next.len() == self.laid.len() && next.iter().zip(&self.laid).all(|(next, was)| next.key == was.key && same_rect(next.block.at, was.block.at));
        let change = change.filter(|_| !same);
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
                self.anim = Some(moving(now, &next, change));
            }
            _ => self.anim = None,
        }
        self.laid = next;
        // The block under the pointer, as laid out now.
        self.hover = self.pointer.and_then(|(x, y)| self.block_at(x, y));
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

    /// The blocks as they are seen now: each's layout and where it is and how seen (moving, as far as it has).
    fn now_rects(&self, now: Instant) -> Vec<(Placed, Rect, f32, bool)> {
        match &self.anim {
            Some(anim) => {
                let t = anim_t(anim, now);
                anim.moving.iter().map(|moving| (moving.placed.clone(), lerp_rect(moving.from, moving.to, t), moving.from_alpha + (moving.to_alpha - moving.from_alpha) * t, moving.placed.block.group)).collect()
            }
            None => self.laid.iter().map(|placed| (placed.clone(), placed.block.at, 1.0, placed.block.group)).collect(),
        }
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
    fn timers(&mut self, now: Instant) {
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

    /// Asked to close: what waits to go goes now; poured into the place it was asked from, if it can be. Whether it is
    /// (else it closes at once).
    fn pour_away(&mut self) -> bool {
        self.commit_now();
        if self.genie.as_ref().is_some_and(|genie| !genie.opening()) {
            return true;
        }
        if self.genie.take().is_some() {
            super::genie::reveal(self.hwnd);
        }
        let Some(place) = super::genie::wanted(self.place).and_then(|place| super::genie::back_into(self.hwnd, place)) else { return false };
        if unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(self.hwnd) }.as_bool() {
            return false;
        }
        let Some(genie) = super::genie::closing(self.hwnd, place, |_| {}) else { return false };
        self.genie = Some(genie);
        WINDOW.store(0, Ordering::Release);
        true
    }

    /// The window drawn as it is now.
    fn draw(&mut self, now: Instant) {
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.064);
        self.last_frame = now;
        if !self.follow_device() {
            self.next_frame = now + Duration::from_millis(250);
            return;
        }
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        let size = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
        let gfx = self.gfx.clone();
        let mut surface = self.surface.take().unwrap();
        let sizing = gfx::sizing(self.by_hand);
        let mut moving = false;
        let drawn = surface.draw(&gfx, size, self.scale, |frame| {
            frame.origin(0.0, 0.0);
            moving = self.paint(frame, now, dt);
        });
        drop(sizing);
        self.surface = Some(surface);
        self.drawn_size = drawn.is_ok().then_some(size);
        if let Err(error) = &drawn {
            gfx::failed("disk organizer", &self.gfx, Some(error));
            self.next_frame = now + Duration::from_millis(250);
            return;
        }
        self.gfx.sweep();
        // Moving: the next frame at once (the screen's next); else when a timer is due, or a while.
        let due = [self.pending.as_ref().map(|pending| pending.since + UNDO_FOR), self.said.as_ref().map(|said| said.since + SAID_FOR), self.went.as_ref().map(|went| went.until)].into_iter().flatten().min();
        self.next_frame = if moving || self.pending.is_some() { now + Duration::from_millis(8) } else { due.unwrap_or(now + Duration::from_secs(3600)).max(now + Duration::from_millis(8)) };
    }

    fn follow_device(&mut self) -> bool {
        let Ok(current) = gfx::for_window(self.hwnd) else { return false };
        if Rc::ptr_eq(&current, &self.gfx) && self.surface.is_some() {
            return true;
        }
        self.surface = None;
        match Surface::new(&current, self.hwnd) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.gfx = current;
                true
            }
            Err(error) => {
                gfx::failed("disk organizer", &current, Some(&error));
                false
            }
        }
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

/// The blocks moving from how they are seen now (`now`) to the layout `next`, as `change` says.
fn moving(now: Vec<(Placed, Rect, f32, bool)>, next: &[Placed], change: Change) -> Anim {
    let map = next.iter().fold(Rect::default(), |all, placed| {
        let r = placed.block.at;
        Rect { x: 0.0, y: 0.0, w: all.w.max(r.x + r.w), h: all.h.max(r.y + r.h) }
    });
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
    let mut moves = Vec::new();
    for placed in next {
        let from = now.iter().find(|(was, _, alpha, _)| !fading && was.key == placed.key && *alpha > 0.01);
        match from {
            Some((_, at, alpha, _)) => moves.push(Moving { placed: placed.clone(), from: *at, to: placed.block.at, from_alpha: *alpha, to_alpha: 1.0, gone: false }),
            None => {
                let (at, alpha) = enter(placed.block.at);
                moves.push(Moving { placed: placed.clone(), from: at, to: placed.block.at, from_alpha: alpha, to_alpha: 1.0, gone: false });
            }
        }
    }
    for (was, at, alpha, _) in now {
        if alpha > 0.01 && (fading || !next.iter().any(|placed| placed.key == was.key)) {
            let (to, to_alpha) = exit(at);
            moves.push(Moving { placed: was, from: at, to, from_alpha: alpha, to_alpha, gone: true });
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
}
