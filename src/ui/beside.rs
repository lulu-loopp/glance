//! The page beside the panel or a widget: what opens from them and needs
//! more room than they have, laid out beside them on their inner side, as tall as they are:
//!
//! - quick launch's adding: what is kept, in its order (dragged into another, taken off by its cross), and what may
//!   be added, by its kind: every program the Start menu lists (searched by name or pinyin), files and folders used
//!   lately; what is not listed chosen on the PC, or dropped on it, as on quick launch;
//! - a note written (see `note`): its colour chosen, it stuck on the desktop or taken to the Recycle Bin from its menu.
//!
//! A window of its own thread (a single-threaded apartment with OLE, for the shell's pictures and what is dropped on
//! it), drawn as Glance draws. The adding and a thing's page are drawn in the skin as the panel is: its sheet over
//! what is behind it (taken as a widget takes it, see `ui::scene`), its corners, its edge and its shadow, in room left
//! round it for that; a note's page on a ground of its own. It takes the keyboard while it is up (words are typed into its
//! search); it goes as Esc is pressed, as its close is, or as another program's window is clicked (the panel, which
//! takes no focus, leaves it up: the panel and the page are one place). The panel stays open while it is up.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, InvalidateRect, MonitorFromRect, ValidateRect, LOGFONTW, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionFontW, ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::canvas::{Align, Canvas, Color, Point};
use super::gfx::{self, Frame, Gfx, Surface};
use super::icons::Icon;
use super::theme::{Skin, Theme};
use super::typing::Typing;
use crate::station::apps::App;
use crate::station::found::{self, Found};

/// What the page shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Page {
    /// Quick launch's adding.
    Picker,
    /// A note, by its file's name in the notes' folder (none: a new one, named by its title as it is first kept).
    Note(Option<String>),
}

/// Where the page goes: beside `by` (screen px: the panel or the widget it opens from), on the side `side` (away from
/// the screen's edge it is at), in the skin and its light or dark.
#[derive(Clone)]
pub struct Opening {
    pub page: Page,
    pub by: RECT,
    pub side: Side,
    pub skin: Skin,
    pub dark: bool,
    /// A note's folder, where it is not the one the settings name (a note opened again where it was read: see
    /// `WM_DPICHANGED`).
    pub folder: Option<std::path::PathBuf>,
    /// The window it opens from (a panel's, a widget's): that one stays up while it is (see `up_for`).
    pub owner: isize,
}

/// Which side of what it opens from the page goes on, where there is room: left of it, right of it, or under it (the
/// top edge's panel).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    Below,
}

/// How wide it is (DIPs); its head, its search box, a row, a letter's heading, its foot; the room round its parts.
const WIDTH: f32 = 360.0;
const HEAD: f32 = 52.0;
const SEARCH: f32 = 34.0;
const ROW: f32 = 40.0;
const LETTER: f32 = 26.0;
const PAD: f32 = 16.0;
/// The gap between it and what it opens from (DIPs).
const GAP: f32 = 8.0;
/// How long a search may be.
const SEARCH_MOST: usize = 64;
/// Quick launch's adding (DIPs, from its top): what is kept, a cell each; its kinds' segment; the search; a row of
/// a file or folder (its name and where it is); its foot (the button that chooses on the PC, and what it says).
const STRIP_TOP: f32 = 50.0;
const STRIP_CELL: f32 = 42.0;
const KINDS_TOP: f32 = 102.0;
const KINDS_H: f32 = 34.0;
const SEARCH_TOP: f32 = 146.0;
const LIST_TOP: f32 = 188.0;
const ROW_TWO: f32 = 46.0;
const PICK_FOOT: f32 = 74.0;
/// How far a kept thing is moved, held, before it is dragged into another place (DIPs).
const DRAG_FROM: f32 = 4.0;

/// What is asked of its thread.
enum Ask {
    Open(Opening),
    Close,
    /// What it opened from as it is now: where (physical px), in which skin, light or dark.
    Follow(RECT, Skin, bool),
    /// The skin chosen does not see the desktop: the scene kept from page to page let go of.
    Unsee,
    /// Glance going: what is written on the page kept (see `Beside::keep_as_glance_goes`), told when it is.
    Quitting(Sender<()>),
}

static ASKS: OnceLock<Mutex<Sender<Ask>>> = OnceLock::new();
/// Its thread's window for asks (message-only), and whether a page is up.
static POST: AtomicIsize = AtomicIsize::new(0);
static UP: AtomicBool = AtomicBool::new(false);
/// Its messages: something asked; the page to go, as soon as it is free (see `gone`); the page made again as its
/// screen's scale changed.
const ASKED: u32 = WM_APP + 7;
const GO_SOON: u32 = WM_APP + 8;
/// The page's own part of what was written in a note's words' box (see `Beside::written`), done once the page is free:
/// the box was given a message while the page was in the middle of another.
const WRITTEN: u32 = WM_APP + 9;
/// Something that may be added dragged over the page (wparam 1), or gone (0).
const DROPPING: u32 = WM_APP + 10;
/// Its timers: the caret's blink, drawn again while pictures are being made, and a note kept a moment after it was
/// last written in (the text services' own timers are numbered past these).
const BLINK: usize = 1;
const AGAIN: usize = 2;
const KEEP: usize = 3;
/// Its timer for a note's file looked at, for what changed outside (see `Beside::read_outside`), and how often.
const FOLLOW: usize = 4;
const FOLLOW_MS: u32 = 1000;
/// Its timer for showing it, drawn without the desktop behind it, where that does not come (it cannot be taken), and
/// how long it waits for it.
const SHOW: usize = 5;
/// How far in from a listed row's plate (as wide as the search) what is in it is (DIPs).
const ROW_IN: f32 = 8.0;
/// Its timer for drawing again while a mark slides (each frame's time).
const SLIDING: usize = 6;
const SHOW_WITHIN_MS: u32 = 400;

/// The page's window while one is up (0 while none is): what a page's going asked for later is for (see `go_soon`).
static PAGE: AtomicIsize = AtomicIsize::new(0);

/// The page up (its window `page`) to go as soon as it is free: asked of its thread's loop, for that page alone (one
/// opened meanwhile, another note pressed on the panel as this one was left for it, stays).
fn go_soon(page: isize) {
    let post = POST.load(Ordering::Acquire);
    let _ = unsafe { PostMessageW(Some(HWND(post as *mut _)), GO_SOON, WPARAM(page as usize), LPARAM(0)) };
}

/// The page that goes as the note it put on the desktop shows there (see `stuck_shown`): its window, and the note's
/// name.
static LEAVES_FOR: Mutex<Option<(isize, String)>> = Mutex::new(None);

/// The page up to go once the note `name` it puts on the desktop shows there: else, gone first, nothing was there a
/// moment (the note shows once its widget is made and drawn, on the panel's thread).
fn leave_for(name: &str) {
    *LEAVES_FOR.lock().unwrap() = Some((PAGE.load(Ordering::Acquire), name.to_string()));
}

/// The note `name` shown on the desktop (or not to be: one there already raised, or none could be made): the page
/// that put it there goes.
pub fn stuck_shown(name: &str) {
    let mut leaves = LEAVES_FOR.lock().unwrap();
    if leaves.as_ref().is_some_and(|(_, waits)| waits == name) {
        let (page, _) = leaves.take().unwrap();
        go_soon(page);
    }
}

/// Whether a page's going asked for `meant` (its window) is for the page up now, `up`.
fn goes_now(meant: isize, up: isize) -> bool {
    meant != 0 && meant == up
}

/// Whether the page is up.
pub fn is_up() -> bool {
    UP.load(Ordering::Acquire)
}

/// The note written in on the page up, by its name (a new one, once it is first kept).
static WRITING: Mutex<Option<String>> = Mutex::new(None);

/// The note written in on the page up, opened from `window`, by its name (a new one, once it is first kept).
pub fn writing_for(window: HWND) -> Option<String> {
    up_for(window).then(|| WRITING.lock().unwrap().clone()).flatten()
}

/// The window the page up opened from (see `Opening::owner`).
static OWNER: AtomicIsize = AtomicIsize::new(0);

/// Whether the page is up, opened from `window`.
pub fn up_for(window: HWND) -> bool {
    is_up() && OWNER.load(Ordering::Acquire) == window.0 as isize
}

/// The page `opening` asks for opened (another up goes), on its thread (started when first asked).
pub fn open(opening: Opening) {
    crate::perf::mark(|| "beside asked".into());
    ask(Ask::Open(opening));
}

/// The page up kept beside what it opened from as that is now (`by`, physical px; drawn in `skin`, light or `dark`):
/// moved along, where only that moved; opened again so, where its skin or its size is another.
pub fn follow(by: RECT, skin: Skin, dark: bool) {
    // (Asked as what it opened from changes, not each time that is placed as it was.)
    let mut asked = FOLLOWED.lock().unwrap();
    if is_up() && *asked != Some((by, skin, dark)) {
        *asked = Some((by, skin, dark));
        ask(Ask::Follow(by, skin, dark));
    }
}

/// The skin chosen: one that does not see the desktop, the scene of what is behind the page that its thread keeps from
/// page to page (see `Beside::look_behind`) let go of, the page up or not (where it has a thread yet).
pub fn follow_skin(skin: Skin) {
    if !skin.sees_backdrop() && ASKS.get().is_some() {
        ask(Ask::Unsee);
    }
}

/// What the page up was last asked to follow, or opened from (see `follow`).
static FOLLOWED: Mutex<Option<(RECT, Skin, bool)>> = Mutex::new(None);

/// Glance going: what is written on the page up kept before it goes, as long as that takes (a while at most). Else
/// what was written since it was last kept (it is kept a moment after the last key), or all of it where it could not
/// be (its folder's drive gone), went with Glance.
pub fn keep_as_glance_goes() {
    if !is_up() {
        return;
    }
    let (told, kept) = std::sync::mpsc::channel();
    ask(Ask::Quitting(told));
    let _ = kept.recv_timeout(std::time::Duration::from_secs(5));
}

/// The page up, gone.
pub fn close() {
    if is_up() {
        ask(Ask::Close);
    }
}

fn ask(ask: Ask) {
    let sender = ASKS.get_or_init(|| {
        let (sender, asks) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("glance: beside".into()).spawn(move || run(asks)).expect("a thread started");
        Mutex::new(sender)
    });
    if sender.lock().unwrap().send(ask).is_ok() {
        let post = POST.load(Ordering::Acquire);
        if post != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(post as *mut _)), ASKED, WPARAM(0), LPARAM(0)) };
        }
    }
}

/// The page on its thread: its window, how it is drawn, and what it shows.
struct Beside {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    surface: Option<Surface>,
    _showing: super::life::Showing,
    scale: f32,
    theme: Rc<Theme>,
    lang: super::text::Lang,
    page: Page,
    /// Drawn in the skin as the panel is (see the top), in room `inset` (DIPs) round it for its shadow; how frosted.
    inset: f32,
    frost: f32,
    /// What a press began on (a press acts only let go on the same), and Windows' chooser asked for by one (opened
    /// once the page is free: see `WM_LBUTTONUP`).
    pressed: Option<Hit>,
    choosing: bool,
    /// Quick launch's adding: the kind listed; something dragged over it to be added; a kept thing held (what opens
    /// it, where it was pressed, whether it is being dragged); how far what is kept is scrolled along, and the most.
    kind: Kinds,
    dropping: bool,
    holding: Option<(String, (f32, f32), bool)>,
    strip_scroll: f32,
    strip_most: f32,
    search: Typing,
    /// How far its list is scrolled, and the most it can be.
    scroll: f32,
    scroll_most: f32,
    pointer: Option<(f32, f32)>,
    hits: Vec<(f32, f32, f32, f32, Hit)>,
    /// Where the caret was drawn (DIPs: its left, its top, its height), for the input method's words to go there.
    caret: Option<(f32, f32, f32)>,
    caret_on: bool,
    /// A picture was not ready as it was last drawn: drawn again soon.
    waiting: bool,
    /// The note written, on a note's page.
    writing: Option<super::note::Writing>,
    /// How it was opened (made again so as its screen's scale changes).
    opening: Option<Opening>,
    /// A note kept in a file, moved by its head: where its corner was as it was taken (pulled far enough, the note is
    /// stuck on the desktop where it is put: see `carry_note`).
    moved_from: Option<POINT>,
    /// Shown on the screen yet: not before it is first drawn whole (on glass and Windows 11, with the desktop behind it,
    /// see `ready`), else it came up as it was drawn, and a moment after changed (the desktop come in behind).
    shown: bool,
    /// The kind listed's mark, sliding to it as another is chosen (glass's: see `view::glass_choice`).
    marks: super::thumb::Dots<Kinds>,
}

thread_local! {
    /// A note's menu up, with what each row does.
    static MENU: RefCell<Option<(super::menu::Menu, Vec<super::note::Choice>)>> = const { RefCell::new(None) };
}

/// What quick launch's adding lists: the Start menu's programs, files used lately, the folders they are in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kinds {
    Programs,
    Files,
    Folders,
}

/// A row of what may be added: its name, what opens it, what is said under it, the letter its group is under (a
/// program's), and whether it is a file or folder (drawn by its mark) or a program (by its picture).
struct PickRow {
    name: String,
    target: String,
    /// Where it is, and when it came (each shown whole as far as there is room: the time kept, the place shortened).
    about: String,
    when: String,
    letter: Option<char>,
    on_disk: bool,
}

/// What a point on the page is on.
#[derive(Clone, PartialEq, Debug)]
enum Hit {
    Close,
    Search,
    /// Quick launch's adding: a kind listed; a kept thing, by its place, and its cross; the button that chooses on
    /// the PC.
    Kind(Kinds),
    Kept(String),
    Unkeep(String),
    Choose,
    /// A note's menu.
    NoteMenu,
    /// A thing listed, by what opens it and its name: added, or taken off where it is kept already.
    Pick(String, String),
}

thread_local! {
    static BESIDE: RefCell<Option<Beside>> = const { RefCell::new(None) };
    /// What a note's words' box answers the page's messages with while the page shows one (see `note::Writing::answer`):
    /// reached by them whatever the page is in the middle of (see `write`), and how deep they are in it.
    static ANSWER: RefCell<Option<super::note::Answer>> = const { RefCell::new(None) };
    static ANSWERING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn with<R>(act: impl FnOnce(&mut Beside) -> R) -> Option<R> {
    BESIDE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut beside| beside.as_mut().map(act)))
}

fn run(asks: Receiver<Ask>) {
    unsafe {
        let _ = windows::Win32::System::Ole::OleInitialize(None);
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceBesideAsks"), ..Default::default() };
        RegisterClassW(&class);
        let Ok(post) = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) else { return };
        POST.store(post.0 as isize, Ordering::Release);
        let page = WNDCLASSW { style: CS_DBLCLKS, lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceBeside"), hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
        RegisterClassW(&page);
        // (Asks sent before the window was there.)
        let _ = PostMessageW(Some(post), ASKED, WPARAM(0), LPARAM(0));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.hwnd == post && message.message == GO_SOON {
                if goes_now(message.wParam.0 as isize, PAGE.load(Ordering::Acquire)) {
                    gone();
                }
                continue;
            }
            if message.hwnd == post && message.message == ASKED {
                while let Ok(ask) = asks.try_recv() {
                    match ask {
                        Ask::Open(opening) => show(opening),
                        Ask::Close => gone(),
                        Ask::Unsee => BEHIND.with(|held| *held.borrow_mut() = (None, None, None)),
                        Ask::Quitting(told) => {
                            with(Beside::keep_as_glance_goes);
                            let _ = told.send(());
                        }
                        // (Moved along; or opened again as it was, in the skin now, its room round it the skin's, or as
                        // large as it is to be.)
                        Ask::Follow(by, skin, dark) => {
                            let again = with(|beside| {
                                let opening = beside.opening.clone()?;
                                if opening.by == by && opening.skin == skin && opening.dark == dark {
                                    return None;
                                }
                                let now = Opening { by, skin, dark, ..opening.clone() };
                                if opening.skin == skin && opening.dark == dark {
                                    beside.move_to(placed(&now).0);
                                    beside.opening = Some(now);
                                    return None;
                                }
                                // (Kept first: kept apart, as a conflict's copy, it is that copy opened again. Not
                                // kept, it stays as it is, saying so, as it would not go.)
                                if !beside.keep() {
                                    beside.redraw();
                                    return None;
                                }
                                let page = match &beside.writing {
                                    Some(writing) => Page::Note(writing.name.clone()),
                                    None => beside.page.clone(),
                                };
                                let folder = beside.writing.as_ref().map(|writing| writing.folder.clone());
                                Some(Opening { page, folder, ..now })
                            })
                            .flatten();
                            if let Some(opening) = again {
                                gone();
                                show(opening);
                            }
                        }
                    }
                }
                continue;
            }
            // A note's menu up: its own messages its, then what was chosen done.
            let handled = MENU.with(|menu| menu.borrow_mut().as_mut().map(|(open, _)| open.handle(&message)));
            match handled {
                Some(super::menu::Handled::Taken) => continue,
                Some(super::menu::Handled::Done(choice)) => {
                    if let Some((open, choices)) = MENU.with(|menu| menu.borrow_mut().take()) {
                        open.close();
                        match choice.and_then(|row| choices.get(row).copied()) {
                            Some(choice) => {
                                let gone_now = with(|beside| beside.note_chose(choice)).unwrap_or(false);
                                if gone_now {
                                    gone();
                                }
                            }
                            // (Gone with nothing chosen as another program came in front, its capture lost: the page
                            // goes with it, as it took no deactivation of its own, its menu having had the front.)
                            None if !ours(GetForegroundWindow()) => gone(),
                            None => {}
                        }
                    }
                    continue;
                }
                _ => {}
            }
            // Its menu left for another program's window: the menu and the page go (the page took no deactivation:
            // its menu had the front).
            let left = MENU.with(|menu| menu.borrow().as_ref().is_some_and(|(open, _)| open.left()));
            if left && !ours(GetForegroundWindow()) {
                if let Some((open, _)) = MENU.with(|menu| menu.borrow_mut().take()) {
                    open.close();
                }
                gone();
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Where the page `opening` asks for is on the screen (physical px, its shadow's room left out), and how many pixels a
/// DIP is there: beside what it opens from on its side away from the edge, else on the other, else over it; under the
/// top edge's panel, as tall as the room under it is.
fn placed(opening: &Opening) -> (RECT, f32) {
    let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let monitor = unsafe { MonitorFromRect(&opening.by, MONITOR_DEFAULTTONEAREST) };
    let (mut dpi, mut unused) = (96u32, 96u32);
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut screen);
        let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(monitor, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
    }
    let scale = dpi.max(96) as f32 / 96.0;
    (placed_in(opening, screen.rcWork, scale), scale)
}

/// Where the page `opening` asks for is in `room` (a screen's work area, physical px) at `scale` (see `placed`): never
/// wider or taller than the room (on a narrow screen at a large scale, a page beside the panel was wider than the
/// screen, and placing it within it failed, ending Glance).
fn placed_in(opening: &Opening, room: RECT, scale: f32) -> RECT {
    let (wide, gap) = ((WIDTH * scale).round() as i32, (GAP * scale).round() as i32);
    let least = (480.0 * scale) as i32;
    let wide = wide.min(room.right - room.left);
    let left_of = opening.by.left - gap - wide;
    let right_of = opening.by.right + gap;
    let (x, y, tall) = match opening.side {
        Side::Below => {
            let tall = (room.bottom - opening.by.bottom - gap - gap).max(least).min(room.bottom - room.top);
            ((opening.by.left + opening.by.right - wide) / 2, opening.by.bottom + gap, tall)
        }
        side => {
            let tall = (opening.by.bottom - opening.by.top).max(least).min(room.bottom - room.top);
            let x = match (side == Side::Left, left_of >= room.left, right_of + wide <= room.right) {
                (true, true, _) | (false, true, false) => left_of,
                (false, _, true) | (true, false, true) => right_of,
                _ => (opening.by.left + opening.by.right - wide) / 2,
            };
            (x, opening.by.top, tall)
        }
    };
    let x = x.clamp(room.left, room.right - wide);
    let y = y.clamp(room.top, room.bottom - tall);
    RECT { left: x, top: y, right: x + wide, bottom: y + tall }
}

/// The page `opening` asks for, up beside what it opens from.
fn show(opening: Opening) {
    // (The page up before it gone, not told: the windows that give way to a page would else come over for a moment
    // between the two.)
    let_go(false);
    // (The page before still up, its note not kept or the page in use: it stays, the new one not made.)
    if BESIDE.with(|cell| cell.try_borrow().map_or(true, |beside| beside.is_some())) {
        return;
    }
    crate::perf::mark(|| "beside thread".into());
    let Ok(gfx) = gfx::current() else { return };
    crate::perf::mark(|| "beside device".into());
    let showing = super::life::Showing::of(&gfx);
    let (page, scale) = placed(&opening);
    let (x, y, wide, tall) = (page.left, page.top, page.right - page.left, page.bottom - page.top);
    unsafe {
        // In the skin, a note's page too (as its card on the panel is): its window larger by its shadow's room each
        // way, its corners and shadow drawn as the panel's are.
        let theme = Theme::new(opening.skin, opening.dark);
        let inset = theme.margin;
        let room_px = (inset * scale).round() as i32;
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP, w!("GlanceBeside"), &HSTRING::from("Glance"), WS_POPUP, x - room_px, y - room_px, wide + 2 * room_px, tall + 2 * room_px, None, None, None, None) else { return };
        crate::perf::mark(|| "beside window".into());
        let square = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as *const _, size_of_val(&square) as u32);
        // (Shown as it is drawn, at once: Windows' fade as it came up showed it all but clear a moment first.)
        let off = windows::core::BOOL(1);
        let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const _ as *const _, size_of_val(&off) as u32);
        let beside = Beside {
            hwnd,
            gfx,
            surface: None,
            _showing: showing,
            scale,
            theme: Rc::new(theme),
            lang: crate::update::language(),
            page: opening.page.clone(),
            inset,
            frost: 0.0,
            pressed: None,
            choosing: false,
            kind: Kinds::Programs,
            dropping: false,
            holding: None,
            strip_scroll: 0.0,
            strip_most: 0.0,
            search: Typing::most(SEARCH_MOST),
            scroll: 0.0,
            scroll_most: 0.0,
            pointer: None,
            hits: Vec::new(),
            caret: None,
            caret_on: true,
            waiting: false,
            writing: None,
            opening: Some(opening.clone()),
            moved_from: None,
            shown: false,
            marks: super::thumb::Dots::new(),
        };
        let page = beside.page.clone();
        let theme = beside.theme.clone();
        BESIDE.with(|cell| *cell.borrow_mut() = Some(beside));
        // A note: its words read, and a box for them where the page writes them.
        if let Page::Note(name) = page {
            let folder = opening.folder.clone().map_or_else(crate::notes::folder, crate::notes::folder_at);
            let rect = super::note::words_box(room_px, room_px, wide, tall, scale);
            if let Some(writing) = super::note::Writing::open(hwnd, rect, &folder, name.clone(), &theme, super::note::WORDS * scale) {
                crate::perf::mark(|| "beside words dressed".into());
                let _ = writing.editor.send(WM_SETFOCUS, WPARAM(0), LPARAM(0));
                ANSWER.with(|held| *held.borrow_mut() = Some(writing.answer()));
                *WRITING.lock().unwrap() = name;
                with(|beside| beside.writing = Some(writing));
            }
        }
        OWNER.store(opening.owner, Ordering::Release);
        *FOLLOWED.lock().unwrap() = Some((opening.by, opening.skin, opening.dark));
        PAGE.store(hwnd.0 as isize, Ordering::Release);
        UP.store(true, Ordering::Release);
        // The widgets kept under it while it is up (one drawn again under it, as a note stuck on the desktop is as
        // it is written in over it, came over it: on top of what is written, the input method's words broken off).
        crate::app().controller.page_front(Some(hwnd));
        // The Start menu read (again): drawn once it is.
        let told = HWND(hwnd.0);
        let told = told.0 as isize;
        found::look(move || {
            let _ = PostMessageW(Some(HWND(told as *mut _)), WM_TIMER, WPARAM(AGAIN), LPARAM(0));
        });
        // The choosing of programs: what is dropped on it kept at hand, as on quick launch; while it is dragged over,
        // the page says so. (A note's page takes nothing dropped: what is dropped there is not to be kept at hand.)
        if opening.page == Page::Picker {
            let told = hwnd.0 as isize;
            let target = crate::station::drop::Target::new(move |dragged| {
                use crate::station::drop::Dragged;
                let over = matches!(dragged, Dragged::Over(_));
                let _ = PostMessageW(Some(HWND(told as *mut _)), DROPPING, WPARAM(usize::from(over)), LPARAM(0));
                if matches!(dragged, Dragged::Dropped(_)) {
                    crate::station::run::caught(dragged, None, crate::station::run::Lands::Apps, None);
                }
            });
            let _ = windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target);
            DROP_TARGET.with(|held| *held.borrow_mut() = Some(target));
        }
        let _ = SetTimer(Some(hwnd), BLINK, windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime(), None);
        let _ = SetTimer(Some(hwnd), FOLLOW, FOLLOW_MS, None);
        // Shown once drawn whole (see `Beside::shown`), or after a while drawn as it can be.
        let _ = SetTimer(Some(hwnd), SHOW, SHOW_WITHIN_MS, None);
        crate::perf::mark(|| "beside set up".into());
        with(Beside::draw);
        crate::perf::mark(|| "beside first drawn".into());
    }
}

/// How long what is behind a page stands before it is taken again.
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(2);

thread_local! {
    /// What is behind the page (see `Beside::look_behind`): its scene (kept from page to page), what was taken last
    /// (and its bitmap, made as it is drawn), and when it was last asked for.
    #[allow(clippy::type_complexity)]
    static BEHIND: RefCell<(Option<super::scene::Scene>, Option<(super::backdrop::Capture, Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>)>, Option<std::time::Instant>)> = const { RefCell::new((None, None, None)) };
}

thread_local! {
    /// What takes drops on the page, while it is up.
    static DROP_TARGET: RefCell<Option<windows::Win32::System::Ole::IDropTarget>> = const { RefCell::new(None) };
}

/// The page up, gone (a note on it kept first: one that cannot be kept leaves it up, saying so). Asked while the page
/// is in use (a message come in the middle of what it does), it goes as soon as it is free.
fn gone() {
    let_go(true);
}

/// The page up gone (see `gone`), what gives way to it told so (`told`), or not (another comes at once).
fn let_go(told: bool) {
    // (Its words' box given a message meanwhile, and in the middle of it: the page goes once it has answered.)
    let answering = ANSWERING.get() > 0;
    let held = if answering { Err(()) } else { BESIDE.with(|cell| cell.try_borrow_mut().map(|mut beside| beside.take()).map_err(drop)) };
    let Ok(mut held) = held else {
        go_soon(PAGE.load(Ordering::Acquire));
        return;
    };
    if let Some(beside) = held.as_mut() {
        if !beside.keep() {
            beside.redraw();
            BESIDE.with(|cell| *cell.borrow_mut() = held);
            return;
        }
    }
    if let Some((open, _)) = MENU.with(|menu| menu.borrow_mut().take()) {
        open.close();
    }
    if let Some(mut beside) = held {
        // (The text services let go of before the window they write in.)
        ANSWER.with(|held| held.borrow_mut().take());
        beside.writing = None;
        unsafe {
            let _ = windows::Win32::System::Ole::RevokeDragDrop(beside.hwnd);
            let _ = DestroyWindow(beside.hwnd);
        }
        DROP_TARGET.with(|held| held.borrow_mut().take());
        BEHIND.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(scene) = held.0.as_mut() {
                scene.rest();
            }
            held.1 = None;
            held.2 = None;
        });
        if told {
            crate::app().controller.page_front(None);
        }
    }
    PAGE.store(0, Ordering::Release);
    UP.store(false, Ordering::Release);
    *WRITING.lock().unwrap() = None;
}

/// A message for a note's words' box, given it where it is one of its (see `note::Writing::answer`): what it answers
/// (none: not its, the page's own, its own timers among them, or the system's to answer as well). The page's own part is
/// done then too, or as soon as the page is free.
fn write(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<Option<LRESULT>> {
    if message == WM_TIMER && matches!(wparam.0, BLINK | AGAIN | KEEP | FOLLOW | SHOW | SLIDING) {
        return None;
    }
    let answer = ANSWER.with(|held| held.borrow().clone())?;
    ANSWERING.set(ANSWERING.get() + 1);
    let answered = answer(hwnd, message, wparam, lparam);
    ANSWERING.set(ANSWERING.get() - 1);
    if answered.is_some() && with(Beside::written).is_none() {
        let _ = unsafe { PostMessageW(Some(hwnd), WRITTEN, WPARAM(0), LPARAM(0)) };
    }
    answered
}

/// Whether `window` is one of this thread's (the page's, its menu's).
fn ours(window: HWND) -> bool {
    !window.is_invalid() && unsafe { GetWindowThreadProcessId(window, None) } == unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let point = || ((lparam.0 & 0xFFFF) as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as i16 as f32);
    // A note's words' box: the keys, the input method's, what the pointer does over it, its own timers.
    if let Some(Some(answered)) = write(hwnd, message, wparam, lparam) {
        return answered;
    }
    match message {
        WRITTEN => {
            with(Beside::written);
        }
        DROPPING => {
            with(|beside| {
                beside.dropping = wparam.0 == 1;
                beside.redraw();
                // (Drawn again once what was dropped is kept.)
                let _ = unsafe { SetTimer(Some(hwnd), AGAIN, 300, None) };
            });
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point();
            with(|beside| {
                let (x, y) = beside.on_page(x, y);
                beside.hold(x, y);
            });
        }
        WM_PAINT => {
            with(Beside::draw);
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
        }
        WM_MOUSEMOVE => {
            let (x, y) = point();
            with(|beside| {
                let (x, y) = beside.on_page(x, y);
                beside.pointer = Some((x, y));
                if let Some((_, from, moved)) = beside.holding.as_mut() {
                    *moved = *moved || ((x - from.0).powi(2) + (y - from.1).powi(2)).sqrt() > DRAG_FROM;
                }
                beside.redraw();
            });
        }
        WM_LBUTTONUP => {
            let (x, y) = point();
            let close = with(|beside| {
                let (x, y) = beside.on_page(x, y);
                if beside.holding.is_some() { beside.let_go(x) } else { beside.press(x, y) }
            })
            .unwrap_or(false);
            if close {
                gone();
            }
            // Windows' chooser, asked for: opened with the page free (its own loop hands the page its messages
            // meanwhile), what is chosen added after.
            let asked = with(|beside| std::mem::take(&mut beside.choosing).then_some((beside.kind, beside.lang))).flatten();
            if let Some((kind, lang)) = asked {
                let chosen = choose(hwnd, kind, lang);
                with(|beside| beside.add_paths(chosen));
                // (Asks come meanwhile are its thread's loop's: looked at now.)
                let post = POST.load(Ordering::Acquire);
                let _ = unsafe { PostMessageW(Some(HWND(post as *mut _)), ASKED, WPARAM(0), LPARAM(0)) };
            }
        }
        WM_CAPTURECHANGED => {
            with(|beside| {
                beside.holding = None;
                beside.pressed = None;
                beside.redraw();
            });
        }
        WM_MOUSEWHEEL => {
            let turns = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with(|beside| {
                // (Over what is kept: along it; else down the list.)
                if beside.pointer.is_some_and(|(_, y)| beside.page == Page::Picker && (STRIP_TOP..STRIP_TOP + STRIP_CELL).contains(&y)) {
                    beside.strip_scroll = (beside.strip_scroll - turns * STRIP_CELL).clamp(0.0, beside.strip_most);
                } else {
                    beside.scroll = (beside.scroll - turns * 3.0 * ROW).clamp(0.0, beside.scroll_most);
                }
                beside.redraw();
            });
        }
        // Closed as a window is (Alt+F4): gone as with Esc, its note kept first (destroyed as it is, nothing of it was).
        WM_CLOSE => {
            gone();
        }
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            if key == VK_ESCAPE {
                gone();
            } else {
                with(|beside| beside.key(key));
            }
        }
        WM_CHAR => {
            with(|beside| beside.char(wparam.0 as u16));
        }
        WM_TIMER => {
            with(|beside| {
                match wparam.0 {
                    BLINK => beside.caret_on = !beside.caret_on,
                    KEEP => {
                        let _ = unsafe { KillTimer(Some(hwnd), KEEP) };
                        beside.keep_soon();
                    }
                    FOLLOW => beside.read_outside(),
                    SHOW => {
                        let _ = unsafe { KillTimer(Some(hwnd), SHOW) };
                        beside.show_now();
                    }
                    _ => {}
                }
                // (Not shown yet, it is sent nothing to paint: drawn here, shown once it is drawn whole.)
                if beside.shown {
                    beside.redraw();
                } else {
                    beside.draw();
                }
            });
        }
        WM_IME_STARTCOMPOSITION => {
            with(|beside| beside.place_ime());
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        // A note kept in a file: taken by its head (where nothing of it is pressed) and moved as a window is by its title
        // (put down far enough away, it is stuck on the desktop there: see `carry_note`).
        WM_NCHITTEST => {
            let at = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
            if with(|beside| beside.takes_by_head(at)).unwrap_or(false) {
                return LRESULT(HTCAPTION as isize);
            }
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        // (Its head pressed twice: nothing, as a window that cannot be made larger.)
        WM_NCLBUTTONDBLCLK => {}
        WM_ENTERSIZEMOVE => {
            with(|beside| beside.moved_from = Some(beside.corner()));
        }
        // Moved by its head: kept within its screen's work area.
        WM_MOVING => {
            let place = unsafe { &mut *(lparam.0 as *mut RECT) };
            let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = unsafe { GetMonitorInfoW(MonitorFromRect(place, MONITOR_DEFAULTTONEAREST), &mut screen) };
            let room = screen.rcWork;
            let (wide, tall) = (place.right - place.left, place.bottom - place.top);
            let left = place.left.min(room.right - wide).max(room.left);
            let top = place.top.min(room.bottom - tall).max(room.top);
            *place = RECT { left, top, right: left + wide, bottom: top + tall };
            return LRESULT(1);
        }
        WM_EXITSIZEMOVE => {
            with(Beside::carry_note);
        }
        // Another program's window come in front: the page goes (the panel takes no focus, and leaves it up; its own
        // menu leaves it up too). (The choosing of programs stays: a program is dragged onto it from another window,
        // Explorer pressed to take it. It goes by its own close, Esc, or as the panel or widget it opened from goes.)
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            if !ours(HWND(lparam.0 as *mut _)) && !with(|beside| beside.page == Page::Picker).unwrap_or(false) {
                // (Gone once the page is free: this may come in the middle of what it does.)
                go_soon(hwnd.0 as isize);
            }
        }
        // Its screen's scale changed (moved to another screen, or the scale chosen): made again at the new one.
        WM_DPICHANGED => {
            // (A note opened again as it is now, from where it was read: one new when it opened and kept since, or
            // kept apart as a conflict's copy, by its name now; in its folder, whatever the settings name since.)
            let again = with(|beside| {
                // (Not kept, it stays as it is, saying so, as it would not go.)
                if !beside.keep() {
                    beside.redraw();
                    return None;
                }
                let page = match &beside.writing {
                    Some(writing) => Page::Note(writing.name.clone()),
                    None => beside.page.clone(),
                };
                let folder = beside.writing.as_ref().map(|writing| writing.folder.clone());
                beside.opening.as_ref().map(|opening| Opening { page, folder, ..opening.clone() })
            })
            .flatten();
            if let Some(opening) = again {
                gone();
                show(opening);
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}

impl Beside {
    fn redraw(&self) {
        let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    /// Its page put at `page` (physical px, its shadow's room left out), as large as that is; a note's words' box
    /// with it.
    fn move_to(&mut self, page: RECT) {
        let room = (self.inset * self.scale).round() as i32;
        let (wide, tall) = (page.right - page.left, page.bottom - page.top);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, page.left - room, page.top - room, wide + 2 * room, tall + 2 * room, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        if let Some(writing) = &mut self.writing {
            writing.place(super::note::words_box(room, room, wide, tall, self.scale));
        }
        self.redraw();
    }

    /// Whether it is drawn whole: the desktop behind it taken, where its skin shows it (chart paper's does not).
    fn ready(&self) -> bool {
        self.theme.skin == Skin::Paper || BEHIND.with(|held| held.borrow().1.is_some())
    }

    /// Shown, in front, taking the keys (once: see `shown`).
    fn show_now(&mut self) {
        if self.shown {
            return;
        }
        self.shown = true;
        crate::perf::mark(|| "beside shown".into());
        unsafe {
            let _ = KillTimer(Some(self.hwnd), SHOW);
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        // (Told again as it comes in front: the panels come over it again, the front having taken it over them.)
        crate::app().controller.page_front(Some(self.hwnd));
    }

    /// The point (`x`, `y`) physical px in its window, on its page (DIPs from the page's corner, past its shadow's room).
    fn on_page(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale - self.inset, y / self.scale - self.inset)
    }

    /// How large its page `page` (as `page_on_screen` finds it) is, in DIPs.
    fn page_size(&self, page: RECT) -> (f32, f32) {
        ((page.right - page.left) as f32 / self.scale, (page.bottom - page.top) as f32 / self.scale)
    }

    /// Where its page is on the screen (physical px): its window less its shadow's room.
    fn page_on_screen(&self) -> RECT {
        let mut place = RECT::default();
        let _ = unsafe { GetWindowRect(self.hwnd, &mut place) };
        let room = (self.inset * self.scale).round() as i32;
        RECT { left: place.left + room, top: place.top + room, right: place.right - room, bottom: place.bottom - room }
    }

    /// The page's own part of what was written in a note's words' box (see `note::Writing::written`): kept a moment after
    /// the last change, the caret shown.
    fn written(&mut self) {
        let Some(writing) = self.writing.as_mut() else { return };
        if writing.written() {
            unsafe {
                let _ = SetTimer(Some(self.hwnd), KEEP, super::note::KEEP_AFTER_MS, None);
            }
        }
        let asked = writing.take_menu_asked();
        self.caret_on = true;
        self.redraw();
        if let Some(at) = asked {
            self.writing_menu(at);
        }
    }

    /// How a menu opened on it looks: in its skin, light or dark and as frosted as it is drawn.
    fn menu_look(&self) -> super::menu::Look {
        let behind = BEHIND.with(|held| held.borrow().1.is_some());
        super::menu::Look::of(self.theme.skin, self.theme.dark).frosted(self.frost, behind)
    }

    /// The note's writing menu (see `note::writing_menu`), at `at` on the screen, where it was asked for.
    fn writing_menu(&self, at: POINT) {
        let (items, choices) = super::note::writing_menu(self.lang, self.writing.as_ref().map(super::note::Writing::spot).unwrap_or_default());
        if let Some(menu) = super::menu::Menu::open_at(items, self.menu_look(), at, false, false) {
            MENU.with(|held| *held.borrow_mut() = Some((menu, choices)));
        }
    }

    /// Glance going: what is written kept (see `keep`), or, where it cannot be, put aside (see `note::Writing::put_aside`).
    fn keep_as_glance_goes(&mut self) {
        if !self.keep() {
            if let Some(writing) = &self.writing {
                writing.put_aside();
            }
        }
    }

    /// The note written kept in its file (see `note::Writing::keep`), its name now told (see `writing_for`). Whether all
    /// that was written is kept now (a failure leaves it not kept: the page stays up, and says so).
    fn keep(&mut self) -> bool {
        let lang = self.lang;
        let Some(writing) = self.writing.as_mut() else { return true };
        let kept = writing.keep(lang);
        *WRITING.lock().unwrap() = writing.name.clone();
        kept
    }

    /// The note's menu, at its button.
    fn note_menu(&self, at: POINT) {
        let Some(writing) = &self.writing else { return };
        let (items, choices) = super::note::menu(self.lang, writing.colour, writing.name.is_some(), false);
        if let Some(menu) = super::menu::Menu::open_at(items, self.menu_look(), at, false, false) {
            MENU.with(|held| *held.borrow_mut() = Some((menu, choices)));
        }
    }

    /// The note written kept in its file on a thread of its own (see `note::Writing::keep_soon`), its name told as it is
    /// kept: looked at again shortly while that is under way, tried again a while after it failed.
    fn keep_soon(&mut self) {
        let lang = self.lang;
        let Some(writing) = self.writing.as_mut() else { return };
        let keeping = writing.keep_soon(lang);
        *WRITING.lock().unwrap() = writing.name.clone();
        let again = match keeping {
            super::note::Keeping::Kept => return,
            super::note::Keeping::Under => super::note::KEEP_LOOK_MS,
            super::note::Keeping::Failed => super::note::KEEP_AGAIN_MS,
        };
        let _ = unsafe { SetTimer(Some(self.hwnd), KEEP, again, None) };
    }

    /// The note on the page as its file is now, where that was changed outside (see `note::Writing::read_outside`).
    fn read_outside(&mut self) {
        if let Some(writing) = self.writing.as_mut() {
            writing.read_outside();
        }
    }

    /// Whether a press at `at` (on the screen) takes it by its head, to be moved: a note kept in a file (pulled off onto
    /// the desktop, see `carry_note`, a new note, in no file yet, stays where it is), in its head's row, on nothing of it
    /// pressed there.
    fn takes_by_head(&self, at: POINT) -> bool {
        if !self.writing.as_ref().is_some_and(|writing| writing.name.is_some()) {
            return false;
        }
        let mut client = at;
        let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut client) };
        let (x, y) = self.on_page(client.x as f32, client.y as f32);
        (0.0..HEAD).contains(&y) && !self.hits.iter().any(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h)
    }

    /// Where its corner is on the screen.
    fn corner(&self) -> POINT {
        let mut place = RECT::default();
        let _ = unsafe { GetWindowRect(self.hwnd, &mut place) };
        POINT { x: place.left, y: place.top }
    }

    /// Moved by its head as far as a drag goes and put down: the note stuck on the desktop where its page was put (as
    /// from its menu), its page gone as it shows there.
    fn carry_note(&mut self) {
        let Some(from) = self.moved_from.take() else { return };
        if self.writing.as_ref().is_none_or(|writing| writing.name.is_none()) {
            return;
        }
        let to = self.corner();
        let far = (24.0 * self.scale) as i32;
        if (to.x - from.x).abs() < far && (to.y - from.y).abs() < far {
            return;
        }
        // (Kept first: kept apart, as a conflict's copy, it is that copy stuck. Not kept, it stays, saying so.)
        if !self.keep() {
            self.redraw();
            return;
        }
        let Some(name) = self.writing.as_ref().and_then(|writing| writing.name.clone()) else { return };
        let place = self.page_on_screen();
        leave_for(&name);
        crate::app().controller.stick_note(name, POINT { x: place.left, y: place.top }, Some(self.page_size(place)));
    }

    /// `choice` made in the note's menu: done; whether the page is to go.
    fn note_chose(&mut self, choice: super::note::Choice) -> bool {
        use super::note::Choice;
        // (Its writing menu's: done to the words, seen to as what is typed is.)
        if let Choice::Write(how) = choice {
            if let Some(writing) = &self.writing {
                writing.write(how);
            }
            self.written();
            return false;
        }
        // (The notes' Markdown shown as written, or as it reads: the note written dressed again so.)
        if choice == Choice::Source {
            super::note::turn_source();
            self.written();
            self.redraw();
            return false;
        }
        let kept = self.keep();
        let Some(writing) = self.writing.as_mut() else { return false };
        let folder = crate::notes::folder_at(writing.folder.clone());
        match choice {
            Choice::Colour(colour) => {
                writing.recolour(&self.theme, colour);
                self.redraw();
                false
            }
            // Its card on the desktop where the page is, the page gone as it shows there.
            Choice::Stick => {
                let Some(name) = writing.name.clone() else { return false };
                let place = self.page_on_screen();
                leave_for(&name);
                crate::app().controller.stick_note(name, POINT { x: place.left + (24.0 * self.scale) as i32, y: place.top + (24.0 * self.scale) as i32 }, Some(self.page_size(place)));
                false
            }
            // (Offered only where the note is stuck on the desktop, on its card there: see `widget`. Its writing menu's
            // done above.)
            Choice::Unstick | Choice::Source | Choice::Write(_) => false,
            Choice::TakeAway => {
                // (What is written not kept, the page stays, saying so: taken away, the Recycle Bin had the note as it
                // was before, and what was written was gone with the page.)
                if !kept {
                    self.redraw();
                    return false;
                }
                let taken = writing.name.as_ref().is_some_and(|name| folder.take_away(name));
                if taken {
                    writing.unkept = false;
                    crate::notes::read_again();
                    // (Its card on the desktop goes with it.)
                    if let Some(name) = writing.name.clone() {
                        crate::app().controller.unstick_note(name);
                    }
                }
                taken
            }
        }
    }

    /// What it lists now of the kind chosen, as the search asks: the Start menu's programs in its order, each under
    /// its first letter (none till the menu has been read); the files come lately, the latest first; the folders they
    /// are in, the one a file came to latest first.
    fn rows(&self) -> Vec<PickRow> {
        let query = self.search.string();
        let searched = |name: &str| found::matches(&Found { name: name.to_string(), target: String::new(), initials: found::initials(name) }, &query);
        match self.kind {
            Kinds::Programs => found::all()
                .map(|all| {
                    all.iter()
                        .filter(|found| found::matches(found, &query))
                        .map(|found| PickRow {
                            name: found.name.clone(),
                            target: found.target.clone(),
                            about: String::new(),
                            when: String::new(),
                            letter: found.initials.first().map(|bits| if bits.trailing_zeros() < 26 { (b'A' + bits.trailing_zeros() as u8) as char } else { '#' }),
                            on_disk: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Kinds::Files | Kinds::Folders => {
                let mut came: Vec<crate::station::recent::Came> = crate::station::recent::shown().iter().cloned().collect();
                came.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
                let now = crate::station::now_ms();
                let mut seen = std::collections::HashSet::new();
                let mut rows = Vec::new();
                for came in came {
                    let path = if self.kind == Kinds::Folders { came.path.parent().map(std::path::Path::to_path_buf) } else { Some(came.path.clone()) };
                    let Some(path) = path.filter(|path| seen.insert(path.clone())) else { continue };
                    // (A file by its whole name: its kind is part of what tells it apart. Its path as it is kept, not
                    // looked at here: the page draws with what it lists.)
                    let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
                    if !searched(&name) {
                        continue;
                    }
                    let place = path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
                    rows.push(PickRow { name, target: path.display().to_string(), about: place, when: super::text::since(came.came_ms, now, self.lang), letter: None, on_disk: true });
                }
                rows
            }
        }
    }

    /// A press begun at (`x`, `y`) DIPs: what it is on noted; on a kept thing, it is held (to be dragged into another
    /// place).
    fn hold(&mut self, x: f32, y: f32) {
        let held = self.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4.clone());
        self.pressed = held.clone();
        if let Some(Hit::Kept(target)) = held {
            self.holding = Some((target, (x, y), false));
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.hwnd);
            }
        }
    }

    /// Where among what is kept a thing dragged at `x` goes: the gap nearest it.
    fn gap_at(&self, x: f32, count: usize) -> usize {
        (((x - PAD + self.strip_scroll) / STRIP_CELL).round().max(0.0) as usize).min(count)
    }

    /// A kept thing held let go at `x`: dragged, put in the gap it was dragged to (before the thing shown after that
    /// gap, or last), each found by what opens it as what is kept is changed (it may have changed since). Never the
    /// page's going.
    fn let_go(&mut self, x: f32) -> bool {
        let Some((moving, _, moved)) = self.holding.take() else { return false };
        self.pressed = None;
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        }
        if moved {
            let shown = crate::station::apps::shown();
            let before = shown.get(self.gap_at(x, shown.len())).map(|app| app.target.clone());
            crate::station::apps::change(move |apps| crate::station::apps::put_before(apps, &moving, before.as_deref()));
            unsafe {
                let _ = SetTimer(Some(self.hwnd), AGAIN, 120, None);
            }
        }
        self.redraw();
        false
    }

    /// A press let go at (`x`, `y`) DIPs on what it began on: done; whether the page is to go.
    fn press(&mut self, x: f32, y: f32) -> bool {
        let at = self.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4.clone());
        if at.is_none() || std::mem::take(&mut self.pressed) != at {
            return false;
        }
        self.pressed_on(at.unwrap(), x, y)
    }

    /// `paths` kept at hand after what is there (what opens what is kept already, however it is named, not twice).
    fn add_paths(&self, paths: Vec<std::path::PathBuf>) {
        let added: Vec<App> = paths.iter().filter_map(|path| App::of_path(path)).collect();
        // (Shortcuts read for what they open here, in this thread's apartment: see `apps::opens`.)
        added.iter().for_each(|app| {
            crate::station::apps::opens(&app.target);
        });
        if added.is_empty() {
            return;
        }
        crate::station::apps::change(move |apps| {
            for app in added {
                if !apps.iter().any(|kept| crate::station::apps::same(&kept.target, &app.target)) {
                    apps.push(app);
                }
            }
        });
        unsafe {
            let _ = SetTimer(Some(self.hwnd), AGAIN, 120, None);
        }
    }

    /// A press on `hit`, let go at (`x`, `y`) DIPs: done; whether the page is to go.
    fn pressed_on(&mut self, hit: Hit, x: f32, y: f32) -> bool {
        match hit {
            Hit::Close => return true,
            Hit::Search | Hit::Kept(_) => {}
            Hit::Kind(kind) => {
                self.kind = kind;
                self.search = Typing::most(SEARCH_MOST);
                self.scroll = 0.0;
            }
            Hit::Unkeep(target) => {
                crate::station::apps::change(move |apps| apps.retain(|kept| kept.target != target));
                unsafe {
                    let _ = SetTimer(Some(self.hwnd), AGAIN, 120, None);
                }
            }
            Hit::Choose => self.choosing = true,
            Hit::NoteMenu => {
                let mut at = POINT { x: ((x - 120.0) * self.scale) as i32, y: ((y + 14.0) * self.scale) as i32 };
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut at);
                }
                self.note_menu(at);
            }
            Hit::Pick(target, name) => {
                // (A file or folder by its name as it is kept: see `App::of_path`.)
                let app = match self.kind {
                    Kinds::Programs => App { target, name },
                    _ => App::of_path(std::path::Path::new(&target)).unwrap_or(App { target, name }),
                };
                crate::station::apps::change(move |apps| {
                    match apps.iter().position(|kept| crate::station::apps::same(&kept.target, &app.target)) {
                        Some(at) => {
                            apps.remove(at);
                        }
                        None => apps.push(app),
                    }
                });
                // (Drawn again once the settings have it.)
                unsafe {
                    let _ = SetTimer(Some(self.hwnd), AGAIN, 120, None);
                }
            }
        }
        self.redraw();
        false
    }

    fn key(&mut self, key: VIRTUAL_KEY) {
        if self.page != Page::Picker {
            return;
        }
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        match key {
            VK_LEFT => self.search.go(self.search.step(self.search.caret, false), false),
            VK_RIGHT => self.search.go(self.search.step(self.search.caret, true), false),
            VK_HOME => self.search.go(0, false),
            VK_END => self.search.go(self.search.text.len(), false),
            VK_BACK => self.search.erase(false),
            VK_DELETE => self.search.erase(true),
            // Enter: the first one listed added.
            VK_RETURN => {
                if let Some(first) = self.rows().into_iter().next() {
                    let app = if first.on_disk { App::of_path(std::path::Path::new(&first.target)) } else { Some(App { target: first.target, name: first.name }) };
                    if let Some(app) = app {
                        // (Kept already, as it is added: not twice, Enter pressed again before the first was kept.)
                        crate::station::apps::change(move |apps| {
                            if !apps.iter().any(|kept| crate::station::apps::same(&kept.target, &app.target)) {
                                apps.push(app);
                            }
                        });
                    }
                }
            }
            _ if ctrl && key.0 == u16::from(b'A') => {
                self.search.go(0, false);
                self.search.go(self.search.text.len(), true);
            }
            _ => return,
        }
        self.scroll = 0.0;
        self.caret_on = true;
        self.redraw();
    }

    fn char(&mut self, unit: u16) {
        if self.page != Page::Picker {
            return;
        }
        // (Control characters are keys, seen to in `key`.)
        if unit < 0x20 || unit == 0x7F {
            return;
        }
        self.search.put(&[unit]);
        self.scroll = 0.0;
        self.caret_on = true;
        self.redraw();
    }

    /// The input method's words and choices put at the caret.
    fn place_ime(&self) {
        let Some((x, y, h)) = self.caret else { return };
        let px = |dips: f32| (dips * self.scale).round() as i32;
        unsafe {
            let context = ImmGetContext(self.hwnd);
            if context.is_invalid() {
                return;
            }
            let (x, y) = (x + self.inset, y + self.inset);
            let _ = ImmSetCompositionWindow(context, &COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: px(x), y: px(y) }, ..Default::default() });
            let around = RECT { left: px(x), top: px(y), right: px(x) + 1, bottom: px(y + h) };
            let _ = ImmSetCandidateWindow(context, &CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_EXCLUDE, ptCurrentPos: POINT { x: around.left, y: around.bottom }, rcArea: around });
            let mut face = LOGFONTW { lfHeight: -px(self.theme.body.size), lfWeight: 400, lfCharSet: windows::Win32::Graphics::Gdi::DEFAULT_CHARSET, ..Default::default() };
            for (unit, to) in "Microsoft YaHei UI".encode_utf16().zip(face.lfFaceName.iter_mut()) {
                *to = unit;
            }
            let _ = ImmSetCompositionFontW(context, &face);
            let _ = ImmReleaseContext(self.hwnd, context);
        }
    }

    fn draw(&mut self) {
        let mut inside = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut inside);
        }
        let size = (inside.right.max(0) as u32, inside.bottom.max(0) as u32);
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        if self.surface.is_none() {
            if let Ok(gfx) = gfx::current() {
                self.gfx = gfx;
            }
            self.surface = Surface::new(&self.gfx, self.hwnd).ok();
        }
        let Some(mut surface) = self.surface.take() else { return };
        let gfx = self.gfx.clone();
        let inset = self.inset;
        let area = (size.0 as f32 / self.scale - 2.0 * inset, size.1 as f32 / self.scale - 2.0 * inset);
        self.look_behind();
        // (A mark sliding: drawn again a frame on, till it is where it is to be.)
        let sliding = self.marks.choose(std::time::Instant::now(), &[(0, self.kind)]);
        unsafe {
            if sliding {
                let _ = SetTimer(Some(self.hwnd), SLIDING, 15, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), SLIDING);
            }
        }
        let drawn = surface.draw(&gfx, size, self.scale, |frame| {
            let mut was = windows_numerics::Matrix3x2::default();
            unsafe {
                frame.dc.GetTransform(&mut was);
                frame.dc.SetTransform(&(windows_numerics::Matrix3x2::translation(inset, inset) * was));
            }
            self.paint(frame, area);
            unsafe { frame.dc.SetTransform(&was) };
        });
        if drawn.is_ok() {
            self.surface = Some(surface);
            if !self.shown && self.ready() {
                self.show_now();
            }
        } else if let Err(error) = &drawn {
            // (Not drawn, it keeps its surface, and draws again on it; the device gone, it makes one anew on the next.)
            if !gfx::failed("page beside", Some(error)) {
                self.surface = Some(surface);
            }
        }
        self.place_ime();
        // Pictures still being made: drawn again soon; else not.
        unsafe {
            if self.waiting {
                let _ = SetTimer(Some(self.hwnd), AGAIN, 250, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), AGAIN);
            }
        }
    }

    /// What is behind its page taken, as a widget takes what is behind it (see `ui::scene`): asked for as it opens,
    /// as it is moved, and again a while after (what is behind changes); how frosted it is drawn, as what is behind
    /// asks. Its scene is its thread's, kept from page to page (one made anew keeps some of the card's memory).
    fn look_behind(&mut self) {
        // (A skin that does not see the desktop: the scene kept for one that does let go of.)
        if self.theme.skin == Skin::Paper {
            BEHIND.with(|held| held.borrow_mut().0 = None);
            return;
        }
        let page = self.page_on_screen();
        BEHIND.with(|held| {
            let mut held = held.borrow_mut();
            let (scene, taken, asked) = &mut *held;
            if scene.is_none() {
                // (The page up as a frame comes, not the one it was made for: kept from page to page, it told the
                // first, gone, and the next was drawn with what is behind it only as it was drawn again anyway.)
                let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {
                    let page = PAGE.load(Ordering::Acquire);
                    if page != 0 {
                        let _ = unsafe { PostMessageW(Some(HWND(page as *mut _)), WM_TIMER, WPARAM(AGAIN), LPARAM(0)) };
                    }
                });
                *scene = super::scene::Scene::new(page, wake, &self.gfx);
            }
            let Some(scene) = scene.as_mut() else { return };
            let arranged = scene.arrange(page, self.hwnd);
            if arranged || taken.is_none() || asked.is_none_or(|at| at.elapsed() >= LOOK_AGAIN) {
                scene.want();
                *asked = Some(std::time::Instant::now());
            }
            if let Some(capture) = scene.ready() {
                let (mean, spread) = capture.luminance(page, self.scale);
                self.frost = super::skins::frost(mean, spread, self.theme.dark);
                *taken = Some((capture, None));
            }
        });
    }

    /// Its ground: the skin's sheet as the panel's is (its shadow, the desktop behind as the skin shows it, its tint
    /// and edge, its corners), as one piece.
    fn paint_ground(&self, frame: &Frame, (width, height): (f32, f32)) {
        let theme = &self.theme;
        let piece = [super::view::Rect { x: 0.0, y: 0.0, w: width, h: height }];
        BEHIND.with(|held| {
            let mut held = held.borrow_mut();
            let page = self.page_on_screen();
            // (Made once for the frame's device, at its screen's scale.)
            let backdrop = held.1.as_mut().and_then(|(capture, bitmap)| {
                if bitmap.is_none() {
                    *bitmap = capture.bitmap_into(&frame.dc, self.scale, None).ok();
                }
                let at = windows_numerics::Vector2 { X: (page.left - capture.rect.left) as f32 / self.scale, Y: (page.top - capture.rect.top) as f32 / self.scale };
                bitmap.as_ref().map(|bitmap| (bitmap, at))
            });
            let ground = super::skins::Ground {
                theme,
                edge: None,
                size: (width, height),
                lanes: &piece,
                bar: super::view::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
                backdrop: backdrop.map(|(bitmap, at)| (bitmap, at, self.scale)),
                frost: self.frost,
            };
            for part in super::skins::parts(theme.skin) {
                let _ = super::skins::draw(frame, &ground, *part);
            }
        });
    }

    /// The colour of what is chosen and of the one thing to press: Windows 11's accent, chart paper's ink.
    fn accent(&self) -> Color {
        match self.theme.skin {
            Skin::Paper => self.theme.text,
            _ => super::theme::accent(self.theme.dark),
        }
    }

    fn paint(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        match self.page {
            Page::Picker => self.paint_picker(frame, (width, height)),
            Page::Note(_) => self.paint_note(frame, (width, height)),
        }
    }

    /// A note's page: as `note::Writing::paint` draws it, its close and its menu in its head.
    fn paint_note(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        let theme = self.theme.clone();
        let lang = self.lang;
        let mut hits = Vec::new();
        self.paint_ground(frame, (width, height));
        let Some(writing) = &self.writing else {
            let pad = super::note::PAD;
            frame.text(lang.pick("没能打开编辑器", "The editor could not be opened"), theme.body, theme.text2, pad, pad, width - 2.0 * pad, Align::Start);
            self.hits = hits;
            return;
        };
        // (Its window's corner past the shadow's room, from the page's.)
        writing.paint(frame, &theme, lang, (0.0, 0.0, width, height), (-self.inset, -self.inset), self.scale, self.caret_on);
        for (nth, hit, icon) in [(0, Hit::Close, Icon::Close), (1, Hit::NoteMenu, Icon::Ellipsis)] {
            let place = super::note::button_at(nth, (0.0, 0.0, width));
            let lit = self.pointer.is_some_and(|(x, y)| x >= place.0 && x < place.0 + place.2 && y >= place.1 && y < place.1 + place.3);
            super::note::paint_button(frame, &theme, place, icon, lit);
            hits.push((place.0, place.1, place.2, place.3, hit));
        }
        self.hits = hits;
    }

    /// Quick launch's adding: its head (what it is, how much is kept, its close); what is kept, a picture each, in its
    /// order (dragged into another, taken off by the cross over the one under the pointer); the kinds of what may be
    /// added, side by side; the search; what is listed of the kind chosen, each added or taken off at a press; at its
    /// foot, the button that chooses on the PC. Something dragged over it to be added: the whole page says so.
    fn paint_picker(&mut self, frame: &Frame, (width, height): (f32, f32)) {
        let theme = self.theme.clone();
        let lang = self.lang;
        let mut hits = Vec::new();
        let pointer = self.pointer;
        let lit = |at: (f32, f32, f32, f32)| pointer.is_some_and(|(x, y)| x >= at.0 && x < at.0 + at.2 && y >= at.1 && y < at.1 + at.3);
        let round = |tall: f32| if theme.skin == Skin::Glass { tall / 2.0 } else { theme.control_radius.max(4.0).min(tall / 2.0) };
        self.paint_ground(frame, (width, height));
        // Its head.
        let kept = crate::station::apps::shown();
        let inner = width - 2.0 * PAD;
        let title = lang.pick("添加到快捷 App", "Add to quick launch");
        let title_w = frame.measure(title, theme.title);
        frame.text(title, theme.title, theme.text, PAD, 16.0, inner, Align::Start);
        let said = match lang {
            super::text::Lang::Zh => format!("已添加 {} 项", kept.len()),
            super::text::Lang::En => format!("{} added", kept.len()),
        };
        frame.text(&said, theme.small, theme.text3, PAD + title_w + 10.0, 19.0, inner - title_w - 40.0, Align::Start);
        let close = (width - PAD - 28.0, 12.0, 28.0, 28.0);
        if lit(close) {
            frame.fill_rounded(theme.hover, close.0, close.1, close.2, close.3, round(28.0));
        }
        frame.icon(Icon::Close, Point { x: close.0 + 14.0, y: close.1 + 14.0 }, 16.0, theme.text2);
        hits.push((close.0, close.1, close.2, close.3, Hit::Close));
        // What is kept, along a strip scrolled by the wheel where it is more than fits.
        let strip = (PAD - 4.0, STRIP_TOP, inner + 8.0, STRIP_CELL);
        self.strip_most = (kept.len() as f32 * STRIP_CELL - inner).max(0.0);
        self.strip_scroll = self.strip_scroll.clamp(0.0, self.strip_most);
        frame.clip(strip.0, strip.1, strip.2, strip.3);
        if kept.is_empty() {
            frame.text(lang.pick("尚未添加任何项目", "Nothing added yet"), theme.small, theme.text3, PAD, STRIP_TOP + (STRIP_CELL - 16.0) / 2.0, inner, Align::Start);
        }
        let held = self.holding.clone().filter(|(_, _, moved)| *moved);
        let mut placed = Vec::new();
        for (at, app) in kept.iter().enumerate() {
            let x = PAD + at as f32 * STRIP_CELL - self.strip_scroll;
            if x + STRIP_CELL < strip.0 || x > strip.0 + strip.2 {
                continue;
            }
            let cell = (x, STRIP_TOP, STRIP_CELL, STRIP_CELL);
            let over = lit(cell) && held.is_none();
            if over || held.as_ref().is_some_and(|(moving, ..)| *moving == app.target) {
                super::view::app_plate(frame, &theme, (cell.0, cell.1, cell.2), 6.0);
            }
            super::view::paint_app(frame, &theme, &app.name, &app.target, x + 6.0, STRIP_TOP + 6.0, STRIP_CELL - 12.0);
            // Its cross, over the one under the pointer.
            if over {
                let centre = Point { x: x + STRIP_CELL - 8.0, y: STRIP_TOP + 8.0 };
                frame.fill_circle(theme.text2, centre, 7.5);
                frame.icon(Icon::Close, centre, 9.0, self.ground_colour());
                placed.push((centre.x - 8.0, centre.y - 8.0, 16.0, 16.0, Hit::Unkeep(app.target.clone())));
            }
            placed.push((cell.0, cell.1, cell.2, cell.3, Hit::Kept(app.target.clone())));
        }
        // One dragged: the gap it goes to.
        if let (Some(_), Some((px, _))) = (held.as_ref(), pointer) {
            let gap = self.gap_at(px, kept.len());
            frame.fill_rounded(self.accent(), PAD + gap as f32 * STRIP_CELL - self.strip_scroll - 1.0, STRIP_TOP + 6.0, 2.0, STRIP_CELL - 12.0, 1.0);
        }
        frame.unclip();
        hits.extend(placed);
        // The kinds, side by side as the station's pages are, the one listed marked as the skin marks the station's
        // chosen page: glass's on a pill of its glass (see `view::glass_choice`), Windows 11's a bar of its accent under
        // it, chart paper's underlined in ink; the others lit under the pointer.
        let kinds = [(Kinds::Programs, Icon::LayoutGrid, lang.pick("程序", "Programs")), (Kinds::Files, Icon::File, lang.pick("文件", "Files")), (Kinds::Folders, Icon::Folder, lang.pick("文件夹", "Folders"))];
        let glass = theme.skin == Skin::Glass;
        let cell_w = inner / kinds.len() as f32;
        let cell_at = |at: usize| (PAD + at as f32 * cell_w, KINDS_TOP + 3.0, cell_w, KINDS_H - 6.0);
        {
            let at = |kind: Kinds| kinds.iter().position(|(one, _, _)| *one == kind);
            let slid = super::view::slid_at(self.marks.slid(0, self.kind), at);
            let chosen = at(self.kind).unwrap_or(0);
            if glass {
                let boxes: Vec<(f32, f32)> = (0..kinds.len()).map(|at| (cell_at(at).0, cell_w)).collect();
                let hovered = (0..kinds.len()).find(|at| lit(cell_at(*at)));
                super::view::glass_choice(frame, &theme, (KINDS_TOP + 3.0, KINDS_H - 6.0), &boxes, chosen, slid, hovered);
            } else {
                // (Chart paper's line under its icon and name, Windows 11's bar under its middle.)
                let boxes: Vec<(f32, f32)> = kinds.iter().enumerate().map(|(at, (_, _, name))| {
                    let content = 15.0 + 6.0 + frame.measure(name, theme.body);
                    (cell_at(at).0 + (cell_w - content) / 2.0, content)
                }).collect();
                let (_, y, _, h) = cell_at(0);
                let y = if theme.skin == Skin::Paper { y + h - 2.0 } else { y + h - 3.0 };
                super::view::choice_mark(frame, &theme, &boxes, chosen, slid, y, 16.0, 0.0);
            }
        }
        for (at, (kind, icon, name)) in kinds.into_iter().enumerate() {
            let cell = cell_at(at);
            let chosen = kind == self.kind;
            let hovered = lit(cell);
            let name_w = frame.measure(name, theme.body);
            let content = 15.0 + 6.0 + name_w;
            let start = cell.0 + (cell.2 - content) / 2.0;
            match theme.skin {
                Skin::Fluent if hovered && !chosen => frame.fill_rounded(theme.hover, cell.0, cell.1, cell.2, cell.3, round(cell.3)),
                _ => {}
            }
            let ink = match (chosen || hovered, glass) {
                (true, _) => theme.text,
                (false, true) => theme.text2,
                (false, false) => theme.text3,
            };
            frame.icon(icon, Point { x: start + 7.5, y: cell.1 + cell.3 / 2.0 }, 15.0, ink);
            frame.text(name, theme.body, ink, start + 21.0, cell.1 + (cell.3 - theme.body.size * 1.35) / 2.0, name_w + 2.0, Align::Start);
            hits.push((cell.0, cell.1, cell.2, cell.3, Hit::Kind(kind)));
        }
        // The search, its caret blinking.
        let field = (PAD, SEARCH_TOP, inner, SEARCH);
        // (Glass's a shallow well of its glass, lit along its foot: a grey bar, it was the heaviest thing on the page;
        // Windows 11's its field, the accent along its foot as the words go in; chart paper's a faint wash of ink.)
        match theme.skin {
            Skin::Glass => {
                frame.fill_rounded(if theme.dark { Color::hex(0xFFFFFF, 0.08) } else { Color::hex(0, 0.05) }, field.0, field.1, field.2, field.3, round(SEARCH));
                frame.stroke_rounded(super::canvas::Fill::Down { top: field.1, from: Color::hex(0xFFFFFF, 0.0), bottom: field.1 + field.3, to: Color::hex(0xFFFFFF, if theme.dark { 0.18 } else { 0.6 }) }, field.0, field.1, field.2, field.3, round(SEARCH), 1.0);
            }
            Skin::Fluent => {
                frame.fill_rounded(theme.track, field.0, field.1, field.2, field.3, round(SEARCH));
                frame.clip(field.0, field.1 + field.3 - 2.0, field.2, 2.0);
                frame.fill_rounded(super::theme::accent(theme.dark), field.0, field.1, field.2, field.3, round(SEARCH));
                frame.unclip();
            }
            Skin::Paper => frame.fill_rounded(theme.track.alpha(theme.track.a * 0.6), field.0, field.1, field.2, field.3, round(SEARCH)),
        }
        frame.icon(Icon::Search, Point { x: field.0 + 16.0, y: field.1 + SEARCH / 2.0 }, 14.0, theme.text3);
        let text_x = field.0 + 32.0;
        let text_y = field.1 + (SEARCH - theme.body.size * 1.35) / 2.0;
        let words = self.search.string();
        if words.is_empty() {
            let asked = match self.kind {
                Kinds::Programs => lang.pick("搜索程序，可以用拼音首字母", "Search programs by name"),
                Kinds::Files => lang.pick("搜索最近用过的文件", "Search the files used lately"),
                Kinds::Folders => lang.pick("搜索最近用过的文件夹", "Search the folders used lately"),
            };
            frame.text(asked, theme.body, theme.text3, text_x, text_y, field.2 - 40.0, Align::Start);
        } else {
            frame.text(&words, theme.body, theme.text, text_x, text_y, field.2 - 40.0, Align::Start);
        }
        let before = String::from_utf16_lossy(&self.search.text[..self.search.caret]);
        let caret_x = text_x + frame.measure(&before, theme.body);
        let caret_h = theme.body.size * 1.35;
        self.caret = Some((caret_x, text_y, caret_h));
        if self.caret_on {
            frame.fill(theme.text, caret_x, text_y + 1.0, 1.2, caret_h - 2.0);
        }
        hits.push((field.0, field.1, field.2, field.3, Hit::Search));
        // The list: programs under each first letter (none while searching); files and folders used lately, each with
        // where it is and when it came; each with what a press does (added; kept already, taken off).
        let top = LIST_TOP;
        let bottom = height - PICK_FOOT;
        let rows = self.rows();
        let searching = !words.trim().is_empty();
        let tall = if self.kind == Kinds::Programs { ROW } else { ROW_TWO };
        let mut laid: Vec<(f32, Option<String>, Option<&PickRow>)> = Vec::new();
        let mut y = 0.0;
        let mut letter = None;
        if self.kind != Kinds::Programs && !rows.is_empty() {
            laid.push((y, Some(lang.pick("最近用过", "Used lately").to_string()), None));
            y += LETTER;
        }
        for row in &rows {
            if self.kind == Kinds::Programs && !searching && row.letter != letter {
                letter = row.letter;
                laid.push((y, row.letter.map(String::from), None));
                y += LETTER;
            }
            laid.push((y, None, Some(row)));
            y += tall;
        }
        self.scroll_most = (y - (bottom - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, self.scroll_most);
        frame.clip(0.0, top, width, bottom - top);
        let mut waiting = false;
        let none = match self.kind {
            Kinds::Programs if found::all().is_none() => Some(lang.pick("正在读取开始菜单中的程序…", "Reading the Start menu's programs…")),
            Kinds::Programs => lang.pick("没有找到匹配的程序", "No program found by that").into(),
            _ if !crate::station::recent::on() => Some(lang.pick("最近文件未开启，可以从电脑中选择", "Recent files is off: choose on the PC")),
            Kinds::Files => Some(lang.pick("最近用过的文件中没有找到", "None of the files used lately")),
            Kinds::Folders => Some(lang.pick("最近用过的文件夹中没有找到", "None of the folders used lately")),
        };
        if rows.is_empty() {
            if let Some(none) = none {
                frame.text(none, theme.small, theme.text3, PAD, top + 8.0, inner, Align::Start);
            }
        }
        for (at, heading, row) in laid {
            let ry = top + at - self.scroll;
            if ry + tall < top || ry > bottom {
                continue;
            }
            if let Some(heading) = heading {
                frame.text(&heading, theme.small, theme.text3, PAD + ROW_IN, ry + (LETTER - 16.0) / 2.0 + 2.0, inner, Align::Start);
                continue;
            }
            let Some(row) = row else { continue };
            // (As wide as the search over it, what is in it a little in from its edges; its corners concentric with
            // its picture's, as round as that and the room round it.)
            let place = (PAD, ry, inner, tall);
            let hovered = lit(place) && pointer.is_some_and(|(_, py)| py >= top && py < bottom);
            if hovered {
                frame.fill_rounded(theme.hover, place.0, place.1 + 1.0, place.2, place.3 - 2.0, round(tall).min(5.0 + ROW_IN));
            }
            let face = 28.0;
            let face_x = PAD + ROW_IN;
            if row.on_disk {
                super::view::paint_app(frame, &theme, &row.name, &row.target, face_x, ry + (tall - face) / 2.0, face);
            } else {
                match crate::station::thumbs::icon(std::path::Path::new(&row.target), super::view::APP_PIXELS) {
                    Some(picture) => frame.picture(&picture, face_x, ry + (tall - face) / 2.0, face, face, 5.0, 1.0),
                    // (None yet, or none to be had: a program's mark in its place, a grey block read as still loading.)
                    None => {
                        waiting = true;
                        frame.icon(Icon::LayoutGrid, Point { x: face_x + face / 2.0, y: ry + tall / 2.0 }, 20.0, theme.text3);
                    }
                }
            }
            // What a press does, at its end as the panel's buttons are drawn: a plus, on a plate while the pointer is on
            // the row; kept already, a tick in the accent.
            let is_kept = kept.iter().any(|app| crate::station::apps::same(&app.target, &row.target));
            let mark = (PAD + inner - ROW_IN - 28.0, ry + (tall - 28.0) / 2.0, 28.0, 28.0);
            let words_x = face_x + 38.0;
            let words_w = mark.0 - 8.0 - words_x;
            let ink = if is_kept { theme.text2 } else { theme.text };
            if row.about.is_empty() {
                frame.text(&row.name, theme.body, ink, words_x, ry + (tall - theme.body.size * 1.35) / 2.0, words_w, Align::Start);
            } else {
                let name_top = ry + (tall - theme.body.size * 1.35 - 15.0) / 2.0;
                frame.text(&row.name, theme.body, ink, words_x, name_top, words_w, Align::Start);
                let when = format!(" · {}", row.when);
                let when_w = frame.measure(&when, theme.small);
                let about_y = name_top + theme.body.size * 1.35;
                let place = super::view::fitted(frame, &row.about, theme.small, (words_w - when_w).max(0.0)).into_owned();
                let place_w = frame.measure(&place, theme.small);
                frame.text(&place, theme.small, theme.text3, words_x, about_y, words_w - when_w, Align::Start);
                frame.text(&when, theme.small, theme.text3, words_x + place_w, about_y, when_w + 1.0, Align::Start);
            }
            let centre = Point { x: mark.0 + mark.2 / 2.0, y: mark.1 + mark.3 / 2.0 };
            if is_kept {
                frame.icon(Icon::Check, centre, 15.0, self.accent());
            } else {
                // (Its plate only with the pointer on it: on the row's, a plate in a plate.)
                if lit(mark) {
                    frame.fill_rounded(theme.hover, mark.0, mark.1, mark.2, mark.3, round(28.0));
                }
                frame.icon(Icon::Add, centre, 15.0, if hovered { theme.text } else { theme.text2 });
            }
            hits.push((place.0, place.1.max(top), place.2, (place.1 + place.3).min(bottom) - place.1.max(top), Hit::Pick(row.target.clone(), row.name.clone())));
        }
        frame.unclip();
        // A thin thumb where in the list the view is.
        if self.scroll_most > 0.0 {
            let tall = bottom - top;
            let thumb = (tall * tall / (tall + self.scroll_most)).max(16.0);
            frame.fill_rounded(theme.text3.alpha(0.6), width - 6.0, top + (tall - thumb) * self.scroll / self.scroll_most, 2.0, thumb, 1.0);
        }
        // Its foot: what is not listed, chosen on the PC.
        frame.fill(theme.rule, 0.0, bottom, width, 1.0);
        let button = (PAD, bottom + 10.0, inner, 32.0);
        // (Glass's a pill of its glass, as what is chosen is; the others' their plate, ruled round.)
        if theme.skin == Skin::Glass {
            super::view::glass_pill(frame, &theme, button, round(32.0), lit(button));
        } else {
            // (Chart paper's its paper ruled round in ink, a wash of it under the pointer: a filled bar, it was the search
            // field over again.)
            if theme.skin != Skin::Paper {
                frame.fill_rounded(theme.track, button.0, button.1, button.2, button.3, round(32.0));
            }
            if lit(button) {
                frame.fill_rounded(theme.hover, button.0, button.1, button.2, button.3, round(32.0));
            }
            let edge = if theme.skin == Skin::Paper { theme.text.alpha(0.25) } else { theme.rule };
            frame.stroke_rounded(super::canvas::Fill::Solid(edge), button.0, button.1, button.2, button.3, round(32.0), 1.0);
        }
        let (label, hint, icon) = match self.kind {
            Kinds::Programs => (lang.pick("列表中没有？从电脑中选择程序", "Not listed? Choose a program on the PC"), lang.pick("可以选择 exe 或快捷方式", "A program or a shortcut to one"), Icon::File),
            Kinds::Files => (lang.pick("从电脑中选择文件", "Choose files on the PC"), lang.pick("也可以把文件拖到此页面上", "Or drag files onto this page"), Icon::File),
            Kinds::Folders => (lang.pick("从电脑中选择文件夹", "Choose folders on the PC"), lang.pick("也可以把文件夹拖到此页面上", "Or drag folders onto this page"), Icon::Folder),
        };
        let label_w = frame.measure(label, theme.body);
        let start = button.0 + (button.2 - 15.0 - 8.0 - label_w) / 2.0;
        frame.icon(icon, Point { x: start + 7.5, y: button.1 + button.3 / 2.0 }, 15.0, theme.text2);
        frame.text(label, theme.body, theme.text, start + 23.0, button.1 + (button.3 - theme.body.size * 1.35) / 2.0, label_w + 2.0, Align::Start);
        hits.push((button.0, button.1, button.2, button.3, Hit::Choose));
        let hint_w = frame.measure(hint, theme.small);
        frame.text(hint, theme.small, theme.text3, (width - hint_w) / 2.0, bottom + 48.0, hint_w + 2.0, Align::Start);
        // Something dragged over it to be added: the whole page says so.
        if self.dropping {
            self.paint_ground(frame, (width, height));
            let mark = self.accent();
            frame.fill(mark.alpha(0.06), 0.0, 0.0, width, height);
            let (x, y, w, h) = (8.0, 8.0, width - 16.0, height - 16.0);
            frame.stroke_dashed(&super::view::rounded_outline(x, y, w, h, round(32.0).max(8.0)), mark, 1.5);
            let middle = height / 2.0;
            frame.icon(Icon::ArrowDown, Point { x: width / 2.0, y: middle - 26.0 }, 28.0, mark);
            let bold = super::canvas::Font { weight: 650.0, ..theme.body };
            let says = lang.pick("松开即可添加到快捷 App", "Let go to add it to quick launch");
            let says_w = frame.measure(says, bold);
            frame.text(says, bold, mark, (width - says_w) / 2.0, middle, says_w + 2.0, Align::Start);
            let more = lang.pick("可以添加程序、文件和文件夹", "Programs, files and folders");
            let more_w = frame.measure(more, theme.small);
            frame.text(more, theme.small, theme.text2, (width - more_w) / 2.0, middle + 24.0, more_w + 2.0, Align::Start);
        }
        self.waiting = waiting;
        self.hits = hits;
    }

    /// The colour its ground comes to, for what is drawn on a mark over it (a kept thing's cross).
    fn ground_colour(&self) -> Color {
        match (self.theme.skin, self.theme.dark) {
            (Skin::Paper, _) => self.theme.paper,
            (_, true) => Color::hex(0x2C2C2C, 1.0),
            (_, false) => Color::hex(0xF9F9F9, 1.0),
        }
    }
}

/// What is chosen on the PC, in Windows' own chooser over `hwnd` (the page), as the kind listed (`kind`) asks:
/// programs (or shortcuts to them), any files, or folders, several at once; none where it is closed.
pub(crate) fn choose(hwnd: HWND, kind: Kinds, lang: super::text::Lang) -> Vec<std::path::PathBuf> {
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
        use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
        use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_NODEREFERENCELINKS, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};
        unsafe {
            let Ok(dialog) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
            let Ok(options) = dialog.GetOptions() else { return Vec::new() };
            // (A shortcut chosen is kept as the shortcut, as one dropped is.)
            let mut options = options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_NODEREFERENCELINKS;
            let title = match kind {
                Kinds::Programs => lang.pick("选择程序", "Choose programs"),
                Kinds::Files => lang.pick("选择文件", "Choose files"),
                Kinds::Folders => lang.pick("选择文件夹", "Choose folders"),
            };
            if kind == Kinds::Folders {
                options |= FOS_PICKFOLDERS;
            }
            let _ = dialog.SetOptions(options);
            let _ = dialog.SetTitle(&HSTRING::from(title));
            if kind == Kinds::Programs {
                let (programs, all) = (HSTRING::from(lang.pick("程序", "Programs")), HSTRING::from(lang.pick("所有文件", "All files")));
                let (programs_spec, all_spec) = (HSTRING::from("*.exe;*.lnk;*.appref-ms;*.bat;*.cmd"), HSTRING::from("*.*"));
                let kinds = [
                    COMDLG_FILTERSPEC { pszName: windows::core::PCWSTR(programs.as_ptr()), pszSpec: windows::core::PCWSTR(programs_spec.as_ptr()) },
                    COMDLG_FILTERSPEC { pszName: windows::core::PCWSTR(all.as_ptr()), pszSpec: windows::core::PCWSTR(all_spec.as_ptr()) },
                ];
                let _ = dialog.SetFileTypes(&kinds);
            }
            if dialog.Show(Some(hwnd)).is_err() {
                return Vec::new();
            }
            let Ok(chosen) = dialog.GetResults() else { return Vec::new() };
            let count = chosen.GetCount().unwrap_or(0);
            (0..count)
                .filter_map(|at| {
                    let item = chosen.GetItemAt(at).ok()?;
                    let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                    let path = name.to_string().ok().map(std::path::PathBuf::from);
                    CoTaskMemFree(Some(name.0 as *const _));
                    path
                })
                .collect()
        }
    }

#[cfg(all(test, feature = "studio"))]
mod pictures {
    use super::*;

    /// Quick launch's choosing of programs in each skin, light and dark, as it opens and searched by pinyin
    /// (`cargo test --features studio --lib -- --ignored beside_picker` writes `target/beside/<skin>-<dark>-<search>.png`).
    #[test]
    #[ignore = "writes pictures to look at"]
    fn beside_picker() {
        let _pretending = crate::pretending();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        crate::station::thumbs::start(|| {});
        let found = |name: &str, target: &str| Found { name: name.into(), target: target.into(), initials: found::initials(name) };
        let mut listed = vec![
            found("微信", r"C:\Windows\System32\cmd.exe"),
            found("网易云音乐", r"C:\Windows\System32
otepad.exe"),
            found("Visual Studio Code", r"C:\Windows\explorer.exe"),
            found("计算器", r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"),
            found("Steam", r"C:\Windows\System32\mspaint.exe"),
            found("文件资源管理器", r"C:\Windows\explorer.exe"),
            found("7-Zip", r"C:\Windows\System32\charmap.exe"),
        ];
        listed.sort_by(|a, b| (a.initials.first().map(|bits| bits.trailing_zeros()), &a.name).cmp(&(b.initials.first().map(|bits| bits.trailing_zeros()), &b.name)));
        found::pretend(listed);
        let file = |name: &str| std::path::PathBuf::from(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).components().collect::<std::path::PathBuf>();
        crate::station::apps::pretend(vec![
            App { target: r"C:\Windows\System32\cmd.exe".into(), name: "微信".into() },
            App { target: r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into(), name: "计算器".into() },
            App { target: file("docs").display().to_string(), name: "docs".into() },
            App { target: file("Cargo.toml").display().to_string(), name: "Cargo".into() },
            App { target: file("gone.pdf").display().to_string(), name: "gone".into() },
        ]);
        {
            use crate::station::recent::{Came, Key, Kind};
            let now = crate::station::now_ms();
            let came = |id: u128, name: &str, ago_min: u64| Came { key: Key { drive: 'D', file: id }, path: file(name), came_ms: now - ago_min * 60_000, bytes: 1000, kind: Kind::Document, downloaded: false, site: None };
            crate::station::recent::pretend(vec![came(1, "Cargo.toml", 3), came(2, "README.md", 30), came(3, "docs/station-and-notes.md", 90), came(4, "website/apple-touch-icon.png", 600), came(5, "scripts/release.ps1", 2000)]);
        }
        let gfx = gfx::current().unwrap();
        let out = std::path::PathBuf::from(format!("{}/target/beside", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        let (w, h) = (WIDTH, 560.0);
        for pass in 0..2 {
            for (skin, name) in [(Skin::Glass, "glass"), (Skin::Fluent, "fluent"), (Skin::Paper, "paper")] {
                for dark in [false, true] {
                    for (kind, search, dropping, label) in [(Kinds::Programs, "", false, "programs"), (Kinds::Programs, "wy", false, "wy"), (Kinds::Files, "", false, "files"), (Kinds::Folders, "", false, "folders"), (Kinds::Files, "", true, "drop")] {
                        let mut beside = Beside {
                            hwnd: HWND::default(),
                            gfx: gfx.clone(),
                            surface: None,
                            _showing: super::super::life::Showing::of(&gfx),
                            scale: 1.5,
                            theme: Rc::new(Theme::new(skin, dark)),
                            lang: super::super::text::Lang::Zh,
                            page: Page::Picker,
                            inset: 0.0,
                            frost: 0.0,
                            pressed: None,
                            choosing: false,
                            kind,
                            dropping,
                            holding: None,
                            strip_scroll: 0.0,
                            strip_most: 0.0,
                            search: Typing::most(SEARCH_MOST),
                            scroll: 0.0,
                            scroll_most: 0.0,
                            pointer: Some((100.0, LIST_TOP + 30.0 + 26.0)),
                            hits: Vec::new(),
                            caret: None,
                            caret_on: true,
                            waiting: false,
                            writing: None,
                            opening: None,
                            moved_from: None,
                            shown: false,
                            marks: super::super::thumb::Dots::new(),
                        };
                        beside.search.put(&search.encode_utf16().collect::<Vec<_>>());
                        let pixels = gfx.draw_offscreen(((w * 1.5) as u32, (h * 1.5) as u32), 1.5, |frame| beside.paint(frame, (w, h))).unwrap();
                        if pass == 1 {
                            let file = format!("{name}-{}-{label}.png", if dark { "dark" } else { "light" });
                            crate::studio::save_png(&out.join(file), (w * 1.5) as u32, (h * 1.5) as u32, &pixels).unwrap();
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        // (Its device let go of before the thread ends: else it hung there.)
        drop(gfx);
        gfx::let_go();
    }
}

#[cfg(test)]
mod tests {
    use super::goes_now;

    /// A page beside the panel on a narrow screen at a large scale (a portrait screen 1080 px wide at 350%, the page
    /// 1260 px wide at it): within the screen, on every side, not ending Glance.
    #[test]
    fn a_page_is_never_wider_than_its_screen() {
        use super::super::theme::Skin;
        use super::{placed_in, Opening, Page, Side};
        use windows::Win32::Foundation::RECT;
        let room = RECT { left: 0, top: 0, right: 1080, bottom: 1920 };
        let by = RECT { left: 700, top: 300, right: 1080, bottom: 900 };
        for side in [Side::Left, Side::Right, Side::Below] {
            let opening = Opening { page: Page::Note(Some("a.md".into())), by, side, skin: Skin::Paper, dark: false, folder: None, owner: 0 };
            let page = placed_in(&opening, room, 3.5);
            assert!(page.left >= room.left && page.right <= room.right && page.top >= room.top && page.bottom <= room.bottom, "{side:?}: {:?}", (page.left, page.top, page.right, page.bottom));
        }
    }

    /// A note's page left for another note pressed on the panel: its going, asked for as it was left, comes after the
    /// other's page is up, and leaves that one up (it went: the page flashed, and the note pressed never showed).
    #[test]
    fn a_page_left_goes_alone() {
        let (left, opened) = (0x1010, 0x2020);
        assert!(!goes_now(left, opened));
        assert!(goes_now(opened, opened));
        assert!(!goes_now(left, 0));
    }
}

#[cfg(all(test, feature = "studio"))]
mod timed {
    use super::*;

    /// How long a note's page takes to come up, from asked to shown, in each skin, the first time on its thread and
    /// after, and that it does at once: its parts' marks written where GLANCE_PERF names (a note in a folder
    /// of the test's own; the page shown on this PC's screen a moment each time). (On glass, the first page to want
    /// the desktop, before any panel or widget did, hung as it asked whether it may be captured without a border; each
    /// after the first waited for being drawn again anyway, what is behind it told to the first page, gone.)
    /// `GLANCE_PERF=<file> cargo test --features studio --lib -- --ignored --exact ui::beside::timed::a_note_s_page_comes_up --nocapture`.
    #[test]
    #[ignore = "puts a note's page on this PC's screen"]
    fn a_note_s_page_comes_up() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        crate::pictured(crate::metrics::Sampler::new().info, Vec::new());
        let folder = std::env::temp_dir().join("glance-note-page-timed");
        let _ = std::fs::create_dir_all(&folder);
        std::fs::write(folder.join("a.md"), "# A note\nIts first line\nIts second\n- [ ] something to do\n").unwrap();
        let wait = |done: &dyn Fn() -> bool| {
            let began = std::time::Instant::now();
            while !done() && began.elapsed() < std::time::Duration::from_secs(5) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            began.elapsed()
        };
        let shown = || {
            let page = PAGE.load(Ordering::Acquire);
            page != 0 && unsafe { IsWindowVisible(HWND(page as *mut _)).as_bool() }
        };
        let mut failed = false;
        for skin in [Skin::Paper, Skin::Glass] {
            let mut took = Vec::new();
            for _ in 0..5 {
                crate::perf::mark(|| format!("round {skin:?}"));
                let began = std::time::Instant::now();
                open(Opening { page: Page::Note(Some("a.md".into())), by: RECT { left: 200, top: 200, right: 400, bottom: 320 }, side: Side::Right, skin, dark: false, folder: Some(folder.clone()), owner: 0 });
                wait(&shown);
                took.push(began.elapsed().as_secs_f64() * 1000.0);
                close();
                wait(&|| !is_up());
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            println!("{skin:?}: {}", took.iter().map(|ms| format!("{ms:.0} ms")).collect::<Vec<_>>().join(", "));
            // The first within a second (its thread's device made first); the others as soon as what is behind them is
            // taken, well before the page would be drawn again anyway, or would give up waiting for it (see
            // `SHOW_WITHIN_MS`).
            failed |= took[0] > 1000.0 || took[1..].iter().any(|ms| *ms > 200.0);
        }
        let _ = std::fs::remove_dir_all(&folder);
        assert!(!failed, "a page was slow to be shown");
    }
}
