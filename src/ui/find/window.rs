//! The search window: a sheet centred on the screen the pointer is on, a little above its middle, called up by the
//! shortcut and gone as anything else comes to the front. What is typed is looked for as it is typed (on a thread of
//! its own: a drive's names are many), and shown under the kinds to choose from; the keyboard and the mouse open what
//! is chosen, show it in Explorer, copy it, or drag it out. Drawn in the skin the panel is (its glass over the screen
//! frosted, Windows 11's acrylic, chart paper), light or dark as the panel is.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
    VK_LWIN, VK_MENU, VK_RWIN, VK_TAB, VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCaretBlinkTime, GetCursorPos, GetForegroundWindow, GetMessageW, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, PeekMessageW, PostMessageW,
    RegisterClassW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, ASFW_ANY, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_IBEAM, MSG, PM_REMOVE, SWP_NOACTIVATE, SW_HIDE, SW_SHOW,
    SW_SHOWNORMAL, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_DPICHANGED, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SYSCHAR, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::pages::{Opens, PAGES};
use super::results::{self, Group, Kind, Line, More, Row, Shown, Sources, View, What};
use crate::station::recent::Kind as FileKind;
use crate::station::run::{self, Ask, FilesAct};
use crate::ui::canvas::{Align, Canvas, Color, Family, Font, Point};
use crate::ui::gfx::{self, Frame, Gfx, Surface};
use crate::ui::icons::Icon;
use crate::ui::look;
use crate::ui::menu::{self, Colours, Item, Look, Mark, Menu};
use crate::ui::text::Lang;
use crate::ui::theme::{self, Skin, Theme};
use crate::ui::typing::{at_point, caret_at, Typing};
use crate::ui::view::Rect;

/// Its window is shown, or hidden (see `toggle`); what was asked for was found (see `work`); the Start menu was read
/// (its programs looked for again); it was left (another window came to the front, or the mouse went to a menu).
const TOGGLE: u32 = WM_APP + 1;
const FOUND: u32 = WM_APP + 2;
const LISTED: u32 = WM_APP + 3;
const LEFT: u32 = WM_APP + 4;
/// The look at the name picked (see `ui::look`) shows another of those it may (WPARAM its place among them), or went.
const LOOKED: u32 = WM_APP + 5;
/// Files and folders found were told of (their sizes, when they changed: see `tell`).
const TOLD: u32 = WM_APP + 6;
/// It is shown, and the first name in it that may be looked at larger is (see `preview`).
const PREVIEW: u32 = WM_APP + 7;

/// Its timers: the caret's blinking; a look at what may have changed while it shows (the drives' indexing, the
/// pictures made for its rows); a frame of the mark of the kind chosen sliding to it.
const CARET: usize = 1;
const WATCH: usize = 2;
const WATCH_EVERY: u32 = 250;
const SLIDING: usize = 3;
const SLIDING_EVERY: u32 = 10;

/// Its measures (DIPs): how wide its sheet is; the field written in, the row of kinds, a heading, a name, the best
/// name, the line saying how many more there are, the foot; how tall the list grows before it scrolls; the room round
/// the sheet for its shadow; how much a turn of the wheel scrolls.
const WIDTH: f32 = 680.0;
const FIELD: f32 = 56.0;
const CHIPS: f32 = 40.0;
const HEAD: f32 = 30.0;
const ROW: f32 = 44.0;
const BEST: f32 = 60.0;
const OTHERS: f32 = 32.0;
const FOOT: f32 = 30.0;
const LIST_MOST: f32 = 440.0;
const LIST_PAD: f32 = 4.0;
const NONE_FOUND: f32 = 72.0;
const SHADOW: f32 = 24.0;
const WHEEL: f32 = 3.0 * ROW;
/// The list's rows inset from the sheet's sides, and where an icon and the words after it stand.
const INSET: f32 = 6.0;
const ICON: f32 = 20.0;
const BEST_ICON: f32 = 32.0;
const TEXT_LEFT: f32 = 52.0;
const BEST_TEXT_LEFT: f32 = 62.0;
/// Room on the right of a file's row for its size and when it changed.
const META: f32 = 150.0;
/// How far down its screen's room the sheet's top is.
const DOWN: f32 = 0.16;
/// The most that may be typed (UTF-16 units).
const TYPED_MOST: usize = 260;
/// How long a word that something was done (copied) stays in the foot.
const TOLD_FOR: std::time::Duration = std::time::Duration::from_millis(1600);

/// Its window, made with its thread when it is first asked for (see `toggle`).
static WINDOW: OnceLock<Option<isize>> = OnceLock::new();

/// Shows the search window, or hides it where it is up in front: from any thread (the shortcut's, the workbench's).
pub fn toggle() {
    post(TOGGLE);
}

/// Shows the search window, and the first name in it that may be looked at larger looked at, as its preview's shortcut
/// there does: from any thread (the guide's, which shows the shortcut).
pub fn preview() {
    post(PREVIEW);
}

/// Tells its window `message`, made with its thread first where it is not yet.
fn post(message: u32) {
    let made = WINDOW.get_or_init(|| {
        let (sender, made) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("glance: search window".into()).spawn(move || run(sender)).ok()?;
        made.recv().ok().flatten()
    });
    if let Some(hwnd) = *made {
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), message, WPARAM(0), LPARAM(0)) };
    }
}

/// Whether it is in front (the keyboard's): its keys its own, the preview's shortcut among them.
pub fn in_front() -> bool {
    WINDOW.get().copied().flatten().is_some_and(|hwnd| unsafe { GetForegroundWindow() }.0 as isize == hwnd)
}

/// What is asked of the finding thread: what is typed, in which view, the files all or not, in which language; its
/// number (an answer to an older one is not shown), and whether what is picked goes back to the first.
struct Asked {
    number: u64,
    typed: String,
    view: View,
    expanded: bool,
    lang: Lang,
    fresh: bool,
}

/// What the finding thread found last: for which asking, and whether it was a fresh one.
static ANSWER: Mutex<Option<(u64, bool, Shown)>> = Mutex::new(None);

/// The asking last made (see `Finder::ask`), and the files and folders found told of since the window last took them:
/// for which asking each, its path, its size and when it changed.
static ASKED: AtomicU64 = AtomicU64::new(0);
static TOLD_OF: Mutex<Vec<Told>> = Mutex::new(Vec::new());
type Told = (u64, PathBuf, (u64, SystemTime));

/// What is `told` of files and folders found (see `tell`) taken into `shown` (found for the asking `shown_for`,
/// `asked` the last): what is told of those found for what was last asked, not shown yet (told before they came),
/// kept till they are; what is told of an asking before let go of. Whether anything was taken in.
fn take_told(told: &mut Vec<Told>, shown: &mut Shown, shown_for: u64, asked: u64) -> bool {
    let mut taken = false;
    told.retain(|(number, path, stat)| {
        if *number == shown_for && *number == asked {
            shown.told(path, *stat);
            taken = true;
            return false;
        }
        *number == asked
    });
    taken
}

/// What the preview's shortcut does (see `Finder::preview`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Previewing {
    /// The look up from it goes.
    Close,
    /// The name picked is looked at larger.
    Open,
    /// What is typed is not found yet (what shows is of what was typed before): what it finds picked is looked at
    /// larger as it comes.
    WhenFound,
    /// Pressed again before it came: not.
    Not,
}

/// What the preview's shortcut does: its look up (`looking`), what shows found for what was last asked or not
/// (`found`), it pressed already as that is found (`waiting`).
fn previewing(looking: bool, found: bool, waiting: bool) -> Previewing {
    match (looking, found, waiting) {
        (true, ..) => Previewing::Close,
        (false, true, _) => Previewing::Open,
        (false, false, false) => Previewing::WhenFound,
        (false, false, true) => Previewing::Not,
    }
}

/// How long what is told of the files found gathers before the window is told (it is drawn again as it is).
const TELL_EVERY: Duration = Duration::from_millis(100);

/// What is asked looked for, the latest only (those asked meanwhile left), and the window told as it is found; the
/// files and folders found then told of (see `tell`).
fn work(asks: Receiver<Asked>, hwnd: isize, tells: Sender<(u64, Vec<(PathBuf, bool)>)>) {
    while let Ok(mut asked) = asks.recv() {
        while let Ok(newer) = asks.try_recv() {
            asked = Asked { fresh: asked.fresh || newer.fresh, ..newer };
        }
        let programs = crate::station::found::all().unwrap_or_default();
        let notes = crate::notes::listed();
        let held = run::shown();
        let lately = crate::station::recent::shown();
        let files = |query: &crate::search::Query, most: usize| crate::search::find(query, most);
        let sources = Sources { programs: &programs, notes: &notes, held: &held.items, lately: &lately, files: &files, lang: asked.lang };
        let shown = results::gather(&asked.typed, asked.view, asked.expanded, &sources);
        let paths = shown.paths();
        *ANSWER.lock().unwrap() = Some((asked.number, asked.fresh, shown));
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), FOUND, WPARAM(0), LPARAM(0)) };
        if !paths.is_empty() {
            let _ = tells.send((asked.number, paths));
        }
    }
}

/// The files and folders found told of (each one's size and when it changed), on a thread of their own: what is found
/// shows at once, a drive slow to answer (a USB drive busy) holds nothing of it. The latest asking's only, left as
/// another is asked; the window told as they gather.
fn tell(asks: Receiver<(u64, Vec<(PathBuf, bool)>)>, hwnd: isize) {
    while let Ok(mut asked) = asks.recv() {
        while let Ok(newer) = asks.try_recv() {
            asked = newer;
        }
        let (number, paths) = asked;
        let mut since = Instant::now();
        let mut gathered = false;
        for (path, _) in paths {
            if ASKED.load(Ordering::Acquire) != number {
                break;
            }
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let Ok(changed) = meta.modified() else { continue };
            TOLD_OF.lock().unwrap().push((number, path, (meta.len(), changed)));
            gathered = true;
            if since.elapsed() >= TELL_EVERY {
                since = Instant::now();
                gathered = false;
                let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), TOLD, WPARAM(0), LPARAM(0)) };
            }
        }
        if gathered {
            let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), TOLD, WPARAM(0), LPARAM(0)) };
        }
    }
}

/// The window's thread: its window made (told to `made`), and its messages run till Glance ends.
fn run(made: Sender<Option<isize>>) {
    // (The shell, which opens what is chosen, expects a single-threaded apartment.)
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let hwnd = unsafe { make_window() };
    // (Drawn first on the main screen's device: it takes up its own screen's as it is drawn, see `Finder::draw`.)
    let (Some(hwnd), Ok(gfx)) = (hwnd, gfx::for_main_screen()) else {
        let _ = made.send(None);
        return;
    };
    let (asks, asked) = std::sync::mpsc::channel();
    let to = hwnd.0 as isize;
    let (tells, told) = std::sync::mpsc::channel();
    let finding = std::thread::Builder::new().name("glance: search finding".into()).spawn(move || work(asked, to, tells));
    let telling = std::thread::Builder::new().name("glance: search files told".into()).spawn(move || tell(told, to));
    if finding.is_err() || telling.is_err() {
        let _ = made.send(None);
        return;
    }
    FINDER.with(|finder| *finder.borrow_mut() = Some(Finder::new(hwnd, gfx, asks)));
    let _ = made.send(Some(hwnd.0 as isize));
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // A menu up takes its own messages (see `menu::Menu`).
            if with(|finder| finder.menu_message(&msg)).unwrap_or(false) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            with(Finder::menu_left);
        }
    }
}

unsafe fn make_window() -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = w!("GlanceSearch");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        // Not layered: the compositor gives a layered window nothing of the screen behind to frost.
        let hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP, class, w!("Glance"), WS_POPUP, 0, 0, 1, 1, None, None, Some(module.into()), None).ok()?;
        // (It shows and goes at once, with no fade of the system's.)
        let disabled = windows::core::BOOL::from(true);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &disabled as *const _ as *const _, size_of::<windows::core::BOOL>() as u32);
        Some(hwnd)
    }
}

thread_local! {
    static FINDER: RefCell<Option<Finder>> = const { RefCell::new(None) };
}

/// `act` with the window's state, where it is not in use already (a message sent to the window while it acts, as
/// showing it sends some).
fn with<R>(act: impl FnOnce(&mut Finder) -> R) -> Option<R> {
    FINDER.with(|finder| finder.try_borrow_mut().ok().and_then(|mut finder| finder.as_mut().map(act)))
}

/// What the window shows: what is drawn, or (glass, Windows 11) that over a pane of the screen behind it frosted by
/// the compositor.
enum Drawn {
    Plain(Surface),
    Frosted(crate::ui::glass::Frosted),
}

/// What a place on the sheet is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Field,
    Clear,
    Chip(View),
    /// A name, by its place among the names shown.
    Row(usize),
    ShowAll,
    Sheet,
}

/// What can be done with a name (from its menu, or by its keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Open,
    Admin,
    Show,
    Copy,
    CopyPath,
}

struct Finder {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    drawn: Option<Drawn>,
    /// Where its window takes the pointer: its sheet as tall as it is drawn, and the room for its shadow round it (the
    /// window is as tall as the sheet's tallest).
    footprint: gfx::Footprint,
    showing: Option<crate::ui::life::Showing>,
    skin: Skin,
    /// Dark or light, and how frosted its glass is (0–1), as the panel's would be over what is behind it (see `dress`).
    dark: bool,
    frost: f32,
    lang: Lang,
    scale: f32,
    /// The room round the sheet for its shadow (DIPs), and where the window is on the screen.
    room: f32,
    rect: RECT,
    typing: Typing,
    caret_on: bool,
    /// Words being put together by the input method (shown by it, where the field's hint would be).
    composing: bool,
    /// How far the field's words are scrolled left, to show the caret.
    shifted: f32,
    view: View,
    expanded: bool,
    shown: Shown,
    asked: u64,
    /// The asking what is shown was found for.
    shown_for: u64,
    /// The preview's shortcut pressed before what was last asked was found: what it finds picked is looked at larger
    /// as it comes (see `previewing`).
    look_found: bool,
    asks: Sender<Asked>,
    /// The name picked (by its place among those shown), and how far the list is scrolled; whether it is to scroll to
    /// show the one picked when next drawn.
    pick: usize,
    scroll: f32,
    follow: bool,
    /// Where each thing drawn is (DIPs on the sheet), and the field's words' box and its caret.
    hits: Vec<(Rect, Hit)>,
    words: Rect,
    caret: Option<Rect>,
    /// What a button went down on, and where (screen px); whether words are being selected by the mouse.
    pressed: Option<(Hit, POINT)>,
    selecting: bool,
    /// Where the pointer was last moved to (screen px): a name comes under a pointer that stays where it is as the
    /// list scrolls, and is not picked by that.
    pointer: POINT,
    /// The mark of the kind chosen, sliding to it from the one chosen before, as the panel's tabs' do.
    marks: crate::ui::thumb::Dots<View>,
    menu: Option<(Menu, Vec<Act>, usize)>,
    /// What was in front before it was shown, given the front back as it is closed by its key.
    before: HWND,
    /// How many pictures had been made as it last drew, and whether a row waited for one.
    pictures: u64,
    waiting: bool,
    /// What was last done, said in the foot a moment.
    told: Option<(String, Instant)>,
    /// How tall the sheet is now (DIPs).
    height: f32,
    /// Each name looked at larger may be, by its place among those shown, in the look's order (see `preview`).
    looked: Vec<usize>,
    /// The first name found that may be looked at larger to be, once one is (see `preview_first`).
    preview_owed: bool,
}

impl Finder {
    fn new(hwnd: HWND, gfx: Rc<Gfx>, asks: Sender<Asked>) -> Self {
        Finder {
            hwnd,
            gfx,
            drawn: None,
            footprint: gfx::Footprint::of(hwnd),
            showing: None,
            skin: Skin::Fluent,
            dark: false,
            frost: 1.0,
            lang: Lang::Zh,
            scale: 1.0,
            room: SHADOW,
            rect: RECT::default(),
            typing: Typing::most(TYPED_MOST),
            caret_on: true,
            composing: false,
            shifted: 0.0,
            view: View::All,
            expanded: false,
            shown: Shown::default(),
            asked: 0,
            shown_for: 0,
            look_found: false,
            asks,
            pick: 0,
            scroll: 0.0,
            follow: false,
            hits: Vec::new(),
            words: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            caret: None,
            pressed: None,
            selecting: false,
            pointer: POINT::default(),
            marks: crate::ui::thumb::Dots::new(),
            menu: None,
            before: HWND::default(),
            pictures: 0,
            waiting: false,
            told: None,
            height: 0.0,
            looked: Vec::new(),
            preview_owed: false,
        }
    }

    fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) }.as_bool()
    }

    fn toggle(&mut self) {
        if !self.visible() {
            self.show();
        } else if unsafe { GetForegroundWindow() } == self.hwnd {
            self.close(true);
        } else {
            self.come_forward();
        }
    }

    /// The skin, light or dark, and the language, as the panel has them now.
    fn look() -> (Skin, crate::ui::prefs::ThemePref, Lang) {
        let settings = crate::app().settings.lock().unwrap();
        let skin = Skin::named(&settings.skin);
        let pref = serde_json::from_value::<crate::ui::prefs::ThemePref>(settings.view["theme"].clone()).unwrap_or_default();
        (skin, pref, crate::update::language_in(&settings.view))
    }

    /// Dark or light, and how frosted, as the panel is dressed (see `panel::dress`): by the theme chosen, and where it
    /// follows the screen, by how light what is behind the sheet (`sheet`, physical px) is; frosted as much as that is
    /// busy, or clashes with the theme. (Glass always as frosted as it could be, it showed nothing of what was behind
    /// it, a sheet of white.) What is behind not to be read, as frosted as it may be.
    fn dress(&mut self, pref: crate::ui::prefs::ThemePref, sheet: RECT) {
        let behind = crate::ui::backdrop::Capture::take(sheet).map(|capture| capture.luminance(sheet, self.scale));
        let dark = theme::is_dark(pref, behind.map(|(mean, _)| mean));
        let frost = behind.map_or(1.0, |(mean, spread)| crate::ui::skins::frost(mean, spread, dark));
        if dark != self.dark {
            self.drawn = None;
        }
        (self.dark, self.frost) = (dark, frost);
    }

    fn show(&mut self) {
        super::super::settings_window::guide::saw(super::super::settings_window::guide::Saw::Searched);
        let (skin, pref, lang) = Finder::look();
        if skin != self.skin {
            self.drawn = None;
        }
        (self.skin, self.lang) = (skin, lang);
        // The drives indexed (once, from now, the machine settled or not), and the Start menu read again (as programs
        // come and go).
        crate::search::asked();
        let hwnd = self.hwnd.0 as isize;
        crate::station::found::look(move || {
            let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LISTED, WPARAM(0), LPARAM(0)) };
        });
        // On the screen the pointer is on, in its middle, its top a little way down.
        let mut cursor = POINT::default();
        let _ = unsafe { GetCursorPos(&mut cursor) };
        let Some(contact) = crate::mouse::monitor_at(cursor) else { return };
        // (A name the sheet comes up under the pointer on is not picked by that: only by the pointer moving.)
        self.pointer = cursor;
        self.scale = contact.scale;
        let scale = self.scale;
        let px = |dips: f32| (dips * scale).round() as i32;
        let work = contact.work;
        // The sheet (at its tallest), and round it the room for its shadow.
        let (sheet_w, sheet_h) = (px(WIDTH), px(Finder::tallest()));
        let sheet_left = (work.left + work.right - sheet_w) / 2;
        let sheet_top = (work.top + ((work.bottom - work.top) as f32 * DOWN) as i32).min(work.bottom - sheet_h).max(work.top);
        self.dress(pref, RECT { left: sheet_left, top: sheet_top, right: sheet_left + sheet_w, bottom: sheet_top + sheet_h });
        self.room = match skin {
            Skin::Glass => theme::room(&Theme::new(Skin::Glass, self.dark).shadows).max(SHADOW),
            _ => SHADOW,
        };
        let (left, top) = (sheet_left - px(self.room), sheet_top - px(self.room));
        self.rect = RECT { left, top, right: sheet_left + sheet_w + px(self.room), bottom: sheet_top + sheet_h + px(self.room) };
        // What was typed kept, all of it selected: typed over, or looked for again.
        self.typing.anchor = 0;
        self.typing.caret = self.typing.text.len();
        self.typing.follow = true;
        self.expanded = false;
        self.told = None;
        self.ask(true);
        self.before = unsafe { GetForegroundWindow() };
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), left, top, self.rect.right - left, self.rect.bottom - top, SWP_NOACTIVATE);
        }
        self.draw();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        crate::widget::own_window_changed(self.rect);
        self.come_forward();
        unsafe {
            let _ = SetTimer(Some(self.hwnd), CARET, GetCaretBlinkTime(), None);
            let _ = SetTimer(Some(self.hwnd), WATCH, WATCH_EVERY, None);
        }
    }

    /// In front, with the keyboard.
    fn come_forward(&mut self) {
        unsafe {
            if !SetForegroundWindow(self.hwnd).as_bool() {
                crate::journal::note("search: not let to the front");
            }
            let _ = SetFocus(Some(self.hwnd));
        }
        self.caret_on = true;
        self.draw();
    }

    /// The window hidden; the front given back to what had it before, where it is closed by its key (`give_back`).
    fn close(&mut self, give_back: bool) {
        if let Some((menu, ..)) = self.menu.take() {
            menu.close();
        }
        if self.looking() {
            look::close();
        }
        self.look_found = false;
        let held = unsafe { GetForegroundWindow() } == self.hwnd;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), CARET);
            let _ = KillTimer(Some(self.hwnd), WATCH);
            let _ = KillTimer(Some(self.hwnd), SLIDING);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        crate::widget::own_window_changed(self.rect);
        self.pressed = None;
        self.selecting = false;
        self.preview_owed = false;
        // (What it draws with let go of while it is hidden.)
        self.drawn = None;
        self.showing = None;
        if give_back && held && unsafe { IsWindow(Some(self.before)) }.as_bool() {
            unsafe {
                let _ = SetForegroundWindow(self.before);
            }
        }
    }

    /// The tallest the sheet may be (DIPs).
    fn tallest() -> f32 {
        FIELD + 1.0 + CHIPS + 1.0 + LIST_MOST + 1.0 + FOOT
    }

    /// What is typed looked for (again), in the view chosen: `fresh`, the first name picked once it is found.
    fn ask(&mut self, fresh: bool) {
        self.asked += 1;
        ASKED.store(self.asked, Ordering::Release);
        let _ = self.asks.send(Asked { number: self.asked, typed: self.typing.string(), view: self.view, expanded: self.expanded, lang: self.lang, fresh });
    }

    /// What was found, if it is for what was last asked.
    fn found(&mut self) {
        let Some((number, fresh, shown)) = ANSWER.lock().unwrap().take() else { return };
        if number != self.asked {
            return;
        }
        let picked = self.picked().map(|row| row.what.clone());
        // (What is looked at larger is of the list as it was: a list found afresh, it goes.)
        if fresh && self.looking() {
            look::close();
        }
        // (Found again: its files told of as before, till they are told of again.)
        let was = std::mem::replace(&mut self.shown, shown);
        self.shown_for = number;
        self.shown.told_as(&was);
        // (Told of before it came: see `told`.)
        self.take_told();
        // (Found again as it was, the same name stays picked.)
        let again = (!fresh).then(|| picked.and_then(|what| self.shown.rows().position(|row| row.what == what))).flatten();
        self.pick = again.unwrap_or(0);
        if fresh {
            self.scroll = 0.0;
        }
        self.follow = true;
        // (Asked to be looked at larger before it came: what it found picked is.)
        if std::mem::take(&mut self.look_found) {
            self.preview();
        }
        self.draw();
        if self.preview_owed {
            self.preview_first();
        }
    }

    /// The files and folders found told of (see `tell`), drawn so.
    fn told(&mut self) {
        if self.take_told() {
            self.draw();
        }
    }

    /// What is told of the files and folders shown (see `tell`) taken in (see `take_told`).
    fn take_told(&mut self) -> bool {
        take_told(&mut TOLD_OF.lock().unwrap(), &mut self.shown, self.shown_for, self.asked)
    }

    fn rows(&self) -> usize {
        self.shown.rows().count()
    }

    fn picked(&self) -> Option<&Row> {
        self.shown.rows().nth(self.pick)
    }

    /// The view `view` chosen, looked for again.
    fn choose(&mut self, view: View) {
        if view != self.view {
            self.view = view;
            self.expanded = false;
            self.ask(true);
        }
        self.draw();
    }

    // ----- Drawing -----

    /// Its menus' colours, its corners as round as the panel's (a sheet as large as a panel, not a menu).
    fn colours(&self, frosted: bool) -> Colours {
        Colours { radius: Theme::new(self.skin, self.dark).radius, ..Colours::new(Look::of(self.skin, self.dark).frosted(self.frost, true), frosted) }
    }

    /// The faces it is written in: the skin's (Windows' own for Windows 11's).
    fn font(&self, size: f32, weight: f32) -> Font {
        let family = match self.skin {
            Skin::Fluent => Family::Segoe,
            skin => Theme::new(skin, self.dark).body.family,
        };
        Font::new(family, size, weight)
    }

    /// How tall the list's lines are, each (DIPs).
    fn line_height(line: &Line) -> f32 {
        match line {
            Line::Head(..) => HEAD,
            Line::Row(_, true) => BEST,
            Line::Row(_, false) => ROW,
            Line::Others(_) => OTHERS,
        }
    }

    /// How tall all the list's lines are (DIPs), and how tall the list is shown (no taller than `LIST_MOST`: it
    /// scrolls past that). Nothing found for what is typed, a line saying so.
    fn list_height(&self) -> (f32, f32) {
        if self.shown.lines.is_empty() {
            let typed = !self.typing.string().trim().is_empty();
            return (0.0, if typed { NONE_FOUND } else { 0.0 });
        }
        let whole = self.shown.lines.iter().map(Finder::line_height).sum::<f32>() + 2.0 * LIST_PAD;
        (whole, whole.min(LIST_MOST))
    }

    fn draw(&mut self) {
        if !self.visible() && self.rect.right == self.rect.left {
            return;
        }
        self.height = self.sheet_height();
        // On the device for the screen it is on (see `gfx::on_screen`): what it draws on made anew on it as it opens on
        // a screen another adapter drives, or its device was lost.
        if let Ok(gfx) = gfx::for_area(self.rect) {
            if !Rc::ptr_eq(&gfx, &self.gfx) {
                self.drawn = None;
                self.showing = None;
                self.gfx = gfx;
            }
        }
        if self.drawn.is_none() {
            use crate::ui::skins::{ACRYLIC_BLUR, ACRYLIC_SATURATION, FROST_BLUR, GLASS_SATURATION};
            let frosting = match self.skin {
                Skin::Glass => Some((FROST_BLUR, GLASS_SATURATION)),
                Skin::Fluent => Some((ACRYLIC_BLUR, ACRYLIC_SATURATION)),
                Skin::Paper => None,
            };
            // (Nothing shows of what it drew before, till it is drawn on that.)
            self.footprint.hidden();
            self.drawn = match frosting.and_then(|frosting| crate::ui::glass::Frosted::new(&self.gfx, self.hwnd, frosting).ok()) {
                Some(frosted) => Some(Drawn::Frosted(frosted)),
                None => Surface::new(&self.gfx, self.hwnd).ok().map(Drawn::Plain),
            };
        }
        let Some(mut drawn) = self.drawn.take() else { return };
        if self.showing.is_none() {
            self.showing = Some(crate::ui::life::Showing::of(&self.gfx));
        }
        let gfx = self.gfx.clone();
        let size = ((self.rect.right - self.rect.left) as u32, (self.rect.bottom - self.rect.top) as u32);
        let (scale, room) = (self.scale, self.room);
        let showing = RECT { left: 0, top: 0, right: size.0 as i32, bottom: (((self.height + 2.0 * room) * scale).ceil() as i32).min(size.1 as i32) };
        // (The compositor having taken what was drawn last: the frosted sheet is put together by Windows' own, whose
        // frame is waited for.)
        let frosted = matches!(drawn, Drawn::Frosted(_));
        let taken = self.footprint.take(showing, || if frosted { unsafe { windows::Win32::Graphics::Dwm::DwmFlush() } } else { unsafe { gfx.dcomp.WaitForCommitCompletion() } });
        let done = taken.and_then(|_| match &mut drawn {
            Drawn::Plain(surface) => surface.draw(&gfx, size, scale, |frame| {
                self.paint(frame, false, (room, room));
            }),
            Drawn::Frosted(frosted) => {
                // (Glass as frosted as the panel's would be there, Windows 11's acrylic always: as their menus are.)
                frosted.set_frost(if self.skin == Skin::Glass { self.frost } else { 1.0 });
                let radius = self.colours(true).radius;
                let pane = (room * scale, room * scale, WIDTH * scale, self.height * scale, radius * scale);
                frosted.draw(&gfx, size, scale, pane, |frame| self.paint(frame, true, (room, room)))
            }
        });
        if done.is_ok() {
            self.footprint.shown(showing);
        }
        // (Not drawn, it keeps what it draws on, and draws on it again; the device gone, it makes all anew.)
        match done {
            Err(error) if gfx::failed("search", &gfx, Some(&error)) => {}
            _ => self.drawn = Some(drawn),
        }
        self.place_ime();
        // (Where each name shows now, told to the look at one: it goes back into its row where that is.)
        look::tiles(self.hwnd, self.rows_on_screen().into_iter().filter_map(|(at, rect)| Some((key_of(&self.shown.rows().nth(at)?.what), rect))).collect());
    }

    /// How tall the sheet is for what it shows (DIPs).
    fn sheet_height(&self) -> f32 {
        let (_, list) = self.list_height();
        FIELD + 1.0 + CHIPS + if list > 0.0 { 1.0 + list } else { 0.0 } + 1.0 + FOOT
    }

    /// The sheet drawn, its corner at `origin` (over the screen frosted behind it, `frosted`).
    fn paint(&mut self, frame: &Frame, frosted: bool, (x, y): (f32, f32)) {
        frame.origin(x, y);
        frame.crisp_text();
        self.hits.clear();
        let c = self.colours(frosted);
        let height = self.height;
        // The sheet: glass's a piece of the glass, lit along its edge; the others' as their menus' are.
        if self.skin == Skin::Glass && frosted {
            let theme = Theme::new(Skin::Glass, self.dark);
            if let Err(error) = crate::ui::skins::glass_sheet(frame, &theme, Rect { x: 0.0, y: 0.0, w: WIDTH, h: height }, c.radius, self.frost) {
                gfx::failed("search", frame.gfx, Some(&error));
            }
            frame.origin(x, y);
        } else {
            frame.shadow(c.shadow, 0.0, 0.0, WIDTH, height, c.radius, 16.0, 8.0);
            frame.fill_rounded(c.sheet, 0.0, 0.0, WIDTH, height, c.radius);
            frame.stroke_rounded(crate::ui::canvas::Fill::Solid(c.stroke), 0.0, 0.0, WIDTH, height, c.radius, 1.0);
            if let Some(rim) = c.rim {
                frame.stroke_rounded(crate::ui::canvas::Fill::Down { top: 0.0, from: rim, bottom: height, to: rim.alpha(rim.a * 0.3) }, 1.0, 1.0, WIDTH - 2.0, height - 2.0, c.radius - 1.0, 1.0);
            }
        }
        self.hits.push((Rect { x: 0.0, y: 0.0, w: WIDTH, h: height }, Hit::Sheet));
        let ink = Inks::of(self, &c);
        self.paint_field(frame, &ink);
        frame.fill(c.rule, 0.0, FIELD, WIDTH, 1.0);
        self.paint_chips(frame, &ink, FIELD + 1.0);
        let mut y = FIELD + 1.0 + CHIPS;
        let (_, list) = self.list_height();
        if list > 0.0 {
            frame.fill(c.rule, 0.0, y, WIDTH, 1.0);
            y += 1.0;
            self.paint_list(frame, &ink, y, list);
            y += list;
        }
        frame.fill(c.rule, 0.0, y, WIDTH, 1.0);
        self.paint_foot(frame, &ink, y + 1.0);
    }

    fn paint_field(&mut self, frame: &Frame, ink: &Inks) {
        frame.icon(Icon::Search, Point { x: 26.0, y: FIELD / 2.0 }, 20.0, ink.text2);
        let font = self.font(19.0, 400.0);
        let typed = !self.typing.text.is_empty();
        let clear = 40.0;
        let words = Rect { x: TEXT_LEFT, y: 0.0, w: WIDTH - TEXT_LEFT - if typed { clear + 8.0 } else { 20.0 }, h: FIELD };
        self.words = words;
        let text = self.typing.string();
        // (Its words' ink across its middle, as every line's: see `Canvas::line_top`.)
        let top = super::super::canvas::Canvas::line_in(frame, font, 0.0, FIELD);
        let layout = frame.gfx.layout(&text, font, 100_000.0, Align::Start);
        let (x, line_top, tall) = caret_at(&layout, self.typing.caret);
        // Scrolled to show the caret, never past the words' end.
        if self.typing.follow {
            self.shifted = self.shifted.min(x).max(x + 2.0 - words.w);
            self.typing.follow = false;
        }
        let wide = frame.gfx.measure(&text, font);
        self.shifted = self.shifted.clamp(0.0, (wide + 2.0 - words.w).max(0.0));
        let left = words.x - self.shifted;
        frame.clip(words.x, 0.0, words.w, FIELD);
        let selection = self.typing.selection();
        if !selection.is_empty() {
            let mut count = 0;
            let _ = unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, None, &mut count) };
            let mut spans = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
            let _ = unsafe { layout.HitTestTextRange(selection.start as u32, selection.len() as u32, 0.0, 0.0, Some(&mut spans), &mut count) };
            for span in &spans {
                frame.fill(ink.selection, left + span.left, top + span.top, span.width.max(4.0), span.height);
            }
        }
        if typed {
            frame.text(&text, font, ink.text, left, top, 100_000.0, Align::Start);
        } else if !self.composing {
            frame.text(self.lang.pick("搜索应用、设置、文件和便签", "Search apps, settings, files and notes"), font, ink.text3, words.x, top, words.w, Align::Start);
        }
        let focused = unsafe { GetForegroundWindow() } == self.hwnd;
        self.caret = Some(Rect { x: left + x, y: top + line_top, w: 1.0, h: tall });
        if focused && self.caret_on && selection.is_empty() {
            frame.fill(ink.text, left + x, top + line_top, 1.0, tall);
        }
        frame.unclip();
        self.hits.push((words, Hit::Field));
        if typed {
            let r = Rect { x: WIDTH - clear - 8.0, y: (FIELD - 32.0) / 2.0, w: 32.0 + 8.0, h: 32.0 };
            if self.hovered() == Some(Hit::Clear) {
                frame.fill_rounded(ink.hover, r.x + 4.0, r.y, 32.0, 32.0, 6.0);
            }
            frame.icon(Icon::Close, Point { x: r.x + 4.0 + 16.0, y: FIELD / 2.0 }, 16.0, ink.text3);
            self.hits.push((r, Hit::Clear));
        }
    }

    fn paint_chips(&mut self, frame: &Frame, ink: &Inks, top: f32) {
        // Marked as the panel's tabs are in each skin (see `view::paint_tabs`): glass's chosen on a pill of its glass,
        // Windows 11's with a short bar of its accent under it, chart paper's underlined in its ink, its name no further
        // out than its line (no room round it).
        let theme = Theme::new(self.skin, self.dark);
        let font = self.font(12.5, 400.0);
        let paper = self.skin == Skin::Paper;
        let (tall, pad, gap) = if paper { (26.0, 0.0, 20.0) } else { (26.0, 12.0, 4.0) };
        let y = top + (CHIPS - tall) / 2.0;
        let (ascent, descent) = frame.gfx.baseline(font);
        let hovered = self.hovered();
        let mut boxes = Vec::new();
        let mut x = 26.0 - pad;
        for view in View::EVERY {
            let w = frame.gfx.measure(view.name(self.lang), font) + 2.0 * pad;
            boxes.push((x, w));
            x += w + gap;
        }
        let chosen = View::EVERY.iter().position(|&view| view == self.view).unwrap_or(0);
        let hovered_at = View::EVERY.iter().position(|&view| hovered == Some(Hit::Chip(view)));
        // (Its frames drawn as it slides, till it is there.)
        let sliding = self.marks.choose(std::time::Instant::now(), &[(0, self.view)]);
        unsafe {
            if sliding {
                SetTimer(Some(self.hwnd), SLIDING, SLIDING_EVERY, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), SLIDING);
            }
        }
        let slid = crate::ui::view::slid_at(self.marks.slid(0, self.view), |from| View::EVERY.iter().position(|&view| view == from));
        if self.skin == Skin::Glass {
            crate::ui::view::glass_choice(frame, &theme, (y, tall), &boxes, chosen, slid, hovered_at);
        } else {
            if let Some(&(x, w)) = hovered_at.filter(|&at| at != chosen).and_then(|at| boxes.get(at)) {
                frame.fill_rounded(theme.hover, x, y, w, tall, theme.control_radius.min(tall / 2.0));
            }
            let under = if paper { y + tall - 2.0 } else { y + tall - 3.0 };
            crate::ui::view::choice_mark(frame, &theme, &boxes, chosen, slid, under, 14.0, 4.0);
        }
        for (at, (&view, &(x, w))) in View::EVERY.iter().zip(&boxes).enumerate() {
            let colour = if at == chosen || hovered_at == Some(at) { ink.text } else { ink.text2 };
            frame.text(view.name(self.lang), font, colour, x + pad, y + (tall - ascent - descent) / 2.0, w, Align::Start);
            self.hits.push((Rect { x, y, w, h: tall }, Hit::Chip(view)));
        }
    }

    fn paint_list(&mut self, frame: &Frame, ink: &Inks, top: f32, height: f32) {
        if self.shown.lines.is_empty() {
            let typed = self.typing.string();
            let said = match self.lang {
                Lang::Zh => format!("未找到与“{}”相符的项目", typed.trim()),
                Lang::En => format!("Nothing found for \u{201C}{}\u{201D}", typed.trim()),
            };
            let font = self.font(13.0, 400.0);
            let wide = frame.gfx.measure(&said, font).min(WIDTH - 40.0);
            let (ascent, descent) = frame.gfx.baseline(font);
            frame.text(&said, font, ink.text3, (WIDTH - wide) / 2.0, top + (height - ascent - descent) / 2.0, wide + 1.0, Align::Start);
            return;
        }
        let (whole, _) = self.list_height();
        let most = (whole - height).max(0.0);
        // Scrolled to show the name picked, where it moved.
        if self.follow {
            let mut y = LIST_PAD;
            let mut at = 0;
            for line in &self.shown.lines {
                let tall = Finder::line_height(line);
                if let Line::Row(..) = line {
                    if at == self.pick {
                        // (The first name shows its heading too.)
                        let from = if at == 0 { 0.0 } else { y };
                        self.scroll = self.scroll.min(from).max(y + tall + LIST_PAD - height);
                        break;
                    }
                    at += 1;
                }
                y += tall;
            }
            self.follow = false;
        }
        self.scroll = self.scroll.clamp(0.0, most);
        frame.clip(0.0, top, WIDTH, height);
        let mut y = top + LIST_PAD - self.scroll;
        let mut at = 0;
        let lines = std::mem::take(&mut self.shown.lines);
        self.waiting = false;
        for line in &lines {
            let tall = Finder::line_height(line);
            if y + tall >= top && y <= top + height {
                match line {
                    Line::Head(group, more) => self.paint_head(frame, ink, y, *group, *more),
                    Line::Row(row, best) => self.paint_row(frame, ink, y, row, *best, at, Rect { x: 0.0, y: top, w: WIDTH, h: height }),
                    Line::Others(count) => {
                        let said = match self.lang {
                            Lang::Zh => format!("另有 {} 项未列出，输入更多文字可缩小范围", count_said(*count)),
                            Lang::En => format!("{} more not listed: type more to narrow them down", count_said(*count)),
                        };
                        let font = self.font(11.5, 400.0);
                        let (ascent, descent) = frame.gfx.baseline(font);
                        frame.text(&said, font, ink.text3, 18.0, y + (tall - ascent - descent) / 2.0, WIDTH - 36.0, Align::Start);
                    }
                }
            }
            if let Line::Row(..) = line {
                at += 1;
            }
            y += tall;
        }
        self.shown.lines = lines;
        frame.unclip();
        // Its scroll bar, where it scrolls.
        if most > 0.0 {
            let thumb = (height * height / whole).max(24.0);
            let at = top + (height - thumb) * self.scroll / most;
            frame.fill_rounded(ink.text3.alpha(ink.text3.a * 0.6), WIDTH - 5.0, at + 2.0, 3.0, thumb - 4.0, 1.5);
        }
    }

    fn paint_head(&mut self, frame: &Frame, ink: &Inks, y: f32, group: Group, more: More) {
        let font = self.font(11.5, 600.0);
        let (ascent, descent) = frame.gfx.baseline(font);
        let middle = y + HEAD / 2.0 + 2.0;
        frame.text(group.name(self.lang), font, ink.text3, 18.0, middle - (ascent + descent) / 2.0, 300.0, Align::Start);
        let font = self.font(11.5, 400.0);
        match more {
            More::No => {}
            More::Others(count) => {
                let said = match self.lang {
                    Lang::Zh => format!("另有 {} 项", count_said(count)),
                    Lang::En => format!("{} more", count_said(count)),
                };
                frame.text(&said, font, ink.text3, WIDTH - 318.0, middle - (ascent + descent) / 2.0, 300.0, Align::End);
            }
            More::ShowAll(count) => {
                let said = match self.lang {
                    Lang::Zh => format!("显示全部 {} 项", count_said(count)),
                    Lang::En => format!("Show all {}", count_said(count)),
                };
                let wide = frame.gfx.measure(&said, font);
                let r = Rect { x: WIDTH - 18.0 - wide - 8.0, y: y + 3.0, w: wide + 16.0, h: HEAD - 4.0 };
                if self.hovered() == Some(Hit::ShowAll) {
                    frame.fill_rounded(ink.hover, r.x, r.y, r.w, r.h, 4.0);
                }
                frame.text(&said, font, ink.link, r.x + 8.0, middle - (ascent + descent) / 2.0, wide + 1.0, Align::Start);
                self.hits.push((r, Hit::ShowAll));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_row(&mut self, frame: &Frame, ink: &Inks, y: f32, row: &Row, best: bool, at: usize, list: Rect) {
        let tall = if best { BEST } else { ROW };
        let r = Rect { x: INSET, y, w: WIDTH - 2.0 * INSET, h: tall };
        let radius = if self.skin == Skin::Paper { Theme::new(Skin::Paper, self.dark).control_radius } else { 6.0 };
        if at == self.pick {
            frame.fill_rounded(ink.pick, r.x, r.y, r.w, r.h, radius);
            frame.fill_rounded(ink.accent, r.x, r.y + (tall - 16.0) / 2.0, 3.0, 16.0, 1.5);
        }
        // (Its place, cut to what of it shows of the list.)
        let top = y.max(list.y);
        let bottom = (y + tall).min(list.y + list.h);
        if bottom > top {
            self.hits.push((Rect { x: r.x, y: top, w: r.w, h: bottom - top }, Hit::Row(at)));
        }
        let side = if best { BEST_ICON } else { ICON };
        let centre = Point { x: INSET + if best { 10.0 } else { 12.0 } + side / 2.0, y: y + tall / 2.0 };
        self.paint_icon(frame, ink, row, centre, side);
        let left = if best { BEST_TEXT_LEFT } else { TEXT_LEFT };
        let meta = self.meta(row);
        let room = WIDTH - INSET - 12.0 - left - if meta.is_some() { META + 12.0 } else { 0.0 };
        let name_font = if best { self.font(15.0, 600.0) } else { self.font(13.5, 400.0) };
        let detail_font = self.font(11.5, 400.0);
        let (name_top, detail_top) = if best { (y + 10.0, y + 33.0) } else { (y + 5.0, y + 24.0) };
        // Where the words typed met the name: marked behind it (chart paper's underlined, as its ink would be).
        let name = row.name.as_str();
        let shown_name = crate::ui::canvas::fitted(frame, name, name_font, room);
        let shown_chars = shown_name.trim_end_matches('…').chars().count();
        let (ascent, descent) = frame.gfx.baseline(name_font);
        for &(from, to) in &row.marks {
            let to = to.min(shown_chars);
            if from >= to {
                continue;
            }
            let start = name.char_indices().nth(from).map_or(name.len(), |(at, _)| at);
            let end = name.char_indices().nth(to).map_or(name.len(), |(at, _)| at);
            let x = left + frame.gfx.measure(&name[..start], name_font);
            let w = frame.gfx.measure(&name[start..end], name_font);
            if self.skin == Skin::Paper {
                frame.fill(ink.text, x, name_top + frame.ink(&shown_name, name_font).1 + 2.0, w, 1.0);
            } else {
                frame.fill_rounded(ink.mark, x - 1.0, name_top + (ascent + descent) * 0.08, w + 2.0, (ascent + descent) * 0.9, 3.0);
            }
        }
        frame.text(&shown_name, name_font, ink.text, left, name_top, room + 2.0, Align::Start);
        frame.text(&row.detail, detail_font, ink.text3, left, detail_top, room, Align::Start);
        if let Some((size, when)) = meta {
            let right = WIDTH - INSET - 12.0 - META;
            frame.text(&when, detail_font, ink.text3, right, name_top + 1.0, META, Align::End);
            if let Some(size) = size {
                frame.text(&size, detail_font, ink.text3, right, detail_top, META, Align::End);
            }
        }
    }

    /// A row's size and when it changed, as said on its right (a file's, a folder's).
    fn meta(&self, row: &Row) -> Option<(Option<String>, String)> {
        let changed = row.changed?;
        Some((row.bytes.map(|bytes| crate::ui::text::bytes(bytes as f64)), when_said(changed, self.lang)))
    }

    /// A row's picture: the program's icon, the file's (a picture's own likeness), the folder's, as Explorer shows
    /// them; Glance's own drawn; till one is made, its kind's drawn.
    fn paint_icon(&mut self, frame: &Frame, ink: &Inks, row: &Row, centre: Point, side: f32) {
        let pixels = (side * self.scale).round() as u32;
        let picture = match &row.what {
            What::Program(target) => crate::station::thumbs::icon(Path::new(target), pixels),
            What::Path(path) => match row.kind {
                Kind::File(FileKind::Picture | FileKind::Screenshot) => crate::station::thumbs::of(path, pixels),
                _ => crate::station::thumbs::icon(path, pixels),
            },
            _ => None,
        };
        if let Some(picture) = picture {
            frame.picture(&picture, centre.x - side / 2.0, centre.y - side / 2.0, side, side, 0.0, 1.0);
            return;
        }
        let drawn = matches!(row.what, What::Program(_) | What::Path(_));
        self.waiting |= drawn;
        let icon = match row.kind {
            Kind::Program => Icon::AppWindow,
            Kind::Page => Icon::Settings,
            Kind::Tool => Icon::Wrench,
            Kind::Folder => Icon::Folder,
            Kind::File(FileKind::Picture | FileKind::Screenshot) => Icon::Image,
            Kind::File(FileKind::Audio) => Icon::Music,
            Kind::File(_) => Icon::File,
            Kind::Note => Icon::StickyNote,
            Kind::Held => Icon::Inbox,
            Kind::Copied => Icon::Clipboard,
            Kind::Todo => Icon::SquareCheck,
        };
        frame.icon(icon, centre, side * if drawn { 0.9 } else { 0.85 }, ink.text2);
    }

    fn paint_foot(&mut self, frame: &Frame, ink: &Inks, top: f32) {
        let font = self.font(11.5, 400.0);
        let (ascent, descent) = frame.gfx.baseline(font);
        let y = top + (FOOT - ascent - descent) / 2.0;
        let lang = self.lang;
        let left = match &self.told {
            Some((said, at)) if at.elapsed() < TOLD_FOR => said.clone(),
            _ if self.typing.string().trim().is_empty() => lang.pick("Enter 打开 · Ctrl+Enter 打开所在位置 · Tab 切换类别", "Enter to open · Ctrl+Enter to show in folder · Tab for kinds").into(),
            _ => match lang {
                Lang::Zh => format!("共 {} 项", count_said(self.shown.total)),
                Lang::En => format!("{} found", count_said(self.shown.total)),
            },
        };
        frame.text(&left, font, ink.text3, 16.0, y, WIDTH / 2.0 - 16.0, Align::Start);
        // How the index stands: being made (a dot breathing before it), or kept current.
        let (said, building) = standing_said(lang);
        let wide = frame.gfx.measure(&said, font).min(WIDTH / 2.0 - 32.0);
        let x = WIDTH - 16.0 - wide;
        frame.text(&said, font, ink.text3, x, y, wide + 1.0, Align::Start);
        if building {
            let phase = (crate::station::now_ms() % 1600) as f32 / 1600.0;
            let alpha = 0.35 + 0.65 * (0.5 - 0.5 * (phase * std::f32::consts::TAU).cos());
            frame.fill_circle(ink.accent.alpha(alpha), Point { x: x - 9.0, y: top + FOOT / 2.0 }, 3.0);
        }
    }

    // ----- Places -----

    /// What is at (`x`, `y`) on the sheet (DIPs), the last drawn first.
    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, hit)| *hit)
    }

    /// What the pointer is on.
    fn hovered(&self) -> Option<Hit> {
        let (x, y) = self.on_sheet(self.pointer);
        self.hit_at(x, y)
    }

    /// A point on the screen (px) on the sheet (DIPs).
    fn on_sheet(&self, at: POINT) -> (f32, f32) {
        ((at.x - self.rect.left) as f32 / self.scale - self.room, (at.y - self.rect.top) as f32 / self.scale - self.room)
    }

    /// The input method's words and choices put at the caret, in the field's face.
    fn place_ime(&self) {
        let Some(caret) = self.caret else { return };
        let px = |dips: f32| ((dips + self.room) * self.scale).round() as i32;
        crate::ui::typing::place_ime(self.hwnd, POINT { x: px(caret.x), y: px(caret.y) }, px(caret.y + caret.h) - px(caret.y), "Segoe UI", (19.0 * self.scale).round() as i32);
    }

    /// Where in the field's words a point on the sheet (DIPs) is, as a caret put there would be.
    fn words_at(&self, x: f32) -> usize {
        let layout = self.gfx.layout(&self.typing.string(), self.font(19.0, 400.0), 100_000.0, Align::Start);
        at_point(&layout, x - self.words.x + self.shifted, 1.0)
    }

    // ----- The mouse -----

    fn moved(&mut self, at: POINT) {
        let before = self.hovered();
        let moved = at.x != self.pointer.x || at.y != self.pointer.y;
        self.pointer = at;
        let (x, _) = self.on_sheet(at);
        if self.selecting {
            let to = self.words_at(x);
            self.typing.go(to, true);
            self.draw();
            return;
        }
        // Pressed on a file and moved far enough: dragged out (by the station, which takes the pointer from here).
        if let Some((Hit::Row(row), from)) = self.pressed {
            if run::drag_begins(from, at) {
                self.pressed = None;
                let path = match self.shown.rows().nth(row).map(|row| &row.what) {
                    Some(What::Path(path)) => Some(path.clone()),
                    _ => None,
                };
                if let Some(path) = path {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    crate::station::drag::announce();
                    run::ask(Ask::DragFiles(vec![path], unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }));
                }
                return;
            }
        }
        let hovered = self.hovered();
        // (Looked at larger, the name picked is had by the arrows or a click only: the pointer crosses others on its way.)
        if moved && !self.looking() {
            if let Some(Hit::Row(row)) = hovered {
                if row != self.pick {
                    self.pick = row;
                    self.draw();
                    return;
                }
            }
        }
        if hovered != before {
            self.draw();
        }
    }

    fn press(&mut self, at: POINT, shift: bool) {
        let (x, y) = self.on_sheet(at);
        let hit = self.hit_at(x, y);
        // Looked at larger: a press on a name looks at that one, and does nothing more. One elsewhere ends the look, and
        // is a press as ever.
        if self.looking() {
            if let Some(Hit::Row(row)) = hit {
                self.pressed = None;
                return self.look_to(row);
            }
            look::close();
        }
        self.pressed = hit.map(|hit| (hit, at));
        match hit {
            // Outside the sheet: away.
            None => self.close(false),
            Some(Hit::Field) => {
                let to = self.words_at(x);
                self.typing.go(to, shift);
                self.selecting = true;
                self.caret_on = true;
                unsafe { SetCapture(self.hwnd) };
                self.draw();
            }
            Some(_) => unsafe {
                SetCapture(self.hwnd);
            },
        }
    }

    fn release(&mut self, at: POINT, ctrl: bool) {
        unsafe {
            let _ = ReleaseCapture();
        }
        self.selecting = false;
        let Some((pressed, _)) = self.pressed.take() else { return };
        let (x, y) = self.on_sheet(at);
        if self.hit_at(x, y) != Some(pressed) {
            return;
        }
        match pressed {
            Hit::Clear => {
                self.typing.anchor = 0;
                self.typing.caret = self.typing.text.len();
                self.typing.put(&[]);
                self.ask(true);
                self.draw();
            }
            Hit::Chip(view) => self.choose(view),
            Hit::ShowAll => {
                self.expanded = true;
                self.ask(false);
            }
            Hit::Row(row) => {
                self.pick = row;
                self.act(if ctrl { Act::Show } else { Act::Open });
            }
            Hit::Field | Hit::Sheet => {}
        }
    }

    fn wheel(&mut self, turns: f32) {
        self.scroll -= turns * WHEEL;
        self.draw();
    }

    /// The menu of the name under the pointer, at the pointer.
    fn context(&mut self, at: POINT) {
        let (x, y) = self.on_sheet(at);
        let Some(Hit::Row(row)) = self.hit_at(x, y) else { return };
        self.pick = row;
        self.draw();
        let Some(picked) = self.picked() else { return };
        let lang = self.lang;
        let mut choices: Vec<(Act, &'static str, Icon)> = vec![(Act::Open, lang.pick("打开", "Open"), Icon::Maximize)];
        if runs_as_admin(picked) {
            choices.push((Act::Admin, lang.pick("以管理员身份运行", "Run as administrator"), Icon::AppWindow));
        }
        match &picked.what {
            What::Path(_) => {
                choices.push((Act::Show, lang.pick("打开所在位置", "Open file location"), Icon::Folder));
                if picked.kind != Kind::Folder {
                    choices.push((Act::Copy, lang.pick("复制", "Copy"), Icon::Copy));
                }
                choices.push((Act::CopyPath, lang.pick("复制路径", "Copy path"), Icon::Clipboard));
            }
            What::Item(_) => choices.push((Act::Copy, lang.pick("复制", "Copy"), Icon::Copy)),
            _ => {}
        }
        let items = choices.iter().map(|(_, label, icon)| Item { label, mark: Some(Mark::Icon(*icon)), checked: false, rule_before: false }).collect();
        if let Some(menu) = Menu::open_at(items, Look::of(self.skin, self.dark), at, false, false) {
            self.menu = Some((menu, choices.into_iter().map(|(act, ..)| act).collect(), row));
        }
    }

    /// A message for the menu up, if it is its own: taken.
    fn menu_message(&mut self, msg: &MSG) -> bool {
        let Some((menu, ..)) = &mut self.menu else { return false };
        if !menu.owns(msg.hwnd) {
            return false;
        }
        match menu.handle(msg) {
            menu::Handled::Not => false,
            menu::Handled::Taken => true,
            menu::Handled::Done(choice) => {
                self.end_menu(choice);
                true
            }
        }
    }

    /// The menu left by something else coming to the front: gone, and the window with it, where it was not the window.
    fn menu_left(&mut self) {
        if self.menu.as_ref().is_some_and(|(menu, ..)| menu.left()) {
            self.end_menu(None);
        }
    }

    fn end_menu(&mut self, choice: Option<usize>) {
        let Some((menu, acts, row)) = self.menu.take() else { return };
        menu.close();
        match choice {
            Some(at) => {
                self.pick = row;
                self.act(acts[at]);
            }
            None if unsafe { GetForegroundWindow() } != self.hwnd => self.close(false),
            None => {}
        }
    }

    // ----- The keyboard -----

    fn key(&mut self, key: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        // Looked at larger: Esc ends the look (and nothing more), the arrows go through what may be looked at; Enter and
        // Ctrl+C do as ever with the name picked; another key ends it, and is a key as ever. Files together looked at
        // (a folder's): the arrows, Enter, Ctrl+C and Backspace the look's, as on the panel (see `look::among`).
        if self.looking() {
            if look::among() {
                let handed = match key {
                    VK_LEFT => Some(look::Key::Left),
                    VK_RIGHT => Some(look::Key::Right),
                    VK_UP => Some(look::Key::Up),
                    VK_DOWN => Some(look::Key::Down),
                    VK_RETURN => Some(look::Key::Open),
                    VK_C if ctrl && !shift => Some(look::Key::Copy),
                    VK_BACK if look::inside() => Some(look::Key::Back),
                    _ => None,
                };
                if let Some(handed) = handed {
                    return look::key(handed);
                }
            }
            match key {
                VK_ESCAPE => return look::close(),
                VK_UP | VK_LEFT => return self.look_step(false),
                VK_DOWN | VK_RIGHT => return self.look_step(true),
                VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN | VK_RWIN | VK_RETURN => {}
                VK_C if ctrl => {}
                _ => look::close(),
            }
        }
        let rows = self.rows();
        let page = ((LIST_MOST / ROW) as usize).max(1);
        match key {
            VK_ESCAPE => return self.close(true),
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT if rows > 0 => {
                self.pick = match key {
                    VK_UP => self.pick.saturating_sub(1),
                    VK_DOWN => (self.pick + 1).min(rows - 1),
                    VK_PRIOR => self.pick.saturating_sub(page),
                    _ => (self.pick + page).min(rows - 1),
                };
                self.follow = true;
            }
            VK_TAB => {
                let at = View::EVERY.iter().position(|view| *view == self.view).unwrap_or(0);
                let n = View::EVERY.len();
                return self.choose(View::EVERY[if shift { (at + n - 1) % n } else { (at + 1) % n }]);
            }
            VK_RETURN => {
                return self.act(match (ctrl, shift) {
                    (true, true) => Act::Admin,
                    (true, false) => Act::Show,
                    _ => Act::Open,
                });
            }
            VK_C if ctrl && shift => return self.act(Act::CopyPath),
            // (Copied with nothing of the field selected: the name picked.)
            VK_C if ctrl && self.typing.selection().is_empty() => return self.act(Act::Copy),
            _ => return self.type_key(key, shift, ctrl),
        }
        self.draw();
    }

    /// A key for the field's words: the caret moved, words taken away, copied or pasted.
    fn type_key(&mut self, key: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        let before = self.typing.text.clone();
        let typing = &mut self.typing;
        match key {
            VK_LEFT | VK_RIGHT => {
                let forward = key == VK_RIGHT;
                let selection = typing.selection();
                let to = if !shift && !selection.is_empty() { if forward { selection.end } else { selection.start } } else { typing.step(typing.caret, forward) };
                typing.go(to, shift);
            }
            VK_HOME => typing.go(0, shift),
            VK_END => typing.go(typing.text.len(), shift),
            VK_BACK | VK_DELETE => typing.erase(key == VK_DELETE),
            VK_A if ctrl => {
                typing.anchor = 0;
                typing.caret = typing.text.len();
            }
            VK_C | VK_X if ctrl => {
                let selection = typing.selection();
                if !selection.is_empty() {
                    let text = String::from_utf16_lossy(&typing.text[selection]);
                    run::ask(Ask::CopyText(text, None));
                    if key == VK_X {
                        self.typing.put(&[]);
                    }
                }
            }
            VK_V if ctrl => {
                // (One line of what was pasted: a name is looked for, not lines.)
                let pasted = crate::clipboard::text(self.hwnd).unwrap_or_default();
                let line: Vec<u16> = pasted.lines().next().unwrap_or_default().encode_utf16().collect();
                self.typing.put(&line);
            }
            _ => return,
        }
        self.caret_on = true;
        if self.typing.text != before {
            self.expanded = false;
            self.ask(true);
        }
        self.draw();
    }

    /// A character typed (or given by the input method).
    fn char(&mut self, unit: u16) {
        if unit < 0x20 || unit == 0x7F {
            return;
        }
        if self.looking() {
            look::close();
        }
        self.typing.put(&[unit]);
        self.caret_on = true;
        self.expanded = false;
        self.ask(true);
        self.draw();
    }

    // ----- Looked at larger -----

    /// Whether the name picked is looked at larger (see `ui::look`).
    fn looking(&self) -> bool {
        look::up_for(self.hwnd)
    }

    /// Shown, in front, and the first name in it that may be looked at larger picked and looked at; none found yet, the
    /// first that is, as it is.
    fn preview_first(&mut self) {
        if !self.visible() {
            // (Showing, it looks again afresh, and what is found afresh puts away what was looked at: the first of
            // what it finds looked at, once found.)
            self.show();
            self.preview_owed = true;
            return;
        }
        self.come_forward();
        let lang = self.lang;
        let Some(first) = self.shown.rows().position(|row| looked_at(row, lang).is_some()) else {
            self.preview_owed = true;
            return;
        };
        self.preview_owed = false;
        self.pick = first;
        self.follow = true;
        if !self.looking() {
            self.preview();
        }
        self.draw();
    }

    /// The name picked looked at larger beside the sheet, the others that may be had by the arrows; looked at, gone back
    /// into its row. (A program, a page of Settings: nothing more to show, nothing done.)
    fn preview(&mut self) {
        match previewing(self.looking(), self.shown_for == self.asked, self.look_found) {
            Previewing::Close => look::close(),
            Previewing::WhenFound => self.look_found = true,
            Previewing::Not => self.look_found = false,
            Previewing::Open => self.look_at_picked(),
        }
    }

    /// The name picked looked at larger (see `preview`).
    fn look_at_picked(&mut self) {
        let lang = self.lang;
        let on_screen = self.rows_on_screen();
        let mut looked = Vec::new();
        let mut items = Vec::new();
        for (at, row) in self.shown.rows().enumerate() {
            if let Some(mut item) = looked_at(row, lang) {
                item.from = on_screen.iter().find(|(shown, _)| *shown == at).map(|(_, rect)| *rect);
                looked.push(at);
                items.push(item);
            }
        }
        let Some(at) = looked.iter().position(|row| *row == self.pick) else { return };
        self.looked = looked;
        look::open(look::Opening { items, at, owner: self.hwnd.0 as isize, beside: Some(self.sheet_on_screen()), skin: self.skin, dark: self.dark, frost: self.frost, tell: Some(LOOKED) });
    }

    /// The name at `row` among those shown picked, and looked at larger where it may be (else the look goes).
    fn look_to(&mut self, row: usize) {
        self.pick = row;
        self.follow = true;
        match self.looked.contains(&row).then(|| self.shown.rows().nth(row)).flatten() {
            Some(picked) => look::show(key_of(&picked.what)),
            None => look::close(),
        }
        self.draw();
    }

    /// The next name that may be looked at larger (`forward`), or the one before, looked at.
    fn look_step(&mut self, forward: bool) {
        let Some(at) = self.looked.iter().position(|row| *row == self.pick) else { return };
        let to = if forward { at + 1 } else { at.wrapping_sub(1) };
        if let Some(&row) = self.looked.get(to) {
            self.look_to(row);
        }
    }

    /// The look told which name it shows now (`at`, in its order), or that it went (`usize::MAX`).
    fn looked_told(&mut self, at: usize) {
        if let Some(&row) = self.looked.get(at) {
            if row != self.pick {
                self.pick = row;
                self.follow = true;
            }
        }
        self.draw();
    }

    /// Each name as much as it shows of the list, by its place among those shown, and where that is on the screen (px).
    fn rows_on_screen(&self) -> Vec<(usize, RECT)> {
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let (left, top) = (self.rect.left + px(self.room), self.rect.top + px(self.room));
        self.hits
            .iter()
            .filter_map(|(r, hit)| match hit {
                Hit::Row(at) => Some((*at, RECT { left: left + px(r.x), top: top + px(r.y), right: left + px(r.x + r.w), bottom: top + px(r.y + r.h) })),
                _ => None,
            })
            .collect()
    }

    // ----- What is done -----

    /// `act` done with the name picked; the window gone after it opened something, or showed it.
    fn act(&mut self, act: Act) {
        let Some(row) = self.picked().cloned() else { return };
        let lang = self.lang;
        let opens = matches!(act, Act::Open | Act::Admin | Act::Show);
        // (What opens it comes to the front, the window gone meanwhile.)
        if opens {
            let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        }
        match (&row.what, act) {
            (What::Program(target), Act::Open) => run::ask(Ask::Launch(target.clone())),
            (What::Program(target), Act::Admin) => shell("runas", target, ""),
            (What::Page(at), Act::Open | Act::Admin) => match PAGES[*at].opens {
                Opens::Settings(uri) => shell("open", uri, ""),
                Opens::Tool(program, given) => shell(if act == Act::Admin { "runas" } else { "open" }, program, given),
            },
            (What::Path(path), Act::Open) => run::ask(Ask::Files(vec![path.clone()], FilesAct::Open)),
            (What::Path(path), Act::Admin) if runs_as_admin(&row) => shell("runas", &path.display().to_string(), ""),
            (What::Path(path), Act::Show) => run::ask(Ask::Files(vec![path.clone()], FilesAct::Show)),
            (What::Path(path), Act::Copy) if row.kind != Kind::Folder => {
                run::ask(Ask::Files(vec![path.clone()], FilesAct::Copy(None)));
                self.tell(lang.pick("已复制文件", "File copied"));
            }
            (What::Path(path), Act::CopyPath | Act::Copy) => {
                run::ask(Ask::CopyText(path.display().to_string(), None));
                self.tell(lang.pick("已复制路径", "Path copied"));
            }
            (What::Note(name), Act::Open) => {
                let by = self.sheet_on_screen();
                let (skin, dark) = (self.skin, self.dark);
                self.close(false);
                crate::ui::beside::open(crate::ui::beside::Opening { page: crate::ui::beside::Page::Note(Some(name.clone())), by, side: crate::ui::beside::Side::Below, skin, dark, folder: None, owner: 0 });
                return;
            }
            (What::Todo(text), Act::Open) => {
                crate::todo::add(text, None);
                self.close(false);
                return;
            }
            (What::Item(id), Act::Open) => run::ask(Ask::Open(*id)),
            (What::Item(id), Act::Show) => run::ask(Ask::Show(*id)),
            (What::Item(id), Act::Copy) => {
                run::ask(Ask::Copy(*id));
                self.tell(lang.pick("已复制", "Copied"));
            }
            _ => return,
        }
        if opens {
            self.close(false);
        }
    }

    /// `said` in the foot a moment.
    fn tell(&mut self, said: &str) {
        self.told = Some((said.into(), Instant::now()));
        self.draw();
    }

    /// Where the sheet is on the screen (px).
    fn sheet_on_screen(&self) -> RECT {
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let (left, top) = (self.rect.left + px(self.room), self.rect.top + px(self.room));
        RECT { left, top, right: left + px(WIDTH), bottom: top + px(self.height) }
    }

    /// A look at what may have changed while it shows: a picture made for a row, the index grown or done, a word in
    /// the foot gone.
    fn watch(&mut self) {
        let pictures = crate::station::thumbs::changes();
        let changed = self.waiting && pictures != self.pictures;
        self.pictures = pictures;
        let building = crate::search::standing().iter().any(|standing| !standing.ready);
        let told = self.told.as_ref().is_some_and(|(_, at)| at.elapsed() >= TOLD_FOR && at.elapsed() < TOLD_FOR + std::time::Duration::from_millis(WATCH_EVERY as u64 * 2));
        if changed || building || told {
            self.draw();
        }
    }
}

/// The colours what is on the sheet is drawn in, besides the sheet's own.
struct Inks {
    text: Color,
    text2: Color,
    text3: Color,
    hover: Color,
    pick: Color,
    mark: Color,
    accent: Color,
    link: Color,
    selection: Color,
}

impl Inks {
    fn of(finder: &Finder, c: &Colours) -> Inks {
        let theme = Theme::new(finder.skin, finder.dark);
        let dark = finder.dark;
        let accent = theme::accent(dark);
        match finder.skin {
            // Chart paper's in its own ink: what was met underlined.
            Skin::Paper => Inks {
                text: c.text,
                text2: theme.text2,
                text3: theme.text3,
                hover: theme.hover,
                pick: theme.hover,
                mark: c.text,
                accent: c.text,
                link: c.text,
                selection: c.text.alpha(0.2),
            },
            _ => Inks {
                text: c.text,
                text2: theme.text2,
                text3: theme.text3,
                hover: c.highlight,
                pick: if dark { Color::hex(0xFFFFFF, 0.08) } else { Color::hex(0x000000, 0.055) },
                mark: accent.alpha(if dark { 0.3 } else { 0.22 }),
                accent,
                link: accent,
                selection: accent.alpha(0.35),
            },
        }
    }
}

/// Whether what `row` names can be run as an administrator: a program, a tool, a file that runs.
fn runs_as_admin(row: &Row) -> bool {
    match &row.what {
        What::Program(_) => true,
        What::Page(at) => PAGES[*at].tool(),
        What::Path(path) if row.kind != Kind::Folder => {
            let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
            matches!(extension.as_str(), "exe" | "bat" | "cmd" | "msi" | "lnk")
        }
        _ => false,
    }
}

/// `file` (with `given`) opened by the shell as `verb` has it.
fn shell(verb: &str, file: &str, given: &str) {
    let given = (!given.is_empty()).then(|| HSTRING::from(given));
    unsafe {
        ShellExecuteW(None, &HSTRING::from(verb), &HSTRING::from(file), given.as_ref().map_or(PCWSTR::null(), |given| PCWSTR(given.as_ptr())), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// How the drives' index stands, said: being made (and how far), or kept current; and whether it is being made.
fn standing_said(lang: Lang) -> (String, bool) {
    let standing = crate::search::standing();
    let names: usize = standing.iter().map(|standing| standing.names).sum();
    let building = standing.is_empty() || standing.iter().any(|standing| !standing.ready);
    if building {
        let said = match lang {
            Lang::Zh => format!("正在建立索引，已收录 {} 项，现在即可搜索", count_said(names)),
            Lang::En => format!("Indexing: {} so far, search now", count_said(names)),
        };
        return (said, true);
    }
    let roots = standing.iter().map(|standing| standing.root.as_str()).collect::<Vec<_>>();
    (ready_said(lang, &roots, names), false)
}

/// What the foot says of the drives indexed once all are ready: their names as Windows writes them (C: D:, each root
/// without its backslash, its colon its own), and how many entries they hold.
fn ready_said(lang: Lang, roots: &[&str], names: usize) -> String {
    let drives = roots.iter().map(|root| root.trim_end_matches('\\')).collect::<Vec<_>>().join(" ");
    match lang {
        Lang::Zh => format!("{drives} 已索引 {} 项 · 实时更新", count_said(names)),
        Lang::En => format!("{} indexed on {drives}, kept current", count_said(names)),
    }
}

/// A count with its thousands set apart: 2,914,305.
fn count_said(count: usize) -> String {
    let digits = count.to_string();
    let mut said = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            said.push(',');
        }
        said.push(digit);
    }
    said
}

/// When something changed, said as Explorer would: today's and yesterday's by their time, this year's by their day,
/// others with their year.
fn when_said(time: SystemTime, lang: Lang) -> String {
    let (Some(then), Some(now)) = (local(time), local(SystemTime::now())) else { return String::new() };
    let day = |t: &windows::Win32::Foundation::SYSTEMTIME| days(t.wYear as i64, t.wMonth as i64, t.wDay as i64);
    let clock = format!("{:02}:{:02}", then.wHour, then.wMinute);
    match (day(&now) - day(&then), lang) {
        (0, Lang::Zh) => format!("今天 {clock}"),
        (0, Lang::En) => format!("Today {clock}"),
        (1, Lang::Zh) => format!("昨天 {clock}"),
        (1, Lang::En) => format!("Yesterday {clock}"),
        (_, Lang::Zh) if then.wYear == now.wYear => format!("{}月{}日", then.wMonth, then.wDay),
        (_, Lang::Zh) => format!("{}年{}月{}日", then.wYear, then.wMonth, then.wDay),
        (_, Lang::En) => {
            let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(then.wMonth as usize).clamp(1, 12) - 1];
            if then.wYear == now.wYear { format!("{month} {}", then.wDay) } else { format!("{month} {}, {}", then.wDay, then.wYear) }
        }
    }
}

/// `time` as the clock here reads it.
fn local(time: SystemTime) -> Option<windows::Win32::Foundation::SYSTEMTIME> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // (A file time counts hundreds of nanoseconds from 1601.)
    let since = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let ticks = (since.as_nanos() / 100) as u64 + 116_444_736_000_000_000;
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        FileTimeToSystemTime(&file, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

/// The day `year`-`month`-`day` counted from 1970-01-01.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_era_days - 719_468
}

/// Whether `key`, with the modifiers held as `held` tells, is the preview's shortcut (see `settings::Shortcut::preview`).
fn preview_key(key: VIRTUAL_KEY, held: impl Fn(VIRTUAL_KEY) -> bool) -> bool {
    let Some(shortcut) = crate::app().settings.lock().unwrap().preview_shortcut else { return false };
    shortcut.key == key.0 && shortcut.ctrl == held(VK_CONTROL) && shortcut.alt == held(VK_MENU) && shortcut.shift == held(VK_SHIFT) && shortcut.win == (held(VK_LWIN) || held(VK_RWIN))
}

/// What a name found is known by to the look at it (see `ui::look::Item::key`).
fn key_of(what: &What) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    what.hash(&mut hasher);
    hasher.finish()
}

/// What `row` is looked at larger as (see `ui::look`): a file or folder, a thing on the station or copied lately, a note.
/// None: a program, a page of Settings (nothing more to show of them).
fn looked_at(row: &Row, lang: Lang) -> Option<look::Item> {
    let key = key_of(&row.what);
    match &row.what {
        What::Path(path) => Some(look::Item {
            key,
            title: row.name.clone(),
            about: row.detail.clone(),
            shows: look::Shows::File(path.clone()),
            from: None,
            open: Some(Ask::Files(vec![path.clone()], FilesAct::Open)),
            copy: Some(if row.kind == Kind::Folder { Ask::CopyText(path.display().to_string(), None) } else { Ask::Files(vec![path.clone()], FilesAct::Copy(None)) }),
            pastes: false,
        }),
        What::Item(id) => {
            let hit = if row.kind == Kind::Copied { crate::ui::view::Hit::Copied(*id) } else { crate::ui::view::Hit::Shelf(*id) };
            crate::ui::view::looked_at(hit, None, lang).map(|item| look::Item { key, ..item })
        }
        What::Note(name) => {
            let notes = crate::notes::shown();
            let note = notes.iter().find(|note| note.name == *name)?;
            Some(look::Item {
                key,
                title: row.name.clone(),
                about: row.detail.clone(),
                shows: look::Shows::Words(note.text.clone()),
                from: None,
                open: None,
                copy: Some(Ask::CopyText(note.text.clone(), None)),
                pastes: false,
            })
        }
        What::Program(_) | What::Page(_) | What::Todo(_) => None,
    }
}

fn point(lparam: LPARAM) -> (i32, i32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) } < 0;
    // A point in the window (px) on the screen.
    let on_screen = |lparam: LPARAM| {
        let (x, y) = point(lparam);
        let mut at = POINT { x, y };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut at);
        }
        at
    };
    match message {
        TOGGLE => {
            with(Finder::toggle);
            LRESULT(0)
        }
        PREVIEW => {
            with(Finder::preview_first);
            LRESULT(0)
        }
        FOUND => {
            with(Finder::found);
            LRESULT(0)
        }
        TOLD => {
            with(Finder::told);
            LRESULT(0)
        }
        LOOKED => {
            with(|finder| finder.looked_told(wparam.0));
            LRESULT(0)
        }
        // The preview's shortcut (see `ui::look`): the name picked looked at larger, or the look gone. (What it would
        // type taken with it: its Ctrl's character, its Alt's beep.)
        WM_KEYDOWN | WM_SYSKEYDOWN if preview_key(VIRTUAL_KEY(wparam.0 as u16), held) => {
            let mut typed = MSG::default();
            let _ = unsafe { PeekMessageW(&mut typed, Some(hwnd), WM_CHAR, WM_SYSCHAR, PM_REMOVE) };
            with(Finder::preview);
            LRESULT(0)
        }
        // Space while its look shows what plays: played or paused (the space it would type taken with it).
        WM_KEYDOWN if VIRTUAL_KEY(wparam.0 as u16) == VK_SPACE && look::plays() && with(|finder| finder.looking()).unwrap_or(false) => {
            let mut typed = MSG::default();
            let _ = unsafe { PeekMessageW(&mut typed, Some(hwnd), WM_CHAR, WM_CHAR, PM_REMOVE) };
            // (Once as it goes down, not again as it is held.)
            if lparam.0 & (1 << 30) == 0 {
                look::key(look::Key::Toggle);
            }
            LRESULT(0)
        }
        LISTED => {
            with(|finder| {
                if finder.visible() {
                    finder.ask(false);
                }
            });
            LRESULT(0)
        }
        // Left: gone, unless it was its own menu that came to the front (or it came back meanwhile).
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                let _ = unsafe { PostMessageW(Some(hwnd), LEFT, WPARAM(0), LPARAM(0)) };
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        LEFT => {
            with(|finder| {
                if finder.menu.is_none() && finder.visible() && unsafe { GetForegroundWindow() } != hwnd {
                    finder.close(false);
                }
            });
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            with(Finder::draw);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let over_words = with(|finder| finder.hovered() == Some(Hit::Field)).unwrap_or(false);
            unsafe { SetCursor(LoadCursorW(None, if over_words { IDC_IBEAM } else { IDC_ARROW }).ok()) };
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let at = on_screen(lparam);
            with(|finder| finder.moved(at));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let at = on_screen(lparam);
            with(|finder| finder.press(at, held(VK_SHIFT)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let at = on_screen(lparam);
            with(|finder| finder.release(at, held(VK_CONTROL)));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let at = on_screen(lparam);
            with(|finder| finder.context(at));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let turns = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with(|finder| finder.wheel(turns));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            with(|finder| finder.key(VIRTUAL_KEY(wparam.0 as u16), held(VK_SHIFT), held(VK_CONTROL)));
            LRESULT(0)
        }
        WM_CHAR => {
            with(|finder| finder.char(wparam.0 as u16));
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                CARET => with(|finder| {
                    finder.caret_on = !finder.caret_on;
                    finder.draw();
                }),
                SLIDING => with(Finder::draw),
                _ => with(Finder::watch),
            };
            LRESULT(0)
        }
        WM_IME_STARTCOMPOSITION => {
            with(|finder| {
                finder.composing = true;
                finder.place_ime();
                finder.draw();
            });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_IME_ENDCOMPOSITION => {
            with(|finder| {
                finder.composing = false;
                finder.draw();
            });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // (Placed at its screen's scale as it is shown: nothing more to do as it is put there.)
        WM_DPICHANGED => LRESULT(0),
        // Alt+F4: hidden, as Esc does.
        WM_CLOSE => {
            with(|finder| finder.close(true));
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drives are named as Windows writes them, each with its one colon, in both languages: the English foot put a
    /// colon after them all, and the last drive's doubled ("C: D:: 1,284,503 indexed").
    #[test]
    fn the_drives_indexed_are_named_with_one_colon_each() {
        assert_eq!(ready_said(Lang::En, &["C:\\", "D:\\"], 1_284_503), "1,284,503 indexed on C: D:, kept current");
        assert_eq!(ready_said(Lang::Zh, &["C:\\", "D:\\"], 1_284_503), "C: D: 已索引 1,284,503 项 · 实时更新");
        assert!(!ready_said(Lang::En, &["C:\\"], 7).contains("::"));
    }

    /// What is told of the files found for the last asking, before what was found for it is shown (an older telling
    /// taking it in first), is kept till it is, and taken in then; what is told of an asking before is let go of.
    #[test]
    fn what_is_told_before_its_files_show_waits_for_them() {
        let row = |path: &str| Line::Row(Row { kind: Kind::File(FileKind::Document), name: path.into(), marks: Vec::new(), detail: String::new(), what: What::Path(path.into()), rank: 0.0, bytes: None, changed: None }, false);
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut shown = Shown { lines: vec![row("D:\\old.txt")], total: 1 };
        let mut told = vec![(1, PathBuf::from("D:\\old.txt"), (1, when)), (2, PathBuf::from("D:\\new.txt"), (2, when))];
        // Asked again (2), its files not shown yet: what is told of them waits, the asking before's let go of.
        assert!(!take_told(&mut told, &mut shown, 1, 2));
        assert_eq!(told.len(), 1);
        shown = Shown { lines: vec![row("D:\\new.txt")], total: 1 };
        assert!(take_told(&mut told, &mut shown, 2, 2));
        assert!(told.is_empty());
        assert_eq!(shown.rows().next().map(|row| (row.bytes, row.changed)), Some((Some(2), Some(when))));
    }

    /// The preview's shortcut pressed as what is typed is still looked for looks at what it finds, not at what is shown
    /// of what was typed before; pressed again meanwhile, at nothing.
    #[test]
    fn what_is_looked_at_waits_for_what_is_typed_to_be_found() {
        assert_eq!(previewing(false, false, false), Previewing::WhenFound);
        assert_eq!(previewing(false, false, true), Previewing::Not);
        assert_eq!(previewing(false, true, false), Previewing::Open);
        assert_eq!(previewing(false, true, true), Previewing::Open);
        assert_eq!(previewing(true, false, false), Previewing::Close);
        assert_eq!(previewing(true, true, false), Previewing::Close);
    }

    #[test]
    fn counts_are_said_with_their_thousands_apart() {
        assert_eq!(count_said(0), "0");
        assert_eq!(count_said(999), "999");
        assert_eq!(count_said(1000), "1,000");
        assert_eq!(count_said(2_914_305), "2,914,305");
    }

    #[test]
    fn days_are_counted_from_1970() {
        assert_eq!(days(1970, 1, 1), 0);
        assert_eq!(days(1970, 3, 1), 59);
        assert_eq!(days(2000, 3, 1), 11_017);
        assert_eq!(days(2026, 10, 27) - days(2026, 10, 26), 1);
        assert_eq!(days(2026, 1, 1) - days(2025, 12, 31), 1);
    }

    /// The sheet in each skin, light and dark, something typed and found (`cargo test --profile fast --features studio
    /// --lib -- --ignored search_looks` writes `target/looks/search.png`).
    #[cfg(feature = "studio")]
    #[test]
    #[ignore = "writes a picture to look at"]
    fn search_looks() {
        use crate::station::recent::Kind as FileKind;
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = gfx::for_main_screen().unwrap();
        let (asks, _asked) = std::sync::mpsc::channel();
        let now = SystemTime::now();
        let row = |kind: Kind, name: &str, marks: Vec<(usize, usize)>, detail: &str, what: What, bytes: Option<u64>, ago: Option<u64>| Row {
            kind,
            name: name.into(),
            marks,
            detail: detail.into(),
            what,
            rank: 0.0,
            bytes,
            changed: ago.map(|ago| now - std::time::Duration::from_secs(ago)),
        };
        let path = |p: &str| What::Path(std::path::PathBuf::from(p));
        let lines = vec![
            Line::Row(row(Kind::Program, "微信", vec![(0, 1)], "应用", What::Program("Tencent.WeChat".into()), None, None), true),
            Line::Head(Group::Programs, More::No),
            Line::Row(row(Kind::Program, "微云", vec![(0, 1)], "应用", What::Program("Tencent.Weiyun".into()), None, None), false),
            Line::Head(Group::Folders, More::Others(12)),
            Line::Row(row(Kind::Folder, "Weiyi", vec![(0, 3)], r"C:\Users", path(r"C:\Users\Weiyi"), None, Some(86_400 * 3)), false),
            Line::Row(row(Kind::Folder, "WeChat Files", vec![(0, 2)], r"D:\Documents", path(r"D:\Documents\WeChat Files"), None, Some(600)), false),
            Line::Head(Group::Files, More::ShowAll(248)),
            Line::Row(row(Kind::File(FileKind::Document), "weixin_export.csv", vec![(0, 3)], r"D:\Documents\Exports", path(r"D:\Documents\Exports\weixin_export.csv"), Some(48_213), Some(3_600)), false),
            Line::Row(row(Kind::File(FileKind::Screenshot), "微信截图_20261027141205.png", vec![(0, 1)], r"D:\Pictures\Screenshots", path(r"D:\Pictures\Screenshots\a.png"), Some(1_873_221), Some(86_400 + 30)), false),
            Line::Head(Group::Glance, More::No),
            Line::Row(row(Kind::Note, "周会纪要 微调方案", vec![(5, 6)], "便签", What::Note("a.md".into()), None, None), false),
        ];
        let looks = [(Skin::Fluent, false), (Skin::Glass, false), (Skin::Paper, false), (Skin::Fluent, true), (Skin::Glass, true), (Skin::Paper, true)];
        let room = theme::room(&Theme::new(Skin::Glass, false).shadows).max(SHADOW);
        let mut finders: Vec<Finder> = looks
            .iter()
            .map(|&(skin, dark)| {
                let mut finder = Finder::new(HWND::default(), gfx.clone(), asks.clone());
                (finder.skin, finder.dark, finder.lang, finder.room) = (skin, dark, Lang::Zh, room);
                finder.typing.put(&"wei".encode_utf16().collect::<Vec<_>>());
                finder.shown = Shown { lines: lines.clone(), total: 312 };
                finder.pick = 0;
                finder.height = finder.sheet_height();
                finder
            })
            .collect();
        let (cell_w, cell_h, px) = (WIDTH + 2.0 * room, finders[0].height + 2.0 * room, 1.5f32);
        let (w, h) = ((3.0 * cell_w * px) as u32, (2.0 * cell_h * px) as u32);
        let pixels = gfx
            .draw_offscreen((w, h), px, |frame| {
                frame.fill(Color::hex(0x7C93B4, 1.0), 0.0, 0.0, 3.0 * cell_w, cell_h);
                frame.fill(Color::hex(0x1B2433, 1.0), 0.0, cell_h, 3.0 * cell_w, cell_h);
                for (at, finder) in finders.iter_mut().enumerate() {
                    let frosted = finder.skin != Skin::Paper;
                    finder.paint(frame, frosted, ((at % 3) as f32 * cell_w + room, (at / 3) as f32 * cell_h + room));
                }
            })
            .expect("drawn");
        let out = std::path::PathBuf::from(format!("{}/target/looks", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        crate::studio::save_png(&out.join("search.png"), w, h, &pixels).unwrap();
    }

    /// The search's window is as tall as its tallest sheet, above all: it takes the pointer only where its sheet (and
    /// the room for its shadow round it) is drawn, not over the screen under a short sheet, nor anywhere before it is
    /// first drawn (see `gfx::Footprint`).
    #[test]
    fn the_search_takes_the_pointer_only_where_its_sheet_is() {
        use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, GetAncestor, WindowFromPoint, GA_ROOT};
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = gfx::for_main_screen().unwrap();
        let hwnd = unsafe { make_window() }.expect("a window");
        let (asks, _asked) = std::sync::mpsc::channel();
        let mut finder = Finder::new(hwnd, gfx, asks);
        (finder.skin, finder.scale, finder.room) = (Skin::Paper, 1.0, 20.0);
        finder.rect = RECT { left: 100, top: 100, right: 100 + (WIDTH + 40.0) as i32, bottom: 800 };
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), finder.rect.left, finder.rect.top, finder.rect.right - finder.rect.left, finder.rect.bottom - finder.rect.top, SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE);
        }
        let over = |x: i32, y: i32| unsafe { GetAncestor(WindowFromPoint(POINT { x: 100 + x, y: 100 + y }), GA_ROOT) } == hwnd;
        assert!(!over(200, 60), "it takes the pointer before its sheet is drawn");
        finder.draw();
        let tall = (finder.height + 2.0 * finder.room) as i32;
        assert!(tall < 600, "the sheet with nothing found is short ({tall} px)");
        assert!(over(200, 60), "it takes no pointer over its sheet");
        assert!(!over(200, tall + 40), "it takes the pointer under its sheet");
        drop(finder);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
