//! A look at a thing, larger: what is under the pointer on a station (the panel's, a widget's: put on it, copied
//! lately, come lately), or picked in search, called up by the preview's shortcut. It grows out of its tile to a
//! sheet beside what it was called up from, drawn in the skin the panel is (its glass frosted over the screen,
//! Windows 11's acrylic, chart paper), light or dark as the panel is: a picture whole (zoomed by the wheel about the
//! pointer, moved by a drag, between fitted and its own pixels by a double click), words all of them, a document or
//! a video as Explorer's preview pane shows it (live, by its kind's previewer: see `pane`), else as Explorer pictures
//! it, else its icon and what it is. Files together show as themselves, each its own picture in rows: one clicked
//! grows out of its place into a look of its own, the others of them had by the arrows, and shrinks back into its place
//! (its back button, Backspace). It goes back into its tile as the shortcut or Esc is pressed again, or as anything but
//! it and what it was called up from is clicked.
//!
//! It never takes the keyboard: what called it up keeps it, and hands it the keys that are its own while it is up
//! (see `key`): the arrows to the thing beside (another is had by them, or by a click on its tile, never by the
//! pointer passing over one), Enter to open it, Ctrl+C to copy it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_LAYER_PARAMETERS1;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetAncestor, GetCursorPos, GetMessageW, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW, RegisterClassW, SetCursor,
    SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WindowFromPoint, ASFW_ANY, CS_DBLCLKS, GA_ROOT, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_HAND, MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SW_HIDE,
    SW_SHOWNOACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_numerics::Matrix3x2;

use super::canvas::{Align, Canvas, Color, Family, Fill, Font, Picture, Point};
use super::gfx::{self, Frame, Gfx, Surface};
use super::icons::Icon;
use super::menu::{Colours, Look};
use super::motion::Easing;
use super::text::Lang;
use super::theme::{self, Skin, Theme};
use super::view::Rect;
use crate::station::run::Ask;

mod pane;
mod play;

/// One thing to look at.
#[derive(Clone, Debug)]
pub struct Item {
    /// What it is known by to what called it up (see `shown`, `tiles`).
    pub key: u64,
    /// Its name, and what it is, as its tile says them (a file's said more fully once it is read).
    pub title: String,
    pub about: String,
    pub shows: Shows,
    /// Its tile on the screen (physical px): where it grows out of and goes back into. None: not shown now.
    pub from: Option<RECT>,
    /// What its main button and Enter do (none: no main button), and what its copy does.
    pub open: Option<Ask>,
    pub copy: Option<Ask>,
    /// Its main button pastes it where the keyboard is (words on the station), rather than opening it.
    pub pastes: bool,
}

/// What is looked at: words as they are (text on the station, a web address, a note); words kept in a file (text on
/// the station too long to keep in its list, in a file named by its number, of no kind its name tells); a file (or a
/// picture the station keeps); files together.
#[derive(Clone, Debug, PartialEq)]
pub enum Shows {
    Words(String),
    Kept(PathBuf),
    File(PathBuf),
    Files(Vec<PathBuf>),
}

/// A look asked for: at `items[at]`, the others had by the arrows, called up from the window `owner` (whose clicks are
/// its own while it is up: see `watch`), beside `beside` (physical px: what it was called up from, the panel; none, in
/// the middle of the screen it is on), in `skin`, light or `dark`, its glass `frost` frosted. Its showing another,
/// and its going, told to `owner` as `tell` (WPARAM the place of the one shown, or `usize::MAX` as it goes).
#[derive(Clone, Debug)]
pub struct Opening {
    pub items: Vec<Item>,
    pub at: usize,
    pub owner: isize,
    pub beside: Option<RECT>,
    pub skin: Skin,
    pub dark: bool,
    pub frost: f32,
    pub tell: Option<u32>,
}

/// A key of its own, handed to it by what called it up: Esc (and its shortcut), the arrows, Enter, Ctrl+C; Backspace
/// while it shows a file of a group by itself (see `inside`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Close,
    Left,
    Right,
    Up,
    Down,
    Open,
    Copy,
    Back,
    /// Space while it shows what plays (see `plays`): played or paused.
    Toggle,
}

/// What is asked of its thread.
enum Asked {
    Open(Opening),
    Close,
    Key(Key),
    /// The thing known by this key shown (its tile clicked).
    Show(u64),
    /// The thing known by this key shown, among these as they show now (its tile clicked: one scrolled to since, of
    /// another list, as well as one there was).
    Among(Vec<Item>, u64),
    /// Where the tiles are now (physical px), by their keys: those of the window `owner`.
    Tiles(isize, Vec<(u64, RECT)>),
}

static ASKS: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
static WINDOW: OnceLock<Option<isize>> = OnceLock::new();
/// The window it was called up from while it is up (0: it is not), and the key of what it shows; whether it shows a
/// file of a group by itself.
static OWNER: AtomicIsize = AtomicIsize::new(0);
static SHOWN: AtomicU64 = AtomicU64::new(0);
static INSIDE: AtomicBool = AtomicBool::new(false);
/// Whether what it shows plays (a video, a sound: see `play`).
static PLAYS: AtomicBool = AtomicBool::new(false);
/// Whether its shortcut was wanted but another program had it as it was last taken (see `key_was_taken`).
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Its messages: something asked, something read, a previewer wanted for what it shows (see `pane`).
const ASKED: u32 = WM_APP + 1;
const LOADED: u32 = WM_APP + 2;
const PANE: u32 = WM_APP + 3;
/// What plays told something of itself (see `play::Player`): WPARAM what.
const PLAYED: u32 = WM_APP + 4;
/// Its timers: each frame while it moves; a look at the buttons pressed elsewhere, and at what changed.
const FRAME: usize = 1;
const FRAME_MS: u32 = 10;
const WATCH: usize = 2;
const WATCH_MS: u32 = 30;

/// How it moves: out of its tile (and how long, and how clear it is as it begins, and when it is whole), from one
/// thing to another (and the new thing's fading in), back into its tile (and how clear it is when, as it goes).
const OPEN_FOR: Duration = Duration::from_millis(280);
const OPEN_EASE: Easing = Easing(0.2, 0.9, 0.25, 1.06);
const OPEN_CLEAR: f32 = 0.3;
const OPEN_WHOLE_AT: f32 = 0.55;
const MOVE_FOR: Duration = Duration::from_millis(180);
/// A file of a group grown out of its place to be looked at by itself, or shrunk back into it.
const INTO_FOR: Duration = Duration::from_millis(300);
const MOVE_EASE: Easing = Easing(0.2, 0.8, 0.2, 1.0);
const FADE_IN: Duration = Duration::from_millis(150);
const CLOSE_FOR: Duration = Duration::from_millis(200);
const CLOSE_EASE: Easing = Easing(0.4, 0.0, 0.7, 1.0);
const CLOSE_STILL: (f32, f32) = (0.7, 0.85);

/// Its parts (DIPs): its buttons (the bar, and the room round its parts: see `bar`, `inset`); its least width for a picture; its sizes for
/// words, and for what has no picture; how much of the screen it takes at most, and how far from the screen's edges and
/// what it was called up from it keeps.
/// (Its bar: its name's two lines, `NAME` tall, and `UNDER_NAME` under them; the room round its parts makes its body's
/// corners concentric with its own, see `inset`.)
const NAME: f32 = 36.0;
const UNDER_NAME: f32 = 12.0;
const INSET_LEAST: f32 = 16.0;
const BUTTON: f32 = 30.0;
const PICTURE_LEAST: f32 = 320.0;
const WORDS: (f32, f32) = (560.0, 340.0);
/// Files together: how wide the sheet is, how tall a row of their pictures is wanted, the room between them, and the
/// line under each for its name.
const GROUP_WIDE: f32 = 640.0;
const GROUP_ROW: f32 = 150.0;
const GROUP_GAP: f32 = 10.0;
const GROUP_NAME: f32 = 22.0;
/// A sound: how wide its sheet is, and how large its cover is (its body as tall); in how many parts its loudness is
/// read along it.
const SOUND_WIDE: f32 = 480.0;
const SOUND_COVER: f32 = 140.0;
const LOUDNESS_PARTS: usize = 400;
/// What plays: the strip of its controls over a video (how tall, how far in from the video's edges), how long they
/// show after the pointer last moves over it, and how long they fade; how wide its volume's slider opens, and how
/// long it takes to.
const STRIP: f32 = 38.0;
const STRIP_IN: f32 = 10.0;
const CONTROLS_FOR: Duration = Duration::from_millis(2000);
const CONTROLS_FADE: Duration = Duration::from_millis(250);
const SLIDER: f32 = 64.0;
/// How large the knob on a track or the volume's slider is.
const KNOB: f32 = 10.0;
const SLIDER_FOR: Duration = Duration::from_millis(180);
const OTHER: (f32, f32) = (400.0, 290.0);
const MOST: (f32, f32) = (0.6, 0.82);
const GAP: f32 = 24.0;
/// A page's shape (A4: its height to its width), and the room beside it.
const PAGE: f32 = 1.414;
const PAGE_ROOM: f32 = 40.0;
/// The room round the sheet for its shadow (DIPs).
const SHADOW: f32 = 24.0;
/// A picture zoomed in at most; a turn of the wheel zooms by so much.
const ZOOM_MOST: f32 = 8.0;
const ZOOM_TURN: f32 = 1.2;
/// A picture read at most so large (px): its longer side, and its pixels in all.
const PICTURE_SIDE: u32 = 8192;
const PICTURE_PIXELS: u64 = 24_000_000;
/// Words read of a file at most (bytes), and what is said once they are cut off there.
const WORDS_MOST: usize = 1 << 20;
/// How long "copied" stands on its button.
const TOLD_FOR: Duration = Duration::from_millis(900);

/// The thing `opening` asks for looked at (one up goes to it), on its thread (started when first asked).
pub fn open(opening: Opening) {
    ask(Asked::Open(opening));
}

/// It goes back into its tile.
pub fn close() {
    if is_up() {
        ask(Asked::Close);
    }
}

/// A key of its own pressed (see `Key`), handed to it by what called it up.
pub fn key(key: Key) {
    if is_up() {
        ask(Asked::Key(key));
    }
}

/// The thing known by `key` shown (its tile clicked while it is up).
pub fn show(key: u64) {
    if is_up() {
        ask(Asked::Show(key));
    }
}

/// The thing known by `key` shown, the others had by the arrows now `items` (the things that may be looked at as they
/// show now where it was called up from: its tile clicked while it is up).
pub fn show_among(items: Vec<Item>, key: u64) {
    if is_up() {
        ask(Asked::Among(items, key));
    }
}

/// Where the tiles of the window `owner` are now (physical px), by their keys: as it was drawn again (scrolled, say).
pub fn tiles(owner: HWND, tiles: Vec<(u64, RECT)>) {
    if up_for(owner) {
        ask(Asked::Tiles(owner.0 as isize, tiles));
    }
}

/// Whether it is up.
pub fn is_up() -> bool {
    OWNER.load(Ordering::Acquire) != 0
}

/// Whether it shows a file of a group by itself, up: Backspace is its own then (back to the group).
pub fn inside() -> bool {
    is_up() && INSIDE.load(Ordering::Acquire)
}

/// Whether it shows what plays, up: Space is its own then (played or paused).
pub fn plays() -> bool {
    is_up() && PLAYS.load(Ordering::Acquire)
}

/// The window it was called up from, while it is up.
pub fn owner() -> Option<HWND> {
    let owner = OWNER.load(Ordering::Acquire);
    (owner != 0).then_some(HWND(owner as *mut _))
}

/// Whether it is up, called up from `window`.
pub fn up_for(window: HWND) -> bool {
    let owner = OWNER.load(Ordering::Acquire);
    owner != 0 && owner == window.0 as isize
}

/// The key of the thing it shows, while it is up, called up from `window`.
pub fn shown_for(window: HWND) -> Option<u64> {
    up_for(window).then(|| SHOWN.load(Ordering::Acquire))
}

/// Whether its shortcut was wanted, as it was last taken, but another program had it.
pub fn key_taken() -> bool {
    TAKEN.load(Ordering::Relaxed)
}

/// What is said on a thing that may be looked at larger as the pointer rests on it (see `view::Hint`): its shortcut, and
/// what it does. None while it has none, or another program has it.
pub fn hint_says(lang: Lang) -> Option<String> {
    let shortcut = crate::app().settings.lock().unwrap().preview_shortcut?;
    (!key_taken()).then(|| format!("{} {}", crate::tray::shortcut_name(shortcut), lang.pick("预览", "Preview")))
}

/// Its shortcut, as it was just taken: had, or another program's.
pub fn key_was_taken(taken: bool) {
    TAKEN.store(taken, Ordering::Relaxed);
}

fn ask(asked: Asked) {
    let made = WINDOW.get_or_init(|| {
        let (sender, made) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("glance: look".into()).spawn(move || run(sender)).ok()?;
        made.recv().ok().flatten()
    });
    let Some(hwnd) = *made else { return };
    ASKS.lock().unwrap().push(asked);
    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), ASKED, WPARAM(0), LPARAM(0)) };
}

/// The window's thread: its window made (told to `made`), and its messages run till Glance ends.
fn run(made: Sender<Option<isize>>) {
    // (The shell, which opens what is looked at, expects a single-threaded apartment.)
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let hwnd = unsafe { make_window() };
    let (Some(hwnd), Ok(gfx)) = (hwnd, gfx::current()) else {
        let _ = made.send(None);
        return;
    };
    let (reads, read) = std::sync::mpsc::channel();
    let to = hwnd.0 as isize;
    if std::thread::Builder::new().name("glance: look reading".into()).spawn(move || reading(read, to)).is_err() {
        let _ = made.send(None);
        return;
    }
    LOOKER.with(|looker| *looker.borrow_mut() = Some(Looker::new(hwnd, gfx, reads)));
    let _ = made.send(Some(hwnd.0 as isize));
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn make_window() -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = w!("GlanceLook");
        RegisterClassW(&WNDCLASSW { style: CS_DBLCLKS, lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        // Not layered (the compositor gives a layered window nothing of the screen behind to frost), never taking the
        // keyboard; as large as its screen, its clicks only where the sheet is (see `Looker::draw`).
        let style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_NOACTIVATE;
        let hwnd = CreateWindowExW(style, class, w!("Glance"), WS_POPUP, 0, 0, 1, 1, None, None, Some(module.into()), None).ok()?;
        let disabled = windows::core::BOOL::from(true);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &disabled as *const _ as *const _, size_of::<windows::core::BOOL>() as u32);
        Some(hwnd)
    }
}

thread_local! {
    static LOOKER: RefCell<Option<Looker>> = const { RefCell::new(None) };
}

/// `act` with its state, where it is not in use already (a message sent to the window while it acts).
fn with<R>(act: impl FnOnce(&mut Looker) -> R) -> Option<R> {
    LOOKER.with(|looker| looker.try_borrow_mut().ok().and_then(|mut looker| looker.as_mut().map(act)))
}

// ----- Reading -----

/// What a thing is, as far as its name tells (before it is read): a picture, words, a page (a document), a video,
/// files together, or what is not known till it is read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Picture,
    Words,
    Page,
    Video,
    Sound,
    Files,
    Unknown,
}

/// What a file's name says it is.
fn kind_of(path: &Path) -> Kind {
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "bmp" | "dib" | "tif" | "tiff" | "webp" | "heic" | "heif" | "avif" | "ico" | "jxr" | "wdp" | "dds" => Kind::Picture,
        "pdf" | "doc" | "docx" | "rtf" | "odt" | "xls" | "xlsx" | "csv" | "ods" | "ppt" | "pptx" | "odp" | "xps" | "oxps" | "epub" | "vsdx" | "pub" => Kind::Page,
        "mp4" | "m4v" | "mov" | "mkv" | "avi" | "wmv" | "webm" | "flv" | "ts" | "m2ts" | "mts" | "3gp" | "mpg" | "mpeg" => Kind::Video,
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "wma" | "ogg" | "oga" | "opus" | "aif" | "aiff" | "mka" | "ac3" | "amr" => Kind::Sound,
        _ => Kind::Unknown,
    }
}

/// What a thing is before it is read.
fn kind_shown(shows: &Shows) -> Kind {
    match shows {
        Shows::Words(_) | Shows::Kept(_) => Kind::Words,
        Shows::File(path) => kind_of(path),
        Shows::Files(_) => Kind::Files,
    }
}

/// What a thing's body shows, once read.
#[derive(Clone)]
enum Body {
    /// A picture, and its own size (px) as it is turned to be seen, before it was made no larger than `PICTURE_SIDE`.
    Picture(Picture, (u32, u32)),
    /// Words (in a fixed face: code, a file of words of any kind but plain text), and whether they were cut short.
    Words { text: String, fixed: bool, cut: bool },
    /// A page of a document, a frame of a video, as Explorer pictures it.
    Pictured(Picture),
    /// What its kind's previewer shows, live, over the body while the sheet rests (see `pane`). Meanwhile its picture,
    /// as Explorer makes it, where it has one.
    Live { picture: Option<Picture>, previewer: windows::core::GUID },
    /// Files together: each one, as its picture shows in rows (see `group_cells`).
    Group(Vec<Member>),
    /// A video or a `sound`, played (see `play`): the picture standing for it till it plays (a video's frame, a
    /// sound's cover, as Explorer makes it, where it has one), and a sound's loudness along it (see `play::loudness`),
    /// once read.
    Played { picture: Option<Picture>, sound: bool, loudness: Option<Vec<f32>> },
    /// What has no picture: its icon (made as it shows), what kind of file it is.
    Other(Option<PathBuf>),
}

/// A file of a group: where it is, its name, and how large it is seen (a picture's, a video's: see `measure`), where it
/// is known; else its picture, Explorer's, is drawn in a square.
#[derive(Clone, Debug, PartialEq)]
struct Member {
    path: PathBuf,
    name: String,
    seen: Option<Seen>,
}

/// A thing read: what its body shows, and what it is said to be (a file's: its kind, size, when it was written).
#[derive(Clone)]
struct Read {
    body: Body,
    about: Option<String>,
}

/// What its reading thread is asked: the thing `at` of the look numbered `number` read (or its group's file `member`),
/// in `lang`, its pictures for a screen `scale` px a DIP.
struct Reading {
    number: u64,
    at: usize,
    member: Option<usize>,
    shows: Shows,
    lang: Lang,
    scale: f32,
}

/// What was read, for which look and which thing (and which file of its group).
static READ: Mutex<Vec<ReadFor>> = Mutex::new(Vec::new());
type ReadFor = (u64, usize, Option<usize>, Read);
/// How large a thing is seen (see `measure`), for which look and which thing: known before it is read. (A group's
/// files are measured as the group is read.)
static MEASURED: Mutex<Vec<(u64, usize, Seen)>> = Mutex::new(Vec::new());
/// A thing's size as it is seen (px across, down).
type Seen = (u32, u32);

/// Things read as they are asked for, the latest first (those of a look gone left), each told to the window as it is.
fn reading(asks: Receiver<Reading>, hwnd: isize) {
    // (The shell's picture makers expect a single-threaded apartment.)
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let mut waiting: Vec<Reading> = Vec::new();
    loop {
        if waiting.is_empty() {
            match asks.recv() {
                Ok(asked) => waiting.push(asked),
                Err(_) => return,
            }
        }
        waiting.extend(asks.try_iter());
        // (Only the latest look's, the one asked for last first: the one now shown, before those beside it.)
        let latest = waiting.iter().map(|asked| asked.number).max().unwrap_or(0);
        waiting.retain(|asked| asked.number == latest);
        let Some(asked) = waiting.pop() else { continue };
        // (How large it is seen first, told at once: the sheet grows to that size as it opens, and keeps it as what it
        // shows is read, which may take a while.)
        if let Some(size) = measure(&asked.shows).filter(|_| asked.member.is_none()) {
            MEASURED.lock().unwrap().push((asked.number, asked.at, size));
            let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LOADED, WPARAM(0), LPARAM(0)) };
        }
        let key = asked.number ^ ((asked.at as u64) << 40) ^ asked.member.map_or(0, |member| (member as u64 + 1) << 20);
        let read = read(&asked.shows, asked.lang, asked.scale, key);
        let sound = matches!(read.body, Body::Played { sound: true, .. });
        READ.lock().unwrap().push((asked.number, asked.at, asked.member, read.clone()));
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LOADED, WPARAM(0), LPARAM(0)) };
        // (A sound read through for how loud it is along it, after it shows: told again with that.)
        if let (true, Shows::File(path), Read { body: Body::Played { picture, .. }, about }) = (sound, &asked.shows, read) {
            if let Some(loudness) = play::loudness(path, LOUDNESS_PARTS) {
                let read = Read { body: Body::Played { picture, sound: true, loudness: Some(loudness) }, about };
                READ.lock().unwrap().push((asked.number, asked.at, asked.member, read));
                let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LOADED, WPARAM(0), LPARAM(0)) };
            }
        }
    }
}

/// What was read (or measured) of the things `before` (by their places there) at their places among `items`: none for
/// those new.
fn carried<T>(before: &[Item], mut read: Vec<Option<T>>, items: &[Item]) -> Vec<Option<T>> {
    items.iter().map(|item| before.iter().position(|had| had.key == item.key).and_then(|at| read.get_mut(at)?.take())).collect()
}

/// `shows` read: a picture's pixels, a file's words, a page's or a video's picture as Explorer makes it, else what it
/// is. Its pictures known by `key`, made for a screen `scale` px a DIP.
fn read(shows: &Shows, lang: Lang, scale: f32, key: u64) -> Read {
    match shows {
        Shows::Words(text) => Read { body: Body::Words { text: text.clone(), fixed: false, cut: false }, about: None },
        // (Words, whatever the file's name: plain text, as they were copied.)
        Shows::Kept(path) => match words_of(path) {
            Some((text, cut)) => Read { body: Body::Words { text, fixed: false, cut }, about: None },
            None => Read { body: Body::Other(None), about: Some(lang.pick("文件已不存在", "File no longer exists").into()) },
        },
        // (Each measured, to be laid out in rows at its own shape: see `group_cells`.)
        Shows::Files(paths) => {
            let members = paths
                .iter()
                .map(|path| {
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                    Member { path: path.clone(), name, seen: measure(&Shows::File(path.clone())) }
                })
                .collect();
            Read { body: Body::Group(members), about: None }
        }
        Shows::File(path) => read_file(path, lang, scale, key),
    }
}

fn read_file(path: &Path, lang: Lang, scale: f32, key: u64) -> Read {
    let meta = std::fs::metadata(path).ok();
    let Some(meta) = meta.filter(|meta| meta.is_file()) else {
        let folder = std::fs::metadata(path).is_ok_and(|meta| meta.is_dir());
        let said = if folder { lang.pick("文件夹", "Folder") } else { lang.pick("文件已不存在", "File no longer exists") };
        return Read { body: Body::Other(folder.then(|| path.to_path_buf())), about: Some(said.into()) };
    };
    let kind_name = type_name(path);
    let written = meta.modified().ok().map(|time| super::text::since(ms_of(time), crate::station::now_ms(), lang));
    let about = |pixels: Option<(u32, u32)>| {
        let mut said = Vec::new();
        if let Some((w, h)) = pixels {
            said.push(format!("{w} × {h}"));
        }
        said.extend(kind_name.clone());
        said.push(super::text::bytes(meta.len() as f64));
        said.extend(written.clone());
        Some(said.join(" · "))
    };
    let kind = kind_of(path);
    if kind == Kind::Picture {
        if let Ok((picture, own)) = decode(path, key) {
            return Read { body: Body::Picture(picture, own), about: about(Some(own)) };
        }
    }
    if kind == Kind::Unknown {
        if let Some((text, cut)) = words_of(path) {
            // (Plain text in the face words are written in; anything else, code, in a fixed one.)
            let plain = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt") || extension.eq_ignore_ascii_case("md"));
            return Read { body: Body::Words { text, fixed: !plain, cut }, about: about(None) };
        }
    }
    // (A video or a sound: played, its picture standing for it till it plays, as large as a page may be shown on this
    // screen.)
    let side = (1600.0 * scale.max(1.0)).min(2400.0) as u32;
    if matches!(kind, Kind::Video | Kind::Sound) {
        let picture = crate::station::thumbs::made_now(path, side, crate::station::thumbs::Made::Thumbnail, key);
        return Read { body: Body::Played { picture, sound: kind == Kind::Sound, loudness: None }, about: about(None) };
    }
    // (Its kind's previewer, where it has one, and a picture standing for it meanwhile. Else only the picture, as
    // Explorer pictures it: its first page.)
    let previewer = if kind == Kind::Picture { None } else { pane::previewer(path) };
    let pictured = previewer.is_some() || matches!(kind, Kind::Page | Kind::Picture);
    let picture = if pictured { crate::station::thumbs::made_now(path, side, crate::station::thumbs::Made::Thumbnail, key) } else { None };
    match (previewer, picture) {
        (Some(previewer), picture) => Read { body: Body::Live { picture, previewer }, about: about(None) },
        (None, Some(picture)) => Read { body: Body::Pictured(picture), about: about(None) },
        (None, None) => Read { body: Body::Other(Some(path.to_path_buf())), about: about(None) },
    }
}

/// Milliseconds since 1970 of `time`.
fn ms_of(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// The kind of file `path` is, as Explorer names it ("PNG 文件", "Rust Source File").
fn type_name(path: &Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME};
    let mut info = SHFILEINFOW::default();
    let found = unsafe { SHGetFileInfoW(&HSTRING::from(path.as_os_str()), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_TYPENAME) };
    let name = String::from_utf16_lossy(&info.szTypeName[..info.szTypeName.iter().position(|&unit| unit == 0).unwrap_or(info.szTypeName.len())]);
    (found != 0 && !name.trim().is_empty()).then_some(name)
}

/// The words of the file at `path`, where it holds words (and as much of them as `WORDS_MOST` takes, and whether
/// they were cut off there): marked as UTF-16 or UTF-8, else UTF-8 as it is, else in the system's own code page (a
/// Chinese Windows' GBK); none in a file that holds what no words do (a nought among its bytes).
fn words_of(path: &Path) -> Option<(String, bool)> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref().take(WORDS_MOST as u64 + 1).read_to_end(&mut bytes).ok()?;
    let cut = bytes.len() > WORDS_MOST;
    bytes.truncate(WORDS_MOST);
    let text = words_in(&bytes, cut)?;
    Some((text, cut))
}

/// The words `bytes` hold (see `words_of`): `cut`, a character cut short at their end is left off.
fn words_in(bytes: &[u8], cut: bool) -> Option<String> {
    let units = |bytes: &[u8], big: bool| -> Vec<u16> { bytes.chunks_exact(2).map(|pair| if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) }).collect() };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Some(String::from_utf16_lossy(&units(rest, false)));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Some(String::from_utf16_lossy(&units(rest, true)));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_string()),
        // (Cut off in the middle of a character, the rest of it is UTF-8 still.)
        Err(error) if cut && error.error_len().is_none() => Some(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()),
        Err(_) => in_code_page(bytes),
    }
}

/// `bytes` read in the system's own code page, where they are words in it.
fn in_code_page(bytes: &[u8]) -> Option<String> {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MB_ERR_INVALID_CHARS};
    let length = unsafe { MultiByteToWideChar(CP_ACP, MB_ERR_INVALID_CHARS, bytes, None) };
    if length <= 0 {
        return None;
    }
    let mut wide = vec![0u16; length as usize];
    let written = unsafe { MultiByteToWideChar(CP_ACP, MB_ERR_INVALID_CHARS, bytes, Some(&mut wide)) };
    (written > 0).then(|| String::from_utf16_lossy(&wide[..written as usize]))
}

/// How large a thing is seen (px, turned as it is seen), where that is known before it is read, and at once: a
/// picture's own size (its header, not its pixels), a video's frames' (as the shell knows them). What the sheet is
/// sized by from its first frame (see `Looker::wanted`).
fn measure(shows: &Shows) -> Option<(u32, u32)> {
    let Shows::File(path) = shows else { return None };
    match kind_of(path) {
        Kind::Picture => picture_size(path).ok(),
        Kind::Video => video_size(path),
        _ => None,
    }
}

/// The picture in the file at `path`, its first frame's size (px) and how its camera says it was held, read from its
/// header (its pixels are not).
fn picture_header(factory: &windows::Win32::Graphics::Imaging::IWICImagingFactory, path: &Path) -> windows::core::Result<(windows::Win32::Graphics::Imaging::IWICBitmapFrameDecode, (u32, u32), u16)> {
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::WICDecodeMetadataCacheOnDemand;
    unsafe {
        let decoder = factory.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0u32, 0u32);
        frame.GetSize(&mut width, &mut height)?;
        if width == 0 || height == 0 {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        }
        let orientation = orientation_of(&frame);
        Ok((frame, (width, height), orientation))
    }
}

/// A picture's own size, turned as it is seen (see `decode`).
fn picture_size(path: &Path) -> windows::core::Result<(u32, u32)> {
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    let factory: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    let (_, (width, height), orientation) = picture_header(&factory, path)?;
    Ok(if matches!(orientation, 5..=8) { (height, width) } else { (width, height) })
}

/// A video's frames' size (px), turned as it plays (a phone's, held upright, is kept on its side and said to be
/// turned), as the shell knows it: none where it does not.
fn video_size(path: &Path) -> Option<(u32, u32)> {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::UI::Shell::{IShellItem2, SHCreateItemFromParsingName};
    // (The shell's System.Video.FrameWidth, FrameHeight and Orientation.)
    const VIDEO: windows::core::GUID = windows::core::GUID::from_u128(0x64440491_4c8b_11d1_8b70_080036b11a03);
    const WIDTH: PROPERTYKEY = PROPERTYKEY { fmtid: VIDEO, pid: 3 };
    const HEIGHT: PROPERTYKEY = PROPERTYKEY { fmtid: VIDEO, pid: 4 };
    const TURNED: PROPERTYKEY = PROPERTYKEY { fmtid: VIDEO, pid: 99 };
    unsafe {
        let item: IShellItem2 = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let (width, height) = (item.GetUInt32(&WIDTH).ok()?, item.GetUInt32(&HEIGHT).ok()?);
        if width == 0 || height == 0 {
            return None;
        }
        let quarter = item.GetUInt32(&TURNED).is_ok_and(|degrees| degrees % 180 == 90);
        Some(if quarter { (height, width) } else { (width, height) })
    }
}

/// The picture in the file at `path`, turned as its camera says it was held, no larger than `PICTURE_SIDE` and
/// `PICTURE_PIXELS` (made smaller to fit), premultiplied, known by `key`; and its own size, turned so.
fn decode(path: &Path, key: u64) -> windows::core::Result<(Picture, (u32, u32))> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let (frame, (width, height), orientation) = picture_header(&factory, path)?;
        // (Made smaller as a whole: its longer side, and all its pixels, no more than may be held.)
        let k = (PICTURE_SIDE as f64 / width.max(height) as f64).min((PICTURE_PIXELS as f64 / (width as f64 * height as f64)).sqrt()).min(1.0);
        let source: IWICBitmapSource = if k < 1.0 {
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&frame, ((width as f64 * k).round() as u32).max(1), ((height as f64 * k).round() as u32).max(1), WICBitmapInterpolationModeFant)?;
            scaler.into()
        } else {
            frame.into()
        };
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let turned: IWICBitmapSource = match turn_of(orientation) {
            Some(turn) => {
                let rotator = factory.CreateBitmapFlipRotator()?;
                rotator.Initialize(&converter, turn)?;
                rotator.into()
            }
            None => converter.into(),
        };
        let (mut w, mut h) = (0u32, 0u32);
        turned.GetSize(&mut w, &mut h)?;
        let row = w as usize * 4;
        let mut pixels = vec![0u8; row * h as usize];
        turned.CopyPixels(std::ptr::null(), row as u32, &mut pixels)?;
        // (Turned a quarter, its own size turned too.)
        let own = if matches!(orientation, 5..=8) { (height, width) } else { (width, height) };
        Ok((Picture { key, size: (w, h), pixels: std::sync::Arc::new(pixels) }, own))
    }
}

/// How the camera that took a picture says it was held (EXIF's orientation, 1 as it is: see `turn_of`); 1 where it
/// says nothing.
fn orientation_of(frame: &windows::Win32::Graphics::Imaging::IWICBitmapFrameDecode) -> u16 {
    let Ok(reader) = (unsafe { frame.GetMetadataQueryReader() }) else { return 1 };
    for name in [w!("/app1/ifd/{ushort=274}"), w!("/ifd/{ushort=274}")] {
        let mut value = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
        if unsafe { reader.GetMetadataByName(name, &mut value) }.is_ok() {
            if let Ok(orientation) = u16::try_from(&value) {
                return orientation;
            }
        }
    }
    1
}

/// What turns a picture held as `orientation` says upright: turned (clockwise) and flipped.
fn turn_of(orientation: u16) -> Option<windows::Win32::Graphics::Imaging::WICBitmapTransformOptions> {
    use windows::Win32::Graphics::Imaging::*;
    Some(match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0),
        8 => WICBitmapTransformRotate270,
        _ => return None,
    })
}

// ----- The look -----

/// What the window shows: what is drawn, or (glass, Windows 11) that over a pane of the screen behind it frosted by
/// the compositor.
enum Drawn {
    Plain(Surface),
    Frosted(super::glass::Frosted),
}

/// How it moves now: out of a tile (from where, DIPs), from one place to another (its last), back into a tile (from
/// where, to where), or not at all.
#[derive(Clone, Copy, Debug)]
enum Motion {
    Opening { from: Rect, start: Instant },
    Moving { was: Rect, start: Instant, fade: bool },
    /// A file of the group shown grown out of its place (`cell`, DIPs on the sheet) to be looked at by itself, the
    /// sheet from where it was; shrunk back into its place from where it was shown (`from`).
    Into { member: usize, cell: Rect, was: Rect, start: Instant },
    OutOf { member: usize, from: Rect, cell: Rect, was: Rect, start: Instant },
    Closing { was: Rect, to: Rect, start: Instant },
    Resting,
}

/// How it is drawn at a moment: where (DIPs on its window), laid out how large, how clear, its body how clear.
#[derive(Clone, Copy, Debug)]
struct Shape {
    at: Rect,
    size: (f32, f32),
    opacity: f32,
    body: f32,
}

/// What a place on it is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Copy,
    Open,
    Body,
    /// A file of the group shown, by its place; the button back to the group.
    Cell(usize),
    Back,
    /// What plays: played or paused (its button, a video itself), where it is along it (its track, a sound's
    /// loudness), how loud (the slider), muted or not; the strip of a video's controls (clicks on it go nowhere).
    Toggle,
    Track,
    Volume,
    Mute,
    Strip,
}

/// What of what plays is held (dragged): its track (where it is taken to, seconds), its volume's slider.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Holding {
    Track(f64),
    Volume,
}

struct Looker {
    hwnd: HWND,
    gfx: Rc<Gfx>,
    drawn: Option<Drawn>,
    showing: Option<super::life::Showing>,
    reads: Sender<Reading>,
    /// Its look, numbered (what is read for an older one is let go of).
    number: u64,
    items: Vec<Item>,
    at: usize,
    owner: isize,
    tell: Option<u32>,
    beside: Option<RECT>,
    skin: Skin,
    dark: bool,
    frost: f32,
    lang: Lang,
    /// Its screen (physical px) and how many px a DIP is there; its window covers it.
    monitor: RECT,
    work: RECT,
    scale: f32,
    /// What each thing read is (by its place), how large each is seen where that was known before (see `measure`), and
    /// those asked to be.
    read: Vec<Option<Read>>,
    measured: Vec<Option<(u32, u32)>>,
    asked: Vec<bool>,
    /// Files together shown (see `Body::Group`): the one looked at by itself, the one the keys picked (ringed among
    /// them), how far they are scrolled (DIPs) and may be; what each was read as by itself (by its place), and those
    /// asked to be.
    inside: Option<usize>,
    picked: Option<usize>,
    grid_scroll: f32,
    grid_scroll_most: f32,
    member_read: Vec<Option<Read>>,
    member_asked: Vec<bool>,
    motion: Motion,
    /// A picture zoomed (1: fitted), moved (DIPs), and where a drag of it began (the pointer, and where it was moved).
    zoom: f32,
    pan: (f32, f32),
    dragging: Option<((f32, f32), (f32, f32))>,
    /// Words scrolled (DIPs), and how far they may be.
    scroll: f32,
    scroll_most: f32,
    /// The pointer on the window (DIPs), what a button went down on, and where each thing drawn is (DIPs on the sheet).
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    hits: Vec<(Rect, Hit)>,
    /// What a button said a moment, and since when.
    told: Option<(Hit, Instant)>,
    /// The buttons held as they were last looked at (see `watch`), and how many pictures had been made (see `thumbs`).
    held: bool,
    pictures: u64,
    /// Where on its window it takes clicks now (px), as its sheet is drawn.
    region: RECT,
    /// The window a previewer shows in (made as one is first wanted), and whether one is being made for it.
    pane: Option<pane::Pane>,
    pane_making: bool,
    /// What plays the thing shown (made as it rests: see `tend_player`), and whether it could not be played; how loud
    /// (0 to 1) and whether muted, as the settings keep it.
    player: Option<play::Player>,
    unplayable: bool,
    volume: f32,
    muted: bool,
    /// When the pointer last moved over a video (its controls show till a while after), what of them is held, whether
    /// the volume's slider is to be open (how open it was, since when it opens or closes), and where its parts are
    /// (DIPs on the sheet, as last drawn): the strip, the track, the volume's button and slider together, the slider.
    stirred: Instant,
    holding: Option<Holding>,
    slider_open: (bool, f32, Instant),
    strip_at: Option<Rect>,
    track_at: Option<Rect>,
    loud_at: Option<Rect>,
    slider_at: Option<Rect>,
    /// How it was last drawn: where what plays was (seconds), how clear its controls, how open its slider.
    drawn_playing: (f64, f32, f32),
}

impl Looker {
    fn new(hwnd: HWND, gfx: Rc<Gfx>, reads: Sender<Reading>) -> Self {
        Looker {
            hwnd,
            gfx,
            drawn: None,
            showing: None,
            reads,
            number: 0,
            items: Vec::new(),
            at: 0,
            owner: 0,
            tell: None,
            beside: None,
            skin: Skin::Fluent,
            dark: false,
            frost: 1.0,
            lang: Lang::Zh,
            monitor: RECT::default(),
            work: RECT::default(),
            scale: 1.0,
            read: Vec::new(),
            measured: Vec::new(),
            asked: Vec::new(),
            inside: None,
            picked: None,
            grid_scroll: 0.0,
            grid_scroll_most: 0.0,
            member_read: Vec::new(),
            member_asked: Vec::new(),
            motion: Motion::Resting,
            zoom: 1.0,
            pan: (0.0, 0.0),
            dragging: None,
            scroll: 0.0,
            scroll_most: 0.0,
            pointer: None,
            pressed: None,
            hits: Vec::new(),
            told: None,
            held: false,
            pictures: 0,
            region: RECT::default(),
            pane: None,
            pane_making: false,
            player: None,
            unplayable: false,
            volume: 0.7,
            muted: false,
            stirred: Instant::now(),
            holding: None,
            slider_open: (false, 0.0, Instant::now()),
            strip_at: None,
            track_at: None,
            loud_at: None,
            slider_at: None,
            drawn_playing: (0.0, 1.0, 0.0),
        }
    }

    fn up(&self) -> bool {
        !self.items.is_empty()
    }

    fn asked(&mut self) {
        let asked = std::mem::take(&mut *ASKS.lock().unwrap());
        for asked in asked {
            match asked {
                Asked::Open(opening) => self.open(opening),
                Asked::Close => self.close(),
                Asked::Key(key) => self.key(key),
                Asked::Show(key) => {
                    if let Some(at) = self.items.iter().position(|item| item.key == key) {
                        self.go_to(at);
                    }
                }
                Asked::Among(items, key) => self.among(items, key),
                Asked::Tiles(owner, tiles) if owner == self.owner => {
                    for item in &mut self.items {
                        item.from = tiles.iter().find(|(key, _)| *key == item.key).map(|(_, rect)| *rect);
                    }
                }
                Asked::Tiles(..) => {}
            }
        }
    }

    // ----- Opening, going -----

    fn open(&mut self, opening: Opening) {
        if opening.items.is_empty() {
            return;
        }
        let at = opening.at.min(opening.items.len() - 1);
        // (On the screen its tile is on, else the one the pointer is on.)
        let mut point = POINT::default();
        let _ = unsafe { GetCursorPos(&mut point) };
        if let Some(from) = opening.items[at].from {
            point = POINT { x: (from.left + from.right) / 2, y: (from.top + from.bottom) / 2 };
        }
        let Some(contact) = crate::mouse::monitor_at(point) else { return };
        let changed = opening.skin != self.skin || opening.dark != self.dark || contact.scale != self.scale;
        if changed {
            self.drawn = None;
        }
        self.number += 1;
        self.lang = crate::update::language();
        (self.skin, self.dark, self.frost) = (opening.skin, opening.dark, opening.frost);
        (self.monitor, self.work, self.scale) = (contact.monitor, contact.work, contact.scale);
        (self.owner, self.tell, self.beside) = (opening.owner, opening.tell, opening.beside);
        {
            let settings = crate::app().settings.lock().unwrap();
            (self.volume, self.muted) = (settings.look_volume.clamp(0.0, 1.0), settings.look_muted);
        }
        self.read = vec![None; opening.items.len()];
        self.measured = vec![None; opening.items.len()];
        self.asked = vec![false; opening.items.len()];
        self.items = opening.items;
        self.at = at;
        self.out_of_group();
        self.reset_view();
        self.read_around();
        let from = self.tile_of(at).unwrap_or_else(|| shrunk(self.target(), 0.92));
        self.motion = Motion::Opening { from, start: Instant::now() };
        OWNER.store(self.owner, Ordering::Release);
        SHOWN.store(self.items[at].key, Ordering::Release);
        self.held = buttons_held();
        let (w, h) = (self.monitor.right - self.monitor.left, self.monitor.bottom - self.monitor.top);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), self.monitor.left, self.monitor.top, w, h, SWP_NOACTIVATE);
        }
        self.draw();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
            let _ = SetTimer(Some(self.hwnd), WATCH, WATCH_MS, None);
        }
        crate::widget::own_window_changed(self.monitor);
        self.told_owner(at);
        crate::app().controller.wake();
    }

    /// It goes back into its tile (where the tile shows now; else it fades where it is).
    fn close(&mut self) {
        if !self.up() || matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let now = Instant::now();
        let was = self.shape(now).at;
        let to = self.tile_of(self.at).unwrap_or_else(|| shrunk(was, 0.92));
        self.motion = Motion::Closing { was, to, start: now };
        self.dragging = None;
        // (Still as it goes back: its frame then shrinks with it.)
        if let Some(player) = &self.player {
            player.pause();
        }
        self.holding = None;
        let _ = unsafe { ReleaseCapture() };
        // (Gone for what called it up at once: its keys its own again, as it goes.)
        OWNER.store(0, Ordering::Release);
        if let Some(tell) = self.tell {
            let _ = unsafe { PostMessageW(Some(HWND(self.owner as *mut _)), tell, WPARAM(usize::MAX), LPARAM(0)) };
        }
        crate::app().controller.wake();
        unsafe {
            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
        }
    }

    /// Gone: its window hidden, what it draws with and what it read let go of.
    fn gone(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), FRAME);
            let _ = KillTimer(Some(self.hwnd), WATCH);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        crate::widget::own_window_changed(self.monitor);
        if let Some(pane) = &mut self.pane {
            pane.unload();
        }
        self.stop_playing();
        self.items.clear();
        self.read.clear();
        self.measured.clear();
        self.asked.clear();
        self.out_of_group();
        self.drawn = None;
        self.showing = None;
        self.motion = Motion::Resting;
        self.pointer = None;
        self.pressed = None;
        self.told = None;
        READ.lock().unwrap().clear();
        MEASURED.lock().unwrap().clear();
    }

    /// What called it up told which thing it shows now.
    fn told_owner(&self, at: usize) {
        if let Some(tell) = self.tell {
            let _ = unsafe { PostMessageW(Some(HWND(self.owner as *mut _)), tell, WPARAM(at), LPARAM(0)) };
        }
    }

    // ----- What it shows -----

    /// The thing `at` shown (from where it was, as it grows or shrinks to that one's size).
    fn go_to(&mut self, at: usize) {
        if !self.up() || at >= self.items.len() || at == self.at || matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let was = self.shape(Instant::now()).at;
        self.at = at;
        self.moved_from(was);
    }

    /// The thing known by `key` shown, `items` those had by the arrows from now on (what was read of those that were
    /// before kept). One not among them: nothing done.
    fn among(&mut self, items: Vec<Item>, key: u64) {
        let Some(at) = items.iter().position(|item| item.key == key) else { return };
        if !self.up() || matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let was = self.shape(Instant::now()).at;
        let shown = self.items[self.at].key;
        let read = carried(&self.items, std::mem::take(&mut self.read), &items);
        self.measured = carried(&self.items, std::mem::take(&mut self.measured), &items);
        // (What is being read is for the places as they were: asked again, where it is wanted, at theirs now.)
        self.number += 1;
        self.asked = read.iter().map(Option::is_some).collect();
        self.member_asked = self.member_read.iter().map(Option::is_some).collect();
        self.read = read;
        self.items = items;
        self.at = at;
        if key == shown {
            self.read_around();
            self.read_members_around();
            return self.draw();
        }
        self.moved_from(was);
    }

    /// The thing now at `at` shown, moving from where the sheet was (`was`).
    fn moved_from(&mut self, was: Rect) {
        let (at, now) = (self.at, Instant::now());
        self.out_of_group();
        self.reset_view();
        self.read_around();
        self.motion = Motion::Moving { was, start: now, fade: true };
        SHOWN.store(self.items[at].key, Ordering::Release);
        self.told_owner(at);
        unsafe {
            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
        }
        crate::app().controller.wake();
        self.draw();
    }

    /// What was had of the group shown let go of: none of its files looked at by itself, none picked, not scrolled.
    fn out_of_group(&mut self) {
        self.inside = None;
        self.picked = None;
        self.grid_scroll = 0.0;
        self.member_read.clear();
        self.member_asked.clear();
        INSIDE.store(false, Ordering::Release);
    }

    /// What was had of the thing shown let go of, another to be shown: as it was zoomed, scrolled, played.
    fn reset_view(&mut self) {
        self.stop_playing();
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.dragging = None;
        self.scroll = 0.0;
        self.told = None;
    }

    /// The thing shown read, and those beside it (for the arrows); what is read of those further off let go of.
    fn read_around(&mut self) {
        let at = self.at;
        for (place, read) in self.read.iter_mut().enumerate() {
            if place.abs_diff(at) > 2 {
                *read = None;
                self.asked[place] = false;
            }
        }
        // (The one shown first: asked last, it is read first.)
        for place in [at + 1, at.wrapping_sub(1), at] {
            if place < self.items.len() && !self.asked[place] {
                self.asked[place] = true;
                let _ = self.reads.send(Reading { number: self.number, at: place, member: None, shows: self.items[place].shows.clone(), lang: self.lang, scale: self.scale });
            }
        }
    }

    /// The file of the group looked at by itself read, and those beside it (for the arrows); what is read of those
    /// further off let go of.
    fn read_members_around(&mut self) {
        let Some(member) = self.inside else { return };
        let Some(paths) = self.members().map(|members| members.iter().map(|member| member.path.clone()).collect::<Vec<_>>()) else { return };
        if self.member_read.len() != paths.len() {
            self.member_read = vec![None; paths.len()];
            self.member_asked = vec![false; paths.len()];
        }
        for (place, read) in self.member_read.iter_mut().enumerate() {
            if place.abs_diff(member) > 2 {
                *read = None;
                self.member_asked[place] = false;
            }
        }
        // (The one shown first: asked last, it is read first.)
        for place in [member + 1, member.wrapping_sub(1), member] {
            if place < paths.len() && !self.member_asked[place] {
                self.member_asked[place] = true;
                let shows = Shows::File(paths[place].clone());
                let _ = self.reads.send(Reading { number: self.number, at: self.at, member: Some(place), shows, lang: self.lang, scale: self.scale });
            }
        }
    }

    /// The file `member` of the group shown looked at by itself: grown out of its place, the sheet to its size.
    fn enter(&mut self, member: usize) {
        if matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let Some(cell) = self.cell_on_sheet(member) else { return };
        let now = Instant::now();
        let was = self.shape(now).at;
        self.inside = Some(member);
        self.picked = Some(member);
        INSIDE.store(true, Ordering::Release);
        self.reset_view();
        self.read_members_around();
        self.motion = Motion::Into { member, cell, was, start: now };
        self.moving();
    }

    /// Back to the group from the file of it looked at by itself: shrunk back into its place (the group scrolled to
    /// show it), the sheet to the group's size.
    fn leave(&mut self) {
        let Some(member) = self.inside else { return };
        if matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let now = Instant::now();
        let shape = self.shape(now);
        let body = self.body_at(shape.size);
        // (From where it shows: zoomed in, as large as it is.)
        let from = match self.fitted().filter(|_| self.pictured()) {
            Some((fitted, _)) => {
                let (pw, ph) = (fitted.2 * self.zoom, fitted.3 * self.zoom);
                let (cx, cy) = (fitted.0 + fitted.2 / 2.0 + self.pan.0, fitted.1 + fitted.3 / 2.0 + self.pan.1);
                Rect { x: cx - pw / 2.0, y: cy - ph / 2.0, w: pw, h: ph }
            }
            None => self.member_fit(member, body),
        };
        self.inside = None;
        INSIDE.store(false, Ordering::Release);
        self.reset_view();
        self.reveal(member);
        let Some(cell) = self.cell_on_sheet(member) else { return };
        self.motion = Motion::OutOf { member, from, cell, was: shape.at, start: now };
        self.moving();
    }

    /// The file of the group beside the one looked at by itself (`forward`: after it) looked at instead.
    fn step(&mut self, forward: bool) {
        let Some(member) = self.inside else { return };
        let count = self.members().map_or(0, <[Member]>::len);
        let next = if forward { member + 1 } else { member.wrapping_sub(1) };
        if next >= count || matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let now = Instant::now();
        let was = self.shape(now).at;
        self.inside = Some(next);
        self.picked = Some(next);
        self.reset_view();
        self.read_members_around();
        self.motion = Motion::Moving { was, start: now, fade: true };
        self.moving();
    }

    /// Another file of the group shown picked by the arrow `key`: the one beside it in its row, or the nearest in the row
    /// above or below. Past the group's first or last (none that way), the thing before or after it shown instead.
    fn pick(&mut self, key: Key) {
        let Some(members) = self.members() else { return };
        let (cells, _) = group_cells(&shapes(members), self.group_inner());
        let picked = match self.picked {
            None => Some(0),
            Some(at) => {
                let here = cells[at];
                let middle = |cell: &Rect| cell.x + cell.w / 2.0;
                match key {
                    Key::Left => at.checked_sub(1),
                    Key::Right => Some(at + 1).filter(|next| *next < cells.len()),
                    _ => {
                        // (The nearest row that way, and the place in it nearest under or over this one's middle.)
                        let row = cells.iter().map(|cell| cell.y).filter(|y| if key == Key::Down { *y > here.y } else { *y < here.y });
                        let row = if key == Key::Down { row.reduce(f32::min) } else { row.reduce(f32::max) };
                        row.and_then(|row| {
                            (0..cells.len()).filter(|place| cells[*place].y == row).min_by(|a, b| (middle(&cells[*a]) - middle(&here)).abs().total_cmp(&(middle(&cells[*b]) - middle(&here)).abs()))
                        })
                    }
                }
            }
        };
        match picked {
            Some(picked) => {
                self.picked = Some(picked);
                self.reveal(picked);
                self.draw();
            }
            None => match key {
                Key::Left => self.go_to(self.at.wrapping_sub(1)),
                Key::Right => self.go_to(self.at + 1),
                _ => {
                    if let Some(at) = self.beside_of(key == Key::Down) {
                        self.go_to(at);
                    }
                }
            },
        }
    }

    /// The group shown scrolled as little as shows the file `member` of it whole, and its name, as the sheet is to be.
    fn reveal(&mut self, member: usize) {
        let Some(members) = self.members() else { return };
        let (cells, tall) = group_cells(&shapes(members), self.group_inner());
        let target = self.target();
        let room = self.body_at((target.w, target.h)).h;
        let cell = cells[member];
        let (top, bottom) = (cell.y, cell.y + cell.h + GROUP_NAME);
        if top < self.grid_scroll {
            self.grid_scroll = top;
        } else if bottom > self.grid_scroll + room {
            self.grid_scroll = bottom - room;
        }
        self.grid_scroll_most = (tall - room).max(0.0);
        self.grid_scroll = self.grid_scroll.clamp(0.0, self.grid_scroll_most);
    }

    /// Where the file `member` of the group shows by itself in `body` (DIPs on the sheet): its picture as read, else
    /// as large as it was measured, fitted.
    fn member_fit(&self, member: usize, body: Rect) -> Rect {
        let picture = self.member_read.get(member).and_then(Option::as_ref).and_then(|read| match &read.body {
            Body::Picture(picture, _) | Body::Pictured(picture) | Body::Live { picture: Some(picture), .. } | Body::Played { picture: Some(picture), sound: false, .. } => Some(picture.size),
            _ => None,
        });
        let seen = picture.or_else(|| self.members()?.get(member)?.seen).unwrap_or((1, 1));
        let fitted = Picture { key: 0, size: seen, pixels: Default::default() }.fitted_in(body.x, body.y, body.w, body.h);
        Rect { x: fitted.0, y: fitted.1, w: fitted.2, h: fitted.3 }
    }

    /// The sheet in motion: drawn every frame until it rests, and what called it up told (the keys it takes change).
    fn moving(&mut self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
        }
        crate::app().controller.wake();
        self.draw();
    }

    /// What was read (or measured), where it is for this look: taken up (its sheet sized for it anew).
    fn loaded(&mut self) {
        let measured = std::mem::take(&mut *MEASURED.lock().unwrap());
        let read = std::mem::take(&mut *READ.lock().unwrap());
        let mut resized = false;
        for (number, at, size) in measured {
            if number != self.number || at >= self.measured.len() || !self.asked[at] {
                continue;
            }
            resized |= at == self.at;
            self.measured[at] = Some(size);
        }
        for (number, at, member, read) in read {
            if number != self.number || at >= self.read.len() {
                continue;
            }
            match member {
                None if self.asked[at] => {
                    resized |= at == self.at;
                    self.read[at] = Some(read);
                }
                // (A file of the group shown, read by itself.)
                Some(member) if at == self.at && self.member_asked.get(member) == Some(&true) => {
                    resized |= self.inside == Some(member);
                    self.member_read[member] = Some(read);
                }
                _ => {}
            }
        }
        if resized && self.up() {
            // (As it opens, it grows to the size it now has; else it moves there from where it is.)
            if let Motion::Resting | Motion::Moving { .. } = self.motion {
                let now = Instant::now();
                let was = self.shape(now).at;
                if was != self.target() {
                    self.motion = Motion::Moving { was, start: now, fade: false };
                    unsafe {
                        let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
                    }
                }
            }
        }
        self.draw();
    }

    /// What the thing shown was read as (by itself: the file of its group shown so).
    fn shown_read(&self) -> Option<&Read> {
        match self.inside {
            Some(member) => self.member_read.get(member)?.as_ref(),
            None => self.read.get(self.at)?.as_ref(),
        }
    }

    fn body(&self) -> Option<&Body> {
        self.shown_read().map(|read| &read.body)
    }

    /// The files of the group shown, read.
    fn members(&self) -> Option<&[Member]> {
        match &self.read.get(self.at)?.as_ref()?.body {
            Body::Group(members) => Some(members),
            _ => None,
        }
    }

    /// The file shown: the thing's, or the file of its group shown by itself.
    fn shown_path(&self) -> Option<&Path> {
        match self.inside {
            Some(member) => Some(self.members()?.get(member)?.path.as_path()),
            None => match &self.items[self.at].shows {
                Shows::File(path) => Some(path),
                _ => None,
            },
        }
    }

    /// What the thing shown is before it is read.
    fn shown_kind(&self) -> Kind {
        match self.inside {
            Some(_) => self.shown_path().map_or(Kind::Unknown, kind_of),
            None => kind_shown(&self.items[self.at].shows),
        }
    }

    /// How large the thing shown is seen, where that was known before it was read (see `measure`).
    fn shown_measured(&self) -> Option<Seen> {
        match self.inside {
            Some(member) => self.members()?.get(member)?.seen,
            None => self.measured.get(self.at).copied().flatten(),
        }
    }

    // ----- Where it is -----

    /// `rect` (physical px on the screen) on its window (DIPs).
    fn on_window(&self, rect: RECT) -> Rect {
        let s = self.scale;
        Rect { x: (rect.left - self.monitor.left) as f32 / s, y: (rect.top - self.monitor.top) as f32 / s, w: (rect.right - rect.left) as f32 / s, h: (rect.bottom - rect.top) as f32 / s }
    }

    /// Where the tile of the thing `at` is on its window (DIPs), where it shows.
    fn tile_of(&self, at: usize) -> Option<Rect> {
        self.items.get(at)?.from.map(|rect| self.on_window(rect))
    }

    /// The room it may take on its screen (DIPs on its window): beside what it was called up from, on the side with the
    /// most room; or all of its screen's, but for the room kept round it.
    fn room(&self) -> Rect {
        let work = self.on_window(self.work);
        let inside = |r: Rect| Rect { x: r.x + GAP, y: r.y + GAP, w: (r.w - 2.0 * GAP).max(0.0), h: (r.h - 2.0 * GAP).max(0.0) };
        let Some(beside) = self.beside.map(|rect| self.on_window(rect)) else { return inside(work) };
        let (right, bottom) = (work.x + work.w, work.y + work.h);
        let sides = [
            Rect { x: work.x, y: work.y, w: beside.x - work.x, h: work.h },
            Rect { x: beside.x + beside.w, y: work.y, w: right - beside.x - beside.w, h: work.h },
            Rect { x: work.x, y: work.y, w: work.w, h: beside.y - work.y },
            Rect { x: work.x, y: beside.y + beside.h, w: work.w, h: bottom - beside.y - beside.h },
        ];
        let roomiest = sides.into_iter().map(inside).max_by(|a, b| (a.w.min(MOST.0 * work.w) * a.h.min(MOST.1 * work.h)).total_cmp(&(b.w.min(MOST.0 * work.w) * b.h.min(MOST.1 * work.h))));
        roomiest.filter(|side| side.w >= OTHER.0 && side.h >= OTHER.1).unwrap_or_else(|| inside(work))
    }

    /// Where the sheet goes for the thing shown (DIPs on its window): as large as what it shows wants (see `wanted`), in
    /// the middle of its room.
    fn target(&self) -> Rect {
        let room = self.room();
        let (w, h) = self.wanted(self.most());
        let (w, h) = (w.min(room.w).max(1.0), h.min(room.h).max(1.0));
        Rect { x: (room.x + (room.w - w) / 2.0).round(), y: (room.y + (room.h - h) / 2.0).round(), w: w.round(), h: h.round() }
    }

    /// How large it may be (DIPs): its room, and no more of its screen than `MOST`.
    fn most(&self) -> (f32, f32) {
        let (room, work) = (self.room(), self.on_window(self.work));
        (room.w.min(MOST.0 * work.w), room.h.min(MOST.1 * work.h))
    }

    /// How wide the body of the group shown is (DIPs): its files laid out in rows that wide (see `group_cells`).
    fn group_inner(&self) -> f32 {
        GROUP_WIDE.min(self.most().0) - 2.0 * self.inset()
    }

    /// Where the file `member` of the group shown is on the sheet (DIPs), its picture in its place among the others, as
    /// far as they are scrolled.
    fn cell_on_sheet(&self, member: usize) -> Option<Rect> {
        let (cells, _) = group_cells(&shapes(self.members()?), self.group_inner());
        let cell = cells.get(member)?;
        Some(Rect { x: self.inset() + cell.x, y: self.body_top() + cell.y - self.grid_scroll, w: cell.w, h: cell.h })
    }

    /// How far in from its edges its parts are, all round (DIPs): as far as makes the corners of what its body shows
    /// concentric with its own (its corners' radius less theirs), and no less than `INSET_LEAST`.
    fn inset(&self) -> f32 {
        (Theme::new(self.skin, self.dark).radius - self.body_radius()).max(INSET_LEAST)
    }

    /// How tall its bar is (DIPs): its name `inset` down from its top, and room under it. (Windows 11's bar drawn so
    /// deep, chart paper's ruled off there.)
    fn bar(&self) -> f32 {
        self.inset() + NAME + UNDER_NAME
    }

    /// How far down its body begins (DIPs): under its bar; past Windows 11's bar and chart paper's rule, the room under
    /// a name again.
    fn body_top(&self) -> f32 {
        if self.skin == Skin::Glass { self.bar() } else { self.bar() + UNDER_NAME + 2.0 }
    }

    /// How large the sheet wants to be for the thing shown, `most` at most (DIPs): a picture whole, as large as fits, no
    /// larger than its own pixels; a video as a picture of its frames' size; a page as tall as it may be; words, and
    /// what has no picture, at their sizes. A picture or a video is so from before it is read, by its size measured
    /// (see `measure`): the sheet is not opened at one size and made another as what it shows comes.
    fn wanted(&self, most: (f32, f32)) -> (f32, f32) {
        let (top, pad) = (self.body_top(), self.inset());
        // A picture `(pw, ph)` px large, as large as fits, no larger than its own pixels where they are known (`own`).
        let pictured = |(pw, ph): (u32, u32), own: bool| {
            let (pw, ph) = (pw.max(1) as f32 / self.scale, ph.max(1) as f32 / self.scale);
            let k = ((most.0 - 2.0 * pad) / pw).min((most.1 - top - pad) / ph).max(0.0);
            let k = if own { k.min(1.0) } else { k };
            ((pw * k + 2.0 * pad).max(PICTURE_LEAST), ph * k + top + pad)
        };
        // (A page as tall as it may be; where that is wider than it may be, as wide as it may be and as tall as a page is
        // for that width.)
        let page = || {
            let w = (most.1 - top - pad) / PAGE + 2.0 * pad + PAGE_ROOM;
            if w <= most.0 {
                (w.max(PICTURE_LEAST), most.1)
            } else {
                (most.0, ((most.0 - 2.0 * pad - PAGE_ROOM) * PAGE + top + pad).min(most.1))
            }
        };
        let measured = self.shown_measured();
        match self.body() {
            Some(Body::Picture(_, own)) => pictured(*own, true),
            // (A video's picture stands for it, made no larger than may be asked: it is shown at its frames' size.)
            Some(Body::Played { sound: true, .. }) => (SOUND_WIDE.min(most.0), top + SOUND_COVER + pad),
            // (A video at its frames' size, as measured, else as its picture is.)
            Some(Body::Played { picture, .. }) => match measured.or(picture.as_ref().map(|picture| picture.size)) {
                Some(size) => pictured(size, true),
                None => most,
            },
            Some(Body::Pictured(picture) | Body::Live { picture: Some(picture), .. }) => pictured(measured.unwrap_or(picture.size), true),
            Some(Body::Live { picture: None, .. }) => measured.map_or_else(page, |size| pictured(size, true)),
            Some(Body::Words { .. }) => (WORDS.0.min(most.0), WORDS.1.min(most.1)),
            // (Its files in rows, as tall as they are, scrolled where they are taller than it may be.)
            Some(Body::Group(members)) => {
                let wide = GROUP_WIDE.min(most.0);
                let (_, tall) = group_cells(&shapes(members), wide - 2.0 * pad);
                (wide, (top + tall + pad).min(most.1))
            }
            Some(Body::Other(_)) => OTHER,
            // (Not read yet: as large as it was measured.)
            None => match measured {
                Some(size) => pictured(size, true),
                // (Neither read nor measured yet, the first moment it opens: as large as what its name says it is wants, a
                // picture of the shape its tile's is.)
                None => match self.shown_kind() {
                    Kind::Picture | Kind::Video => match self.shown_path() {
                        Some(path) => crate::station::thumbs::of(path, 256).map_or(most, |thumb| pictured(thumb.size, false)),
                        None => most,
                    },
                    Kind::Page => page(),
                    Kind::Sound => (SOUND_WIDE.min(most.0), top + SOUND_COVER + pad),
                    Kind::Words | Kind::Files => (WORDS.0.min(most.0), WORDS.1.min(most.1)),
                    Kind::Unknown => OTHER,
                },
            },
        }
    }

    /// How it is drawn at `now` (see `Motion`).
    fn shape(&self, now: Instant) -> Shape {
        let target = self.target();
        let size = |r: Rect| (r.w, r.h);
        let part = |start: Instant, of: Duration| (now.duration_since(start).as_secs_f32() / of.as_secs_f32()).clamp(0.0, 1.0);
        match self.motion {
            Motion::Opening { from, start } => {
                let p = OPEN_EASE.at(part(start, OPEN_FOR));
                let opacity = if p < OPEN_WHOLE_AT { OPEN_CLEAR + (1.0 - OPEN_CLEAR) * p / OPEN_WHOLE_AT } else { 1.0 };
                Shape { at: between(from, target, p), size: size(target), opacity, body: 1.0 }
            }
            Motion::Moving { was, start, fade } => {
                let at = between(was, target, MOVE_EASE.at(part(start, MOVE_FOR)));
                Shape { at, size: size(at), opacity: 1.0, body: if fade { part(start, FADE_IN) } else { 1.0 } }
            }
            Motion::Into { was, start, .. } | Motion::OutOf { was, start, .. } => {
                let at = between(was, target, MOVE_EASE.at(part(start, INTO_FOR)));
                Shape { at, size: size(at), opacity: 1.0, body: 1.0 }
            }
            Motion::Closing { was, to, start } => {
                let p = CLOSE_EASE.at(part(start, CLOSE_FOR));
                let (still_at, still) = CLOSE_STILL;
                let opacity = if p < still_at { 1.0 - (1.0 - still) * p / still_at } else { still * (1.0 - (p - still_at) / (1.0 - still_at)) };
                Shape { at: between(was, to, p), size: size(was), opacity: opacity.max(0.0), body: 1.0 }
            }
            Motion::Resting => Shape { at: target, size: size(target), opacity: 1.0, body: 1.0 },
        }
    }

    /// Whether it has done moving by `now`: rests (gone, where it was closing).
    fn settle(&mut self, now: Instant) {
        let done = match self.motion {
            Motion::Opening { start, .. } => now.duration_since(start) >= OPEN_FOR,
            Motion::Moving { start, fade, .. } => now.duration_since(start) >= MOVE_FOR.max(if fade { FADE_IN } else { Duration::ZERO }),
            Motion::Into { start, .. } | Motion::OutOf { start, .. } => now.duration_since(start) >= INTO_FOR,
            Motion::Closing { start, .. } => now.duration_since(start) >= CLOSE_FOR,
            Motion::Resting => true,
        };
        if !done {
            return;
        }
        if let Motion::Closing { .. } = self.motion {
            self.gone();
            return;
        }
        self.motion = Motion::Resting;
        if !self.plays_on(now) {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), FRAME);
            }
        }
    }

    // ----- Drawing -----

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

    fn draw(&mut self) {
        if !self.up() {
            return;
        }
        let now = Instant::now();
        self.settle(now);
        if !self.up() {
            return;
        }
        if self.drawn.is_none() {
            if let Ok(gfx) = gfx::current() {
                self.gfx = gfx;
            }
            use super::skins::{ACRYLIC_BLUR, ACRYLIC_SATURATION, FROST_BLUR, GLASS_SATURATION};
            let frosting = match self.skin {
                Skin::Glass => Some((FROST_BLUR, GLASS_SATURATION)),
                Skin::Fluent => Some((ACRYLIC_BLUR, ACRYLIC_SATURATION)),
                Skin::Paper => None,
            };
            self.drawn = match frosting.and_then(|frosting| super::glass::Frosted::new(&self.gfx, self.hwnd, frosting).ok()) {
                Some(frosted) => Some(Drawn::Frosted(frosted)),
                None => Surface::new(&self.gfx, self.hwnd).ok().map(Drawn::Plain),
            };
        }
        let Some(mut drawn) = self.drawn.take() else { return };
        if self.showing.is_none() {
            self.showing = Some(super::life::Showing::of(&self.gfx));
        }
        self.tend_player();
        let shape = self.shape(now);
        self.take_clicks(shape.at);
        let gfx = self.gfx.clone();
        let size = ((self.monitor.right - self.monitor.left) as u32, (self.monitor.bottom - self.monitor.top) as u32);
        let scale = self.scale;
        let done = match &mut drawn {
            Drawn::Plain(surface) => surface.draw(&gfx, size, scale, |frame| self.paint(frame, false, shape)),
            Drawn::Frosted(frosted) => {
                // (Glass as frosted as the panel's would be there, Windows 11's acrylic always; as clear as it is drawn.)
                frosted.set_frost(if self.skin == Skin::Glass { self.frost } else { 1.0 } * shape.opacity);
                let k = (shape.at.w / shape.size.0).min(shape.at.h / shape.size.1);
                let radius = self.colours(true).radius * k;
                let pane = (shape.at.x * scale, shape.at.y * scale, shape.at.w * scale, shape.at.h * scale, radius * scale);
                frosted.draw(&gfx, size, scale, pane, |frame| self.paint(frame, true, shape))
            }
        };
        match done {
            Err(error) if gfx::failed("look", Some(&error)) => {}
            _ => self.drawn = Some(drawn),
        }
        self.place_pane(shape);
        self.drawn_playing = (self.shown_at(), self.controls_clear(now), self.slider_open_now(now));
    }

    // ----- Its previewer -----

    /// The file shown, and its previewer, where one shows it: while the sheet rests.
    fn live(&self) -> Option<(PathBuf, windows::core::GUID)> {
        match (self.body()?, self.shown_path()) {
            (Body::Live { previewer, .. }, Some(path)) if matches!(self.motion, Motion::Resting) => Some((path.to_path_buf(), *previewer)),
            _ => None,
        }
    }

    /// Where the body of the sheet drawn as `shape` is on the screen (physical px).
    fn body_on_screen(&self, shape: Shape) -> RECT {
        on_screen(self.body_at(shape.size), shape.at, self.scale, self.monitor)
    }

    /// Its previewer laid over the body of the sheet drawn as `shape`, where one shows what it shows (asked to be made,
    /// where it is not yet: see `pane_wanted`). Else hidden.
    fn place_pane(&mut self, shape: Shape) {
        let Some((path, _)) = self.live() else {
            if let Some(pane) = &mut self.pane {
                pane.hide();
            }
            return;
        };
        if self.pane.is_none() {
            self.pane = pane::Pane::new(self.hwnd);
        }
        let (at, radius) = (self.body_on_screen(shape), (self.body_radius() * self.scale).round() as i32);
        let Some(pane) = &mut self.pane else { return };
        if pane.shows() == Some(path.as_path()) {
            pane.show_at(at, radius);
        } else if !self.pane_making {
            self.pane_making = true;
            let _ = unsafe { PostMessageW(Some(self.hwnd), PANE, WPARAM(0), LPARAM(0)) };
        }
    }

    /// The previewer to be made for what it shows, where one is still wanted (the one it had let go of), the body's size.
    fn pane_wanted(&mut self) -> Option<pane::Wanted> {
        let wanted = self.live().filter(|(path, _)| self.pane.as_ref().is_some_and(|pane| pane.shows() != Some(path.as_path())));
        let Some((path, previewer)) = wanted else {
            self.pane_making = false;
            return None;
        };
        let at = self.body_on_screen(self.shape(Instant::now()));
        let ground = if self.dark { Color::hex(0x000000, 1.0) } else { Color::hex(0xFFFFFF, 1.0) };
        let colours = pane::Colours { ground: over(self.colours(false).sheet, ground), text: Theme::new(self.skin, self.dark).text };
        let radius = (self.body_radius() * self.scale).round() as i32;
        let pane = self.pane.as_mut()?;
        pane.unload();
        pane.lay(at, radius);
        Some(pane::Wanted { host: pane.host(), path, previewer, size: (at.right - at.left, at.bottom - at.top), colours })
    }

    /// The previewer made for the file at `path` (see `pane::load`) shown, where it is still wanted. One that could not
    /// be made: the thing shown as it is without it (its picture, else what it is).
    fn pane_made(&mut self, path: PathBuf, made: windows::core::Result<windows::Win32::UI::Shell::IPreviewHandler>) {
        self.pane_making = false;
        let wanted = self.live().is_some_and(|(live, _)| live == path);
        match (made, self.pane.as_mut()) {
            (Ok(handler), Some(pane)) if wanted => pane.took(&path, handler),
            (Ok(handler), _) => unsafe {
                let _ = handler.Unload();
            },
            (Err(_), _) => {
                let shown = self.shown_path() == Some(path.as_path());
                let read = match self.inside {
                    Some(member) => self.member_read.get_mut(member),
                    None => self.read.get_mut(self.at),
                };
                if let Some(read) = read.and_then(Option::as_mut).filter(|_| shown) {
                    if let Body::Live { picture, .. } = &read.body {
                        read.body = match picture {
                            Some(picture) => Body::Pictured(picture.clone()),
                            None => Body::Other(Some(path)),
                        };
                        // (Sized anew for what it shows now.)
                        let now = Instant::now();
                        self.motion = Motion::Moving { was: self.shape(now).at, start: now, fade: false };
                        unsafe {
                            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
                        }
                    }
                }
            }
        }
        self.draw();
    }

    /// Its clicks taken only where its sheet is drawn, `at` (DIPs), and the room for its shadow round that: elsewhere
    /// they go to what is under it.
    fn take_clicks(&mut self, at: Rect) {
        let s = self.scale;
        let region = RECT {
            left: ((at.x - SHADOW) * s).floor() as i32,
            top: ((at.y - SHADOW) * s).floor() as i32,
            right: ((at.x + at.w + SHADOW) * s).ceil() as i32,
            bottom: ((at.y + at.h + SHADOW) * s).ceil() as i32,
        };
        if region == self.region {
            return;
        }
        self.region = region;
        unsafe {
            let shape = CreateRectRgn(region.left, region.top, region.right, region.bottom);
            // (The region the system's from here on: not to be let go of here.)
            SetWindowRgn(self.hwnd, Some(shape), false);
        }
    }

    /// The sheet drawn as `shape` says (over the screen frosted behind it, `frosted`).
    fn paint(&mut self, frame: &Frame, frosted: bool, shape: Shape) {
        frame.crisp_text();
        let (w, h) = shape.size;
        let (sx, sy) = (shape.at.w / w.max(1.0), shape.at.h / h.max(1.0));
        let placed = Matrix3x2::scale(sx, sy) * Matrix3x2::translation(shape.at.x, shape.at.y);
        frame.place(placed);
        let _moving = frame.moving_while(!matches!(self.motion, Motion::Resting));
        self.hits.clear();
        faded(frame, shape.opacity, || {
            let c = self.colours(frosted);
            self.paint_sheet(frame, frosted, &c, (w, h), placed);
            let theme = Theme::new(self.skin, self.dark);
            self.paint_bar(frame, &theme, w);
            faded(frame, shape.body, || self.paint_body(frame, &theme, (w, h)));
        });
    }

    fn paint_sheet(&self, frame: &Frame, frosted: bool, c: &Colours, (w, h): (f32, f32), placed: Matrix3x2) {
        if self.skin == Skin::Glass && frosted {
            let theme = Theme::new(Skin::Glass, self.dark);
            if let Err(error) = super::skins::glass_sheet(frame, &theme, Rect { x: 0.0, y: 0.0, w, h }, c.radius, self.frost) {
                gfx::failed("look", Some(&error));
            }
            frame.place(placed);
        } else {
            frame.shadow(c.shadow, 0.0, 0.0, w, h, c.radius, 16.0, 8.0);
            frame.fill_rounded(c.sheet, 0.0, 0.0, w, h, c.radius);
            // (Windows 11's bar a little deeper than its sheet, as its footer strip is.)
            if self.skin == Skin::Fluent {
                let theme = Theme::new(Skin::Fluent, self.dark);
                frame.clip(0.0, 0.0, w, self.bar());
                frame.fill_rounded(theme.footer, 0.0, 0.0, w, h, c.radius);
                frame.unclip();
            }
            frame.stroke_rounded(Fill::Solid(c.stroke), 0.0, 0.0, w, h, c.radius, 1.0);
            if let Some(rim) = c.rim {
                frame.stroke_rounded(Fill::Down { top: 0.0, from: rim, bottom: h, to: rim.alpha(rim.a * 0.3) }, 1.0, 1.0, w - 2.0, h - 2.0, c.radius - 1.0, 1.0);
            }
        }
        if self.skin == Skin::Paper {
            frame.fill(c.rule, 0.0, self.bar(), w, 1.0);
        }
    }

    /// Its bar: what the thing is called and what it is, and its buttons (copy; open, or paste).
    fn paint_bar(&mut self, frame: &Frame, theme: &Theme, w: f32) {
        let lang = self.lang;
        let item = &self.items[self.at];
        let (title, small) = (self.font(13.0, 650.0), self.font(11.5, 400.0));
        // (A file of the group by itself: its name, what it is once read, and its own buttons.)
        let member = self.inside.and_then(|member| Some((member, self.members()?.get(member)?.name.clone(), self.members()?.len())));
        let (title_text, about, open_label, pastes, opens, copies) = match &member {
            Some((_, name, _)) => (name.clone(), self.shown_read().and_then(|read| read.about.clone()).unwrap_or_default(), lang.pick("打开", "Open"), false, true, true),
            None => (
                item.title.clone(),
                self.read.get(self.at).and_then(|read| read.as_ref()?.about.clone()).unwrap_or_else(|| item.about.clone()),
                if item.pastes { lang.pick("粘贴", "Paste") } else { lang.pick("打开", "Open") },
                item.pastes,
                item.open.is_some(),
                item.copy.is_some(),
            ),
        };
        // Its buttons, from the right, as far in from its edge as its body is.
        let font = self.font(12.5, 600.0);
        let pad = self.inset();
        let mut right = w - pad;
        let told = self.told.filter(|(_, at)| at.elapsed() < TOLD_FOR).map(|(hit, _)| hit);
        let accent = match self.skin {
            Skin::Paper => theme.text,
            _ => theme::accent(self.dark),
        };
        let on_accent = if luminance(accent) > 0.55 { Color::hex(0x000000, 0.9) } else { Color::hex(0xFFFFFF, 1.0) };
        let mut buttons = Vec::new();
        if opens {
            let label = if told == Some(Hit::Open) { if pastes { lang.pick("已粘贴", "Pasted") } else { lang.pick("已打开", "Opened") } } else { open_label };
            buttons.push((Hit::Open, label, if pastes { Icon::ClipboardPaste } else { Icon::ExternalLink }, true));
        }
        if copies {
            let label = if told == Some(Hit::Copy) { lang.pick("已复制", "Copied") } else { lang.pick("复制", "Copy") };
            buttons.push((Hit::Copy, label, Icon::Copy, false));
        }
        let radius = match self.skin {
            Skin::Glass => BUTTON / 2.0,
            _ => theme.control_radius.clamp(4.0, BUTTON / 2.0),
        };
        for (hit, label, icon, main) in buttons {
            let wide = (frame.measure(label, font) + 14.0 + 6.0 + 22.0).ceil();
            right -= wide;
            let at = Rect { x: right, y: pad + (NAME - BUTTON) / 2.0, w: wide, h: BUTTON };
            let lit = self.pointer_on(at) && self.pressed.is_none_or(|pressed| pressed == hit);
            // (Glass's each a pill of its glass, as its buttons on the panel are; Windows 11's main one in its accent and the
            // other plain, as its own are; chart paper's main one in its ink.)
            let ink = if self.skin == Skin::Glass {
                super::view::glass_pill(frame, theme, (at.x, at.y, at.w, at.h), radius, lit);
                theme.text
            } else {
                let (fill, ink) = if main { (accent, on_accent) } else { (theme.track, theme.text) };
                frame.fill_rounded(fill, at.x, at.y, at.w, at.h, radius);
                if lit {
                    frame.fill_rounded(if main { Color::hex(0xFFFFFF, if self.dark { 0.12 } else { 0.16 }) } else { theme.hover }, at.x, at.y, at.w, at.h, radius);
                }
                if !main {
                    frame.stroke_rounded(Fill::Solid(theme.rule), at.x, at.y, at.w, at.h, radius, 1.0);
                }
                ink
            };
            frame.icon(icon, Point { x: at.x + 11.0 + 7.0, y: at.y + BUTTON / 2.0 }, 14.0, ink);
            let (ascent, descent) = frame.gfx.baseline(font);
            frame.text(label, font, ink, at.x + 11.0 + 14.0 + 6.0, at.y + (BUTTON - ascent - descent) / 2.0, wide, Align::Start);
            self.hits.push((at, hit));
            right -= 8.0;
        }
        // Which of how many it is, before them (of its group's files, one of them by itself).
        let (place, count) = match member {
            Some((member, _, count)) => (member, count),
            None => (self.at, self.items.len()),
        };
        if count > 1 {
            let said = format!("{} / {}", place + 1, count);
            let font = self.font(11.5, 400.0);
            let wide = frame.measure(&said, font);
            let (ascent, descent) = frame.gfx.baseline(font);
            right -= 4.0 + wide;
            frame.text(&said, font, theme.text3, right, pad + (NAME - ascent - descent) / 2.0, wide + 1.0, Align::Start);
            right -= 12.0;
        }
        // A file of the group by itself: the button back to the group first, as its buttons are.
        let mut left = pad;
        if self.inside.is_some() {
            let at = Rect { x: pad, y: pad + (NAME - BUTTON) / 2.0, w: BUTTON, h: BUTTON };
            let lit = self.pointer_on(at) && self.pressed.is_none_or(|pressed| pressed == Hit::Back);
            if self.skin == Skin::Glass {
                super::view::glass_pill(frame, theme, (at.x, at.y, at.w, at.h), radius, lit);
            } else {
                frame.fill_rounded(if lit { theme.hover } else { theme.track }, at.x, at.y, at.w, at.h, radius);
                frame.stroke_rounded(Fill::Solid(theme.rule), at.x, at.y, at.w, at.h, radius, 1.0);
            }
            frame.icon(Icon::ChevronLeft, Point { x: at.x + BUTTON / 2.0, y: at.y + BUTTON / 2.0 }, 16.0, theme.text);
            self.hits.push((at, Hit::Back));
            left += BUTTON + 10.0;
        }
        // What it is called, and what it is, under that, as far down from its top as from its side.
        let room = (right - left - 8.0).max(0.0);
        let top = pad;
        frame.text(&title_text, title, theme.text, left, top, room, Align::Start);
        frame.text(&about, small, theme.text3, left, top + 18.0, room, Align::Start);
    }

    /// Where its body is on a sheet laid out `(w, h)` large (DIPs on the sheet).
    fn body_at(&self, (w, h): (f32, f32)) -> Rect {
        let (top, pad) = (self.body_top(), self.inset());
        Rect { x: pad, y: top, w: (w - 2.0 * pad).max(0.0), h: (h - top - pad).max(0.0) }
    }

    /// How round the corners of what its body shows are (DIPs): a third as round as its own, chart paper's square.
    fn body_radius(&self) -> f32 {
        match self.skin {
            Skin::Paper => 0.0,
            _ => (self.colours(true).radius / 3.0).max(4.0),
        }
    }

    /// Its body: what the thing shows (files together: their pictures in rows; one of them by itself: it).
    fn paint_body(&mut self, frame: &Frame, theme: &Theme, (w, h): (f32, f32)) {
        let body = self.body_at((w, h));
        self.hits.push((body, Hit::Body));
        if let Motion::Into { .. } | Motion::OutOf { .. } = self.motion {
            return self.paint_morph(frame, theme, body);
        }
        if self.inside.is_none() && self.members().is_some() {
            return self.paint_group(frame, theme, body, None);
        }
        let (read, path, title, opens) = match self.inside {
            Some(member) => {
                let member_of = self.members().and_then(|members| members.get(member)).cloned();
                let read = self.member_read.get(member).cloned().flatten();
                (read, member_of.as_ref().map(|member| member.path.clone()), member_of.map(|member| member.name).unwrap_or_default(), true)
            }
            None => {
                let item = &self.items[self.at];
                (self.read.get(self.at).cloned().flatten(), self.shown_path().map(Path::to_path_buf), item.title.clone(), item.open.is_some())
            }
        };
        self.paint_read(frame, theme, body, read, path, &title, opens);
    }

    /// The files of the group shown in `body`, each its picture in its place (see `group_cells`) with its name under it,
    /// as far as they are scrolled; but `apart`'s (grown out of its place: drawn by itself).
    fn paint_group(&mut self, frame: &Frame, theme: &Theme, body: Rect, apart: Option<usize>) {
        let Some(members) = self.members().map(<[Member]>::to_vec) else { return };
        let (cells, tall) = group_cells(&shapes(&members), self.group_inner());
        if matches!(self.motion, Motion::Resting | Motion::Moving { .. }) {
            self.grid_scroll_most = (tall - body.h).max(0.0);
            self.grid_scroll = self.grid_scroll.clamp(0.0, self.grid_scroll_most);
        }
        let (radius, name_font) = (self.body_radius(), self.font(11.5, 400.0));
        let resting = matches!(self.motion, Motion::Resting);
        // (Room past the body's sides for the ring round a picture at its edge.)
        frame.clip(body.x - 4.0, body.y, body.w + 8.0, body.h);
        for (place, (member, cell)) in members.iter().zip(&cells).enumerate() {
            let at = Rect { x: body.x + cell.x, y: body.y + cell.y - self.grid_scroll, w: cell.w, h: cell.h };
            if at.y + at.h + GROUP_NAME < body.y || at.y > body.y + body.h || apart == Some(place) {
                continue;
            }
            // (Its picture made for as many px as it takes, of two sizes: made once for either.)
            let side = if at.w.max(at.h) * self.scale > 256.0 { 512 } else { 256 };
            let thumb = crate::station::thumbs::of(&member.path, side);
            match (member.seen, thumb) {
                (Some(_), Some(thumb)) => {
                    let (x, y, pw, ph) = thumb.fitted_in(at.x, at.y, at.w, at.h);
                    frame.picture(&thumb, x, y, pw, ph, radius, 1.0);
                }
                // (No picture of it yet, or none of its own: its square, and in it Explorer's picture of it.)
                (seen, thumb) => {
                    frame.fill_rounded(theme.track, at.x, at.y, at.w, at.h, radius);
                    let thumb = thumb.filter(|_| seen.is_none());
                    if let Some(thumb) = thumb.or_else(|| crate::station::thumbs::icon(&member.path, (48.0 * self.scale).round() as u32)) {
                        let inner = (at.w.min(at.h) - 24.0).clamp(0.0, 96.0);
                        let (x, y, pw, ph) = thumb.fitted_in(at.x + (at.w - inner) / 2.0, at.y + (at.h - inner) / 2.0, inner, inner);
                        frame.picture(&thumb, x, y, pw, ph, 0.0, 1.0);
                    }
                }
            }
            let lit = resting && self.pointer_on(at) && self.pressed.is_none_or(|pressed| pressed == Hit::Cell(place));
            let ring = if self.picked == Some(place) { Some(theme::accent(self.dark)) } else { lit.then(|| theme.text.alpha(0.3)) };
            if let Some(ring) = ring {
                frame.stroke_rounded(Fill::Solid(ring), at.x - 2.0, at.y - 2.0, at.w + 4.0, at.h + 4.0, radius + 2.0, 2.0);
            }
            frame.text(&member.name, name_font, theme.text3, at.x, at.y + at.h + 4.0, at.w, Align::Start);
            // (Clicked only where it shows in the body.)
            let (top, bottom) = (at.y.max(body.y), (at.y + at.h).min(body.y + body.h));
            if bottom > top {
                self.hits.push((Rect { x: at.x, y: top, w: at.w, h: bottom - top }, Hit::Cell(place)));
            }
        }
        frame.unclip();
    }

    /// A file of the group growing out of its place or shrinking back into it (see `Motion::Into`, `Motion::OutOf`): its
    /// picture between its place and where it shows by itself, the others fading out as it grows and in as it shrinks.
    /// One without a picture shows as it is read, faded in or out over them.
    fn paint_morph(&mut self, frame: &Frame, theme: &Theme, body: Rect) {
        let now = Instant::now();
        let part = |start: Instant| MOVE_EASE.at((now.duration_since(start).as_secs_f32() / INTO_FOR.as_secs_f32()).clamp(0.0, 1.0));
        let (member, into, p, from, cell) = match self.motion {
            Motion::Into { member, cell, start, .. } => (member, true, part(start), None, cell),
            Motion::OutOf { member, from, cell, start, .. } => (member, false, part(start), Some(from), cell),
            _ => return,
        };
        let others = if into { (1.0 - p / 0.6).max(0.0) } else { ((p - 0.3) / 0.7).clamp(0.0, 1.0) };
        faded(frame, others, || self.paint_group(frame, theme, body, Some(member)));
        let Some(member_of) = self.members().and_then(|members| members.get(member)).cloned() else { return };
        let read = self.member_read.get(member).cloned().flatten();
        if member_of.seen.is_none() {
            let shown = if into { p } else { 1.0 - p };
            return faded(frame, shown, || self.paint_read(frame, theme, body, read, Some(member_of.path), &member_of.name, true));
        }
        let rect = match from {
            Some(from) => between(from, cell, p),
            None => {
                let target = self.target();
                between(cell, self.member_fit(member, self.body_at((target.w, target.h))), p)
            }
        };
        let picture = read.and_then(|read| match read.body {
            Body::Picture(picture, _) | Body::Pictured(picture) | Body::Live { picture: Some(picture), .. } | Body::Played { picture: Some(picture), sound: false, .. } => Some(picture),
            _ => None,
        });
        if let Some(picture) = picture.or_else(|| crate::station::thumbs::of(&member_of.path, 512)) {
            frame.picture(&picture, rect.x, rect.y, rect.w, rect.h, self.body_radius(), 1.0);
        }
    }

    /// What a thing (or a file of a group) shows in `body`, as it was `read` (not yet: its picture, Explorer's, where
    /// it has one); `title` what it is called, `opens` whether it may be opened.
    #[allow(clippy::too_many_arguments)]
    fn paint_read(&mut self, frame: &Frame, theme: &Theme, body: Rect, read: Option<Read>, path: Option<PathBuf>, title: &str, opens: bool) {
        let radius = self.body_radius();
        let Some(read) = read else {
            // (Being read: its tile's picture, where it has one, else nothing yet.)
            if let Some(thumb) = path.and_then(|path| crate::station::thumbs::of(&path, 256)) {
                let (x, y, pw, ph) = thumb.fitted_in(body.x, body.y, body.w, body.h);
                frame.picture(&thumb, x, y, pw, ph, radius, 1.0);
            }
            return;
        };
        match &read.body {
            Body::Picture(picture, _) | Body::Pictured(picture) => {
                let fitted = picture.fitted_in(body.x, body.y, body.w, body.h);
                let (pw, ph) = (fitted.2 * self.zoom, fitted.3 * self.zoom);
                let (cx, cy) = (fitted.0 + fitted.2 / 2.0 + self.pan.0, fitted.1 + fitted.3 / 2.0 + self.pan.1);
                let at = (cx - pw / 2.0, cy - ph / 2.0, pw, ph);
                // (Zoomed in, it shows only within the body; fitted, as the plate it is.)
                let within = if self.zoom > 1.0 { (body.x, body.y, body.w, body.h, radius) } else { (at.0, at.1, at.2, at.3, radius) };
                frame.picture_within(picture, at, within, 1.0);
                if self.skin == Skin::Paper && self.zoom <= 1.0 {
                    frame.stroke_rounded(Fill::Solid(theme.rule), at.0, at.1, at.2, at.3, 0.0, 1.0);
                }
            }
            // (Its previewer over it at rest: its picture as it moves, and while the previewer is made.)
            Body::Live { picture, .. } => {
                if let Some(picture) = picture {
                    let (x, y, pw, ph) = picture.fitted_in(body.x, body.y, body.w, body.h);
                    frame.picture(picture, x, y, pw, ph, radius, 1.0);
                }
            }
            Body::Words { text, fixed, cut } => {
                let font = if *fixed { Font::new(Family::Mono, 12.5, 400.0) } else { self.font(14.0, 400.0) };
                let mut words = text.clone();
                if *cut {
                    words.push_str(self.lang.pick("\n\n……（只显示开头部分）", "\n\n… (only its beginning is shown)"));
                }
                let (inner, x) = if *fixed {
                    frame.fill_rounded(theme.track, body.x, body.y, body.w, body.h, if self.skin == Skin::Paper { 3.0 } else { radius });
                    (body.w - 24.0, body.x + 12.0)
                } else {
                    (body.w, body.x)
                };
                let margin = if *fixed { 10.0 } else { 0.0 };
                let tall = frame.gfx.wrapped_height(&words, font, inner) + 2.0 * margin;
                self.scroll_most = (tall - body.h).max(0.0);
                self.scroll = self.scroll.clamp(0.0, self.scroll_most);
                frame.clip(body.x, body.y, body.w, body.h);
                frame.text_wrapped(&words, font, theme.text, x, body.y + margin - self.scroll, inner);
                frame.unclip();
            }
            // (Drawn as `paint_group`: a group's files are each read as a file.)
            Body::Group(_) => {}
            Body::Played { picture, sound: false, .. } => self.paint_video(frame, body, picture.as_ref()),
            Body::Played { picture, sound: true, loudness } => self.paint_sound(frame, theme, body, picture.as_ref(), loudness.as_deref()),
            Body::Other(path) => {
                let side = 72.0;
                let mut y = body.y + (body.h - side - 70.0).max(0.0) / 2.0;
                let icon = path.as_ref().and_then(|path| crate::station::thumbs::icon(path, (side * self.scale).round() as u32));
                match icon {
                    Some(icon) => frame.picture(&icon, body.x + (body.w - side) / 2.0, y, side, side, 0.0, 1.0),
                    None => frame.icon(Icon::File, Point { x: body.x + body.w / 2.0, y: y + side / 2.0 }, side * 0.7, theme.text2),
                }
                y += side + 10.0;
                // (Each line under the middle of its icon; one wider than the body from its left, wrapped.)
                let centred = |text: &str, font: Font, colour, top: f32| {
                    let wide = frame.measure(text, font);
                    let left = body.x + ((body.w - wide) / 2.0).max(0.0);
                    frame.text(text, font, colour, left, top, body.w - (left - body.x), Align::Start);
                };
                centred(title, self.font(13.0, 600.0), theme.text, y);
                if let Some(about) = &read.about {
                    centred(about, self.font(12.0, 400.0), theme.text2, y + 22.0);
                }
                if opens {
                    let said = self.lang.pick("没有可用的预览，按 Enter 用默认程序打开", "No preview is available. Press Enter to open it in its program");
                    centred(said, self.font(12.0, 400.0), theme.text3, y + 42.0);
                }
            }
        }
    }

    /// A video in `body`: its latest frame (else the picture standing for it), fitted, on black; paused, a play button
    /// over its middle; its controls along its bottom (see `paint_strip`), as clear as they are now.
    fn paint_video(&mut self, frame: &Frame, body: Rect, picture: Option<&Picture>) {
        let radius = self.body_radius();
        frame.fill_rounded(Color::hex(0x000000, 1.0), body.x, body.y, body.w, body.h, radius);
        match (self.player.as_mut().and_then(|player| player.frame(&frame.dc)), picture) {
            (Some((bitmap, size)), _) => {
                let at = fit_in(size, body);
                frame.bitmap_within(&bitmap, size, (at.x, at.y, at.w, at.h), (at.x, at.y, at.w, at.h, radius), 1.0);
            }
            (None, Some(picture)) => {
                let (x, y, w, h) = picture.fitted_in(body.x, body.y, body.w, body.h);
                frame.picture(picture, x, y, w, h, radius, 1.0);
            }
            (None, None) => {}
        }
        if self.unplayable {
            let said = self.lang.pick("无法在此播放该文件，按 Enter 用默认程序打开", "This file cannot be played here. Press Enter to open it in its program");
            let font = self.font(12.5, 500.0);
            let wide = frame.measure(said, font).min(body.w - 24.0);
            let (x, y) = (body.x + (body.w - wide - 24.0) / 2.0, body.y + body.h - 16.0 - 32.0);
            frame.fill_rounded(Color::hex(0x101216, 0.72), x, y, wide + 24.0, 32.0, 16.0);
            let (ascent, descent) = frame.gfx.baseline(font);
            frame.text(said, font, Color::hex(0xFFFFFF, 1.0), x + 12.0, y + (32.0 - ascent - descent) / 2.0, wide, Align::Start);
            return;
        }
        let Some(playing) = self.player.as_ref().map(play::Player::playing) else { return };
        self.hits.push((body, Hit::Toggle));
        let white = Color::hex(0xFFFFFF, 1.0);
        if !playing {
            let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
            frame.fill_rounded(Color::hex(0x101216, 0.62), cx - 30.0, cy - 30.0, 60.0, 60.0, 30.0);
            frame.stroke_rounded(Fill::Solid(white.alpha(0.09)), cx - 30.0, cy - 30.0, 60.0, 60.0, 30.0, 1.0);
            frame.icon_solid(Icon::Play, Point { x: cx + 2.0, y: cy }, 20.0, white);
        }
        let clear = self.controls_clear(Instant::now());
        if clear > 0.0 {
            faded(frame, clear, || self.paint_strip(frame, body, playing));
        } else {
            (self.strip_at, self.track_at, self.loud_at, self.slider_at) = (None, None, None, None);
        }
    }

    /// A video's controls, along the bottom of `body`: played or paused, where it is, its track, how long it is, how
    /// loud.
    fn paint_strip(&mut self, frame: &Frame, body: Rect, playing: bool) {
        let strip = Rect { x: body.x + STRIP_IN, y: body.y + body.h - STRIP_IN - STRIP, w: (body.w - 2.0 * STRIP_IN).max(0.0), h: STRIP };
        let white = Color::hex(0xFFFFFF, 1.0);
        frame.fill_rounded(Color::hex(0x101216, 0.62), strip.x, strip.y, strip.w, strip.h, STRIP / 2.0);
        frame.stroke_rounded(Fill::Solid(white.alpha(0.09)), strip.x, strip.y, strip.w, strip.h, STRIP / 2.0, 1.0);
        self.strip_at = Some(strip);
        self.hits.push((strip, Hit::Strip));
        let toggle = Rect { x: strip.x + 5.0, y: strip.y + 5.0, w: 28.0, h: 28.0 };
        self.round_button(frame, toggle, if playing { Icon::Pause } else { Icon::Play }, white, Hit::Toggle);
        let font = self.font(12.0, 500.0);
        let clock_wide = 44.0;
        let (at, length) = (self.shown_at(), self.player.as_ref().and_then(play::Player::length));
        let mid = strip.y + STRIP / 2.0;
        self.centred_text(frame, &clock(at), font, white.alpha(0.86), toggle.x + toggle.w + 4.0, mid, clock_wide);
        let mute = self.paint_volume(frame, strip.x + strip.w - 10.0, false, mid, white, white.alpha(0.22));
        let length_x = mute - 6.0 - clock_wide;
        self.centred_text(frame, &length.map_or_else(|| "--:--".into(), clock), font, white.alpha(0.86), length_x, mid, clock_wide);
        let start = toggle.x + toggle.w + 4.0 + clock_wide + 6.0;
        let track = Rect { x: start, y: mid - 9.0, w: (length_x - 6.0 - start).max(0.0), h: 18.0 };
        let part = length.map_or(0.0, |length| (at / length).clamp(0.0, 1.0)) as f32;
        let lit = self.holding.is_some_and(|held| matches!(held, Holding::Track(_))) || self.pointer_on(track);
        let thick = if lit { 6.0 } else { 4.0 };
        frame.fill_rounded(white.alpha(0.22), track.x, mid - thick / 2.0, track.w, thick, thick / 2.0);
        frame.fill_rounded(white, track.x, mid - thick / 2.0, track.w * part, thick, thick / 2.0);
        if lit {
            frame.fill_rounded(white, track.x + track.w * part - KNOB / 2.0, mid - KNOB / 2.0, KNOB, KNOB, KNOB / 2.0);
            self.paint_time_tip(frame, track, strip.y - 6.0, length);
        }
        self.track_at = Some(track);
        self.hits.push((track, Hit::Track));
    }

    /// The volume's button, and its slider opened to its right as the pointer is on them: from `from` (DIPs on the
    /// sheet) rightwards, or (`rightwards` not) ending there, centred on `mid`, `ink` on `rail`. Where its button
    /// begins (its left edge).
    fn paint_volume(&mut self, frame: &Frame, from: f32, rightwards: bool, mid: f32, ink: Color, rail: Color) -> f32 {
        let open = self.slider_open_now(Instant::now());
        let wide = SLIDER * open;
        let left = if rightwards { from } else { from - wide - 4.0 * open - 28.0 };
        let mute = Rect { x: left, y: mid - 14.0, w: 28.0, h: 28.0 };
        let slider = Rect { x: mute.x + 28.0 + 4.0 * open, y: mid - 9.0, w: wide, h: 18.0 };
        let silent = self.muted || self.volume <= 0.0;
        self.round_button(frame, mute, if silent { Icon::Muted } else { Icon::Volume }, ink, Hit::Mute);
        if wide > 1.0 {
            frame.clip(slider.x, slider.y, slider.w, slider.h);
            let (start, long) = (slider.x + KNOB / 2.0, SLIDER - KNOB);
            let loud = long * if silent { 0.0 } else { self.volume };
            frame.fill_rounded(rail, start, mid - 2.0, long, 4.0, 2.0);
            frame.fill_rounded(ink, start, mid - 2.0, loud, 4.0, 2.0);
            if self.holding == Some(Holding::Volume) || self.pointer_on(slider) {
                frame.fill_rounded(ink, start + loud - KNOB / 2.0, mid - KNOB / 2.0, KNOB, KNOB, KNOB / 2.0);
            }
            frame.unclip();
            self.hits.push((slider, Hit::Volume));
        }
        self.slider_at = Some(Rect { x: slider.x, y: slider.y, w: SLIDER, h: slider.h });
        self.loud_at = Some(Rect { x: mute.x, y: mid - STRIP / 2.0, w: slider.x + slider.w - mute.x, h: STRIP });
        mute.x
    }

    /// A button at `at` with `icon` in `ink`, a little dimmer till the pointer is on it (played or paused: solid):
    /// `hit` as it is clicked.
    fn round_button(&mut self, frame: &Frame, at: Rect, icon: Icon, ink: Color, hit: Hit) {
        let ink = ink.alpha(ink.a * if self.pointer_on(at) { 1.0 } else { 0.78 });
        let centre = Point { x: at.x + at.w / 2.0, y: at.y + at.h / 2.0 };
        match icon {
            Icon::Play => frame.icon_solid(icon, Point { x: centre.x + 1.0, ..centre }, 13.0, ink),
            Icon::Pause => frame.icon_solid(icon, centre, 13.0, ink),
            _ => frame.icon(icon, centre, 16.0, ink),
        }
        self.hits.push((at, hit));
    }

    /// `text` in the middle of `wide` from `x`, its line's middle on `mid`.
    fn centred_text(&self, frame: &Frame, text: &str, font: Font, ink: Color, x: f32, mid: f32, wide: f32) {
        let (ascent, descent) = frame.gfx.baseline(font);
        let w = frame.measure(text, font);
        frame.text(text, font, ink, x + ((wide - w) / 2.0).max(0.0), mid - (ascent + descent) / 2.0, wide, Align::Start);
    }

    /// The time under the pointer (or where the track is held) along `track`, in a tip whose bottom is at `bottom`.
    fn paint_time_tip(&self, frame: &Frame, track: Rect, bottom: f32, length: Option<f64>) {
        let Some(length) = length else { return };
        let x = match (self.holding, self.pointer) {
            (Some(Holding::Track(seconds)), _) => track.x + track.w * (seconds / length) as f32,
            (_, Some((x, _))) => x.clamp(track.x, track.x + track.w),
            _ => return,
        };
        let said = clock(f64::from((x - track.x) / track.w.max(1.0)) * length);
        let font = self.font(11.5, 500.0);
        let wide = frame.measure(&said, font) + 14.0;
        frame.fill_rounded(Color::hex(0x000000, 0.75), x - wide / 2.0, bottom - 20.0, wide, 20.0, 6.0);
        self.centred_text(frame, &said, font, Color::hex(0xFFFFFF, 1.0), x - wide / 2.0, bottom - 10.0, wide);
    }

    /// A sound in `body`: its cover (else a note) at its left; to its right, its loudness along it (what is played of it
    /// in the accent), where it is and how long it is, played or paused, how loud.
    fn paint_sound(&mut self, frame: &Frame, theme: &Theme, body: Rect, picture: Option<&Picture>, loudness: Option<&[f32]>) {
        let radius = self.body_radius();
        let side = body.h.min(body.w / 2.0);
        match picture {
            Some(picture) => frame.picture_within(picture, (body.x, body.y, side, side), (body.x, body.y, side, side, radius), 1.0),
            None => {
                frame.fill_rounded(theme.track, body.x, body.y, side, side, radius);
                frame.icon(Icon::Music, Point { x: body.x + side / 2.0, y: body.y + side / 2.0 }, side * 0.32, theme.text3);
            }
        }
        let x = body.x + side + 16.0;
        let wide = (body.x + body.w - x).max(0.0);
        let wave = Rect { x, y: body.y + 6.0, w: wide, h: 64.0 };
        let (at, length) = (self.shown_at(), self.player.as_ref().and_then(play::Player::length));
        let part = length.map_or(0.0, |length| (at / length).clamp(0.0, 1.0)) as f32;
        let accent = theme::accent(self.dark);
        // (Its loudness in bars, each the loudest of its parts; not read yet, a line.)
        match loudness.filter(|loudness| !loudness.is_empty()) {
            Some(loudness) => {
                let bars = ((wide / 4.0).floor() as usize).max(1);
                let step = wide / bars as f32;
                // (Drawn twice: as yet unplayed, then in the accent as far as it is played, to the pixel, a bar
                // coloured as it is gone through.)
                let played = wide * part;
                for (ink, until) in [(theme.text.alpha(0.28), wide), (accent, played)] {
                    if until <= 0.0 {
                        continue;
                    }
                    frame.clip(wave.x, wave.y, until, wave.h);
                    for bar in 0..bars {
                        let (from, to) = (bar * loudness.len() / bars, ((bar + 1) * loudness.len() / bars).max(bar * loudness.len() / bars + 1));
                        let loud = loudness[from.min(loudness.len() - 1)..to.min(loudness.len())].iter().copied().fold(0f32, f32::max);
                        let tall = (loud * wave.h).max(2.0);
                        frame.fill_rounded(ink, wave.x + bar as f32 * step + (step - 2.0) / 2.0, wave.y + (wave.h - tall) / 2.0, 2.0, tall, 1.0);
                    }
                    frame.unclip();
                }
                // (And a line where it is, across it: it moves on as it plays, between the bars too.)
                if part > 0.0 {
                    frame.fill_rounded(accent, wave.x + played - 1.0, wave.y, 2.0, wave.h, 1.0);
                }
            }
            None => {
                frame.fill_rounded(theme.text.alpha(0.28), wave.x, wave.y + wave.h / 2.0 - 1.0, wave.w, 2.0, 1.0);
                frame.fill_rounded(accent, wave.x, wave.y + wave.h / 2.0 - 1.0, wave.w * part, 2.0, 1.0);
            }
        }
        let font = self.font(12.0, 500.0);
        let below = wave.y + wave.h + 6.0;
        if self.unplayable {
            let said = self.lang.pick("无法在此播放该文件，按 Enter 用默认程序打开", "This file cannot be played here. Press Enter to open it in its program");
            frame.text(said, font, theme.text3, x, below, wide, Align::Start);
            return;
        }
        let Some(playing) = self.player.as_ref().map(play::Player::playing) else { return };
        if self.pointer_on(wave) || matches!(self.holding, Some(Holding::Track(_))) {
            self.paint_time_tip(frame, wave, wave.y - 2.0, length);
        }
        self.track_at = Some(wave);
        self.hits.push((wave, Hit::Track));
        frame.text(&clock(at), font, theme.text2, x, below, wide, Align::Start);
        frame.text(&length.map_or_else(|| "--:--".into(), clock), font, theme.text2, x, below, wide, Align::End);
        let button = Rect { x, y: body.y + body.h - 40.0, w: 40.0, h: 40.0 };
        let lit = self.pointer_on(button);
        frame.fill_rounded(theme.text.alpha(if lit { 1.0 } else { 0.9 }), button.x, button.y, 40.0, 40.0, 20.0);
        let ink = Color::hex(if self.dark { 0x14161A } else { 0xFFFFFF }, 1.0);
        frame.icon_solid(if playing { Icon::Pause } else { Icon::Play }, Point { x: button.x + 20.0 + if playing { 0.0 } else { 1.5 }, y: button.y + 20.0 }, 15.0, ink);
        self.hits.push((button, Hit::Toggle));
        self.paint_volume(frame, button.x + 40.0 + 10.0, true, button.y + 20.0, theme.text, theme.text.alpha(0.18));
    }

    // ----- The pointer -----

    /// The pointer on the window (px in it) on the sheet (DIPs), as it rests there.
    fn on_sheet(&self, (x, y): (i32, i32)) -> (f32, f32) {
        let at = self.shape(Instant::now()).at;
        (x as f32 / self.scale - at.x, y as f32 / self.scale - at.y)
    }

    fn pointer_on(&self, at: Rect) -> bool {
        self.pointer.is_some_and(|(x, y)| x >= at.x && x < at.x + at.w && y >= at.y && y < at.y + at.h)
    }

    fn hit_at(&self, (x, y): (f32, f32)) -> Option<Hit> {
        self.hits.iter().rev().find(|(at, _)| x >= at.x && x < at.x + at.w && y >= at.y && y < at.y + at.h).map(|(_, hit)| *hit)
    }

    /// Whether the picture shown may be zoomed or moved: a picture, at rest.
    fn pictured(&self) -> bool {
        matches!(self.body(), Some(Body::Picture(..) | Body::Pictured(..))) && matches!(self.motion, Motion::Resting)
    }

    fn moved(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        self.pointer = Some(on);
        self.moved_over_player(on);
        if let Some((from, pan)) = self.dragging {
            self.pan = (pan.0 + on.0 - from.0, pan.1 + on.1 - from.1);
            self.keep_in_view();
        }
        self.draw();
    }

    fn press(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        self.pressed = self.hit_at(on);
        match self.pressed {
            Some(Hit::Track) => self.track_to(on.0),
            Some(Hit::Volume) => {
                self.holding = Some(Holding::Volume);
                self.volume_to(on.0);
            }
            _ => {}
        }
        if self.pressed == Some(Hit::Body) && self.pictured() && self.zoom > 1.0 {
            self.dragging = Some((on, self.pan));
        }
        unsafe {
            SetCapture(self.hwnd);
        }
        self.draw();
    }

    fn release(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        let pressed = self.pressed.take();
        self.dragging = None;
        let _ = unsafe { ReleaseCapture() };
        match self.holding.take() {
            Some(Holding::Track(seconds)) => {
                if let Some(player) = &self.player {
                    player.seek(seconds);
                }
            }
            Some(Holding::Volume) => self.keep_volume(),
            None => {}
        }
        match pressed.filter(|pressed| self.hit_at(on) == Some(*pressed)) {
            Some(Hit::Open) => self.key(Key::Open),
            Some(Hit::Copy) => self.key(Key::Copy),
            Some(Hit::Cell(member)) => self.enter(member),
            Some(Hit::Back) => self.leave(),
            Some(Hit::Toggle) => self.key(Key::Toggle),
            Some(Hit::Mute) => {
                self.muted = !self.muted;
                if let Some(player) = &self.player {
                    player.set_volume(self.loudness());
                }
                self.keep_volume();
                self.draw();
            }
            _ => self.draw(),
        }
    }

    /// A double click: a picture between fitted and its own pixels (twice as large at the least).
    fn double(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        if !self.pictured() || self.hit_at(on) != Some(Hit::Body) {
            return;
        }
        if self.zoom > 1.0 {
            self.zoom = 1.0;
            self.pan = (0.0, 0.0);
        } else if let Some((fitted, own)) = self.fitted() {
            let k = (own.0 as f32 / self.scale / fitted.2).clamp(2.0, ZOOM_MOST);
            self.zoom_about(on, k);
        }
        self.draw();
    }

    /// The picture shown as fitted in the body (DIPs on the sheet), and its own size (px).
    fn fitted(&self) -> Option<((f32, f32, f32, f32), (u32, u32))> {
        let (w, h) = self.shape(Instant::now()).size;
        let (top, pad) = (self.body_top(), self.inset());
        let (picture, own) = match self.body()? {
            Body::Picture(picture, own) => (picture, *own),
            Body::Pictured(picture) => (picture, picture.size),
            _ => return None,
        };
        Some((picture.fitted_in(pad, top, (w - 2.0 * pad).max(0.0), (h - top - pad).max(0.0)), own))
    }

    /// The picture zoomed to `zoom`, the point `on` (DIPs on the sheet) staying where it is.
    fn zoom_about(&mut self, on: (f32, f32), zoom: f32) {
        let Some((fitted, _)) = self.fitted() else { return };
        let zoom = zoom.clamp(1.0, ZOOM_MOST);
        let k = zoom / self.zoom;
        let centre = (fitted.0 + fitted.2 / 2.0 + self.pan.0, fitted.1 + fitted.3 / 2.0 + self.pan.1);
        self.pan = (self.pan.0 - (on.0 - centre.0) * (k - 1.0), self.pan.1 - (on.1 - centre.1) * (k - 1.0));
        self.zoom = zoom;
        self.keep_in_view();
    }

    /// The picture zoomed in kept over the body: moved no further than its edges reach past the body's.
    fn keep_in_view(&mut self) {
        let Some((fitted, _)) = self.fitted() else { return };
        if self.zoom <= 1.0 {
            self.pan = (0.0, 0.0);
            return;
        }
        let (w, h) = self.shape(Instant::now()).size;
        let (top, pad) = (self.body_top(), self.inset());
        let body = ((w - 2.0 * pad).max(0.0), (h - top - pad).max(0.0));
        let reach = |zoomed: f32, room: f32, middle_off: f32| ((zoomed - room) / 2.0).max(0.0) + middle_off.abs();
        // (How far its middle may go from the body's: as far as its edges reach past the body's, where it is larger.)
        let off = (fitted.0 + fitted.2 / 2.0 - (pad + body.0 / 2.0), fitted.1 + fitted.3 / 2.0 - (top + body.1 / 2.0));
        let most = (reach(fitted.2 * self.zoom, body.0, 0.0), reach(fitted.3 * self.zoom, body.1, 0.0));
        self.pan = ((self.pan.0 + off.0).clamp(-most.0, most.0) - off.0, (self.pan.1 + off.1).clamp(-most.1, most.1) - off.1);
    }

    fn wheel(&mut self, turns: f32, at: (i32, i32)) {
        let on = self.on_sheet(at);
        if !matches!(self.motion, Motion::Resting) {
            return;
        }
        if self.pictured() && self.hit_at(on) == Some(Hit::Body) {
            self.zoom_about(on, self.zoom * ZOOM_TURN.powf(turns));
        } else if matches!(self.body(), Some(Body::Words { .. })) {
            self.scroll = (self.scroll - turns * 3.0 * 22.0).clamp(0.0, self.scroll_most);
        } else if matches!(self.body(), Some(Body::Group(_))) {
            self.grid_scroll = (self.grid_scroll - turns * 3.0 * 22.0).clamp(0.0, self.grid_scroll_most);
        }
        self.draw();
    }

    // ----- What plays -----

    /// How loud what plays is played: as set (heard as louder the more it is set, as a player's slider is), or not at
    /// all, muted.
    fn loudness(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume * self.volume }
    }

    /// What plays the thing shown made, where it is a video or a sound, the sheet rests, and none is (it begins
    /// playing as it is made). One that cannot be made: the thing shown as its picture, and said so.
    fn tend_player(&mut self) {
        if self.player.is_some() || self.unplayable || !matches!(self.motion, Motion::Resting) || !matches!(self.body(), Some(Body::Played { .. })) {
            return;
        }
        let Some(path) = self.shown_path().map(Path::to_path_buf) else { return };
        let made = self.gfx.d3d().and_then(|d3d| play::Player::new(&path, &d3d, self.loudness(), self.hwnd, PLAYED));
        match made {
            Ok(player) => {
                player.play();
                self.player = Some(player);
                self.stirred = Instant::now();
                PLAYS.store(true, Ordering::Release);
                crate::app().controller.wake();
                self.keep_drawing();
            }
            Err(error) => {
                crate::journal::note(format!("look: {} could not be played ({error})", path.display()));
                self.unplayable = true;
            }
        }
    }

    /// What played the thing shown let go of (another to be shown, or it gone).
    fn stop_playing(&mut self) {
        let played = self.player.take().is_some();
        self.unplayable = false;
        self.holding = None;
        (self.strip_at, self.track_at, self.loud_at, self.slider_at) = (None, None, None, None);
        if played {
            PLAYS.store(false, Ordering::Release);
            crate::app().controller.wake();
        }
    }

    /// What plays told something of itself (`event`, as its engine says it): drawn as it is now. One that failed: the
    /// thing shown as its picture, and said so.
    fn played(&mut self, event: usize) {
        if self.player.is_none() {
            return;
        }
        if play::failed_event(event) {
            if let Some(path) = self.shown_path() {
                crate::journal::note(format!("look: {} could not be played", path.display()));
            }
            self.stop_playing();
            self.unplayable = true;
        } else if self.player.as_ref().is_some_and(play::Player::playing) {
            self.keep_drawing();
        }
        self.draw();
    }

    /// Played, or paused; at its end, played again from its beginning. The pointer stirred: a video's controls show.
    fn toggle(&mut self) {
        let Some(player) = self.player.as_ref().filter(|_| !matches!(self.motion, Motion::Closing { .. })) else { return };
        if player.playing() {
            player.pause();
        } else {
            if player.ended() {
                player.seek(0.0);
            }
            player.play();
        }
        self.stirred = Instant::now();
        self.keep_drawing();
        self.draw();
    }

    /// Its frame timer running: what plays drawn as it goes (see `frame_due`).
    fn keep_drawing(&self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), FRAME, FRAME_MS, None);
        }
    }

    /// Whether it is to be drawn on as time goes, at rest: what plays plays, or is dragged along, or its controls fade,
    /// or its slider opens or closes.
    fn plays_on(&self, now: Instant) -> bool {
        let clear = self.controls_clear(now);
        self.player.as_ref().is_some_and(play::Player::playing) || self.holding.is_some() || (clear > 0.0 && clear < 1.0) || self.slider_moving(now)
    }

    /// A frame's time: it drawn where it moves; at rest, where what plays has a new frame, or has gone on far enough to
    /// show (its clock, its track by a pixel), or its controls or slider change. Its timer stopped once nothing does.
    fn frame_due(&mut self) {
        if !self.up() {
            return;
        }
        if !matches!(self.motion, Motion::Resting) {
            return self.draw();
        }
        let now = Instant::now();
        let fresh = match (self.player.as_mut(), self.gfx.d3d()) {
            (Some(player), Ok(d3d)) => player.advance(&d3d),
            _ => false,
        };
        let (at, (drawn_at, drawn_clear, drawn_open)) = (self.shown_at(), self.drawn_playing);
        let length = self.player.as_ref().and_then(play::Player::length).unwrap_or(0.0);
        let track = f64::from(self.track_at.map_or(0.0, |track| track.w * self.scale));
        let gone_on = at.floor() != drawn_at.floor() || (length > 0.0 && (at - drawn_at).abs() / length * track >= 0.5);
        if fresh || gone_on || self.controls_clear(now) != drawn_clear || self.slider_open_now(now) != drawn_open {
            self.draw();
        }
        if self.hides_pointer() {
            unsafe { SetCursor(None) };
        }
        if !self.plays_on(now) {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), FRAME);
            }
        }
    }

    /// Where what plays is (seconds): where its track is dragged to, while it is.
    fn shown_at(&self) -> f64 {
        match self.holding {
            Some(Holding::Track(seconds)) => seconds,
            _ => self.player.as_ref().map_or(0.0, play::Player::at),
        }
    }

    /// How clear a video's controls are at `now`: whole while it is paused, held, or the pointer is on them or moved
    /// over it a moment ago (`CONTROLS_FOR`); fading after.
    fn controls_clear(&self, now: Instant) -> f32 {
        let Some(player) = &self.player else { return 1.0 };
        if !player.playing() || self.holding.is_some() || self.strip_at.is_some_and(|strip| self.pointer_on(strip)) {
            return 1.0;
        }
        match now.checked_duration_since(self.stirred + CONTROLS_FOR) {
            None => 1.0,
            Some(past) => (1.0 - past.as_secs_f32() / CONTROLS_FADE.as_secs_f32()).max(0.0),
        }
    }

    /// How open the volume's slider is at `now` (0 shut, 1 open), and whether it is still opening or shutting.
    fn slider_open_now(&self, now: Instant) -> f32 {
        let (open, from, start) = self.slider_open;
        let p = MOVE_EASE.at((now.duration_since(start).as_secs_f32() / SLIDER_FOR.as_secs_f32()).clamp(0.0, 1.0));
        from + (f32::from(u8::from(open)) - from) * p
    }

    fn slider_moving(&self, now: Instant) -> bool {
        now.duration_since(self.slider_open.2) < SLIDER_FOR
    }

    /// Whether the pointer is hidden: over a video playing with its controls gone, the pointer on its window.
    fn hides_pointer(&self) -> bool {
        let video = self.player.as_ref().is_some_and(|player| player.playing() && player.video_size().is_some());
        if !video || self.controls_clear(Instant::now()) > 0.0 || !matches!(self.motion, Motion::Resting) {
            return false;
        }
        let body = self.body_at(self.shape(Instant::now()).size);
        let mut at = POINT::default();
        let _ = unsafe { GetCursorPos(&mut at) };
        let over = unsafe { GetAncestor(WindowFromPoint(at), GA_ROOT) } == self.hwnd;
        over && self.pointer_on(body)
    }

    /// The pointer moved to `on` (DIPs on the sheet): over a video, its controls shown; where its track or slider is
    /// held, taken along; over the volume, its slider opened (and shut as it leaves).
    fn moved_over_player(&mut self, on: (f32, f32)) {
        if self.player.is_none() {
            return;
        }
        let now = Instant::now();
        let body = self.body_at(self.shape(now).size);
        if self.pointer_on(body) {
            self.stirred = now;
        }
        match self.holding {
            Some(Holding::Track(_)) => self.track_to(on.0),
            Some(Holding::Volume) => self.volume_to(on.0),
            None => {}
        }
        let open = self.holding == Some(Holding::Volume) || self.loud_at.is_some_and(|loud| self.pointer_on(loud));
        if open != self.slider_open.0 {
            self.slider_open = (open, self.slider_open_now(now), now);
        }
        self.keep_drawing();
    }

    /// What plays taken to where `x` (DIPs on the sheet) is along its track, and held there.
    fn track_to(&mut self, x: f32) {
        let (Some(track), Some(player)) = (self.track_at, &self.player) else { return };
        let Some(length) = player.length() else { return };
        let seconds = f64::from(((x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0)) * length;
        player.seek(seconds);
        self.holding = Some(Holding::Track(seconds));
        self.keep_drawing();
    }

    /// As loud as `x` (DIPs on the sheet) is along the volume's slider, heard so at once.
    fn volume_to(&mut self, x: f32) {
        let Some(slider) = self.slider_at else { return };
        self.volume = ((x - slider.x - KNOB / 2.0) / (SLIDER - KNOB)).clamp(0.0, 1.0);
        self.muted = false;
        if let Some(player) = &self.player {
            player.set_volume(self.loudness());
        }
        self.keep_drawing();
    }

    /// How loud it plays kept in the settings, for every look after.
    fn keep_volume(&self) {
        let (volume, muted) = (self.volume, self.muted);
        crate::app().change(move |settings| {
            settings.look_volume = volume;
            settings.look_muted = muted;
        });
    }

    // ----- The keys -----

    fn key(&mut self, key: Key) {
        if !self.up() {
            return;
        }
        let group = self.members().is_some();
        match key {
            Key::Close => self.close(),
            Key::Toggle => self.toggle(),
            // (A file of the group by itself: the others of it by the arrows; among them: the one picked.)
            Key::Left | Key::Right if self.inside.is_some() => self.step(key == Key::Right),
            Key::Up | Key::Down if self.inside.is_some() => {}
            Key::Left | Key::Right | Key::Up | Key::Down if group => self.pick(key),
            Key::Back => self.leave(),
            Key::Open if self.inside.is_some() => {
                let Some(path) = self.shown_path().map(Path::to_path_buf) else { return };
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                crate::station::run::ask(Ask::Files(vec![path], crate::station::run::FilesAct::Open));
                self.told = Some((Hit::Open, Instant::now()));
                self.close();
            }
            Key::Open if group && self.picked.is_some() => self.enter(self.picked.unwrap_or_default()),
            Key::Copy if self.inside.is_some() => {
                let Some(path) = self.shown_path().map(Path::to_path_buf) else { return };
                crate::station::run::ask(Ask::Files(vec![path], crate::station::run::FilesAct::Copy(None)));
                self.told = Some((Hit::Copy, Instant::now()));
                self.draw();
            }
            Key::Left => self.go_to(self.at.wrapping_sub(1)),
            Key::Right => self.go_to(self.at + 1),
            Key::Up | Key::Down => {
                let at = self.beside_of(key == Key::Down);
                if let Some(at) = at {
                    self.go_to(at);
                }
            }
            Key::Open => {
                let item = self.items[self.at].clone();
                let Some(open) = item.open else { return };
                // (What opens it comes to the front: it goes meanwhile.)
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                crate::station::run::ask(open);
                self.told = Some((Hit::Open, Instant::now()));
                self.close();
            }
            Key::Copy => {
                let Some(copy) = self.items[self.at].copy.clone() else { return };
                crate::station::run::ask(copy);
                self.told = Some((Hit::Copy, Instant::now()));
                self.draw();
            }
        }
    }

    /// The thing above or `below` the one shown, as their tiles are: the nearest whose tile is that way; where none is
    /// shown, the one before or after it.
    fn beside_of(&self, below: bool) -> Option<usize> {
        let middle = |rect: RECT| ((rect.left + rect.right) as f32 / 2.0, (rect.top + rect.bottom) as f32 / 2.0);
        let Some(here) = self.items[self.at].from.map(middle) else {
            return if below { Some(self.at + 1) } else { self.at.checked_sub(1) };
        };
        self.items
            .iter()
            .enumerate()
            .filter_map(|(at, item)| Some((at, middle(item.from?))))
            .filter(|(_, (_, y))| if below { *y > here.1 + 1.0 } else { *y < here.1 - 1.0 })
            .min_by(|(_, a), (_, b)| {
                let far = |(x, y): (f32, f32)| (y - here.1).abs() + 2.0 * (x - here.0).abs();
                far(*a).total_cmp(&far(*b))
            })
            .map(|(at, _)| at)
    }

    // ----- Watching -----

    /// A look at what may have changed: a button pressed anywhere but on it and what called it up (it goes), what
    /// called it up gone (so it does), a picture made for what it shows, a word on a button gone.
    fn watch(&mut self) {
        if !self.up() || matches!(self.motion, Motion::Closing { .. }) {
            return;
        }
        let owner = HWND(self.owner as *mut _);
        if !unsafe { IsWindowVisible(owner) }.as_bool() {
            return self.close();
        }
        let held = buttons_held();
        if held && !self.held {
            let mut at = POINT::default();
            let _ = unsafe { GetCursorPos(&mut at) };
            let under = unsafe { GetAncestor(WindowFromPoint(at), GA_ROOT) };
            // (On it, its previewer, or what called it up: theirs to say what is done.)
            let on_pane = self.pane.as_ref().is_some_and(|pane| pane.host() == under);
            if under != self.hwnd && under != owner && !on_pane {
                self.held = held;
                return self.close();
            }
        }
        self.held = held;
        let pictures = crate::station::thumbs::changes();
        let told = self.told.is_some_and(|(_, at)| at.elapsed() >= TOLD_FOR);
        if pictures != self.pictures || told {
            self.pictures = pictures;
            if told {
                self.told = None;
            }
            self.draw();
        }
    }
}

/// Whether a mouse button is held (any of them, as the hardware's: a left-handed mouse's too).
fn buttons_held() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|button| unsafe { GetAsyncKeyState(button.0 as i32) } as u16 & 0x8000 != 0)
}

/// A box of the shape `size` (px across, down) as large as fits in `room`, in its middle.
fn fit_in(size: (u32, u32), room: Rect) -> Rect {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let k = (room.w / w).min(room.h / h);
    Rect { x: room.x + (room.w - w * k) / 2.0, y: room.y + (room.h - h * k) / 2.0, w: w * k, h: h * k }
}

/// `seconds` as a clock shows them: minutes and seconds (hours too, an hour or more).
fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0).floor() as u64;
    match whole / 3600 {
        0 => format!("{}:{:02}", whole / 60, whole % 60),
        hours => format!("{hours}:{:02}:{:02}", whole / 60 % 60, whole % 60),
    }
}

/// `rect` made `k` as large about its middle.
fn shrunk(rect: Rect, k: f32) -> Rect {
    Rect { x: rect.x + rect.w * (1.0 - k) / 2.0, y: rect.y + rect.h * (1.0 - k) / 2.0, w: rect.w * k, h: rect.h * k }
}

/// The box `p` of the way from `a` to `b`.
fn between(a: Rect, b: Rect, p: f32) -> Rect {
    let mix = |a: f32, b: f32| a + (b - a) * p;
    Rect { x: mix(a.x, b.x), y: mix(a.y, b.y), w: mix(a.w, b.w), h: mix(a.h, b.h) }
}

/// How the pictures of files together are laid out in a body `inner` wide (DIPs), each `shapes` wide for its height:
/// in rows, each row as many as fill it at `GROUP_ROW` tall, then made as tall as fills it exactly (no taller), the
/// last as tall as `GROUP_ROW`. Each one's place in the body, its name under it; and how tall they are together.
fn group_cells(shapes: &[f32], inner: f32) -> (Vec<Rect>, f32) {
    let mut cells = Vec::with_capacity(shapes.len());
    let (mut first, mut y) = (0, 0.0);
    while first < shapes.len() {
        let (mut last, mut wide) = (first, shapes[first]);
        while last + 1 < shapes.len() && wide * GROUP_ROW + (last - first) as f32 * GROUP_GAP < inner {
            last += 1;
            wide += shapes[last];
        }
        let gaps = (last - first) as f32 * GROUP_GAP;
        let row = if wide * GROUP_ROW + gaps >= inner { (inner - gaps).max(0.0) / wide } else { GROUP_ROW };
        let mut x = 0.0;
        for shape in &shapes[first..=last] {
            cells.push(Rect { x, y, w: shape * row, h: row });
            x += shape * row + GROUP_GAP;
        }
        y += row + GROUP_NAME + GROUP_GAP;
        first = last + 1;
    }
    (cells, (y - GROUP_GAP).max(0.0))
}

/// How wide the picture of each file of a group is for its height, as it was measured (none: square); none narrower
/// or wider than a row may well show.
fn shapes(members: &[Member]) -> Vec<f32> {
    members.iter().map(|member| member.seen.map_or(1.0, |(w, h)| (w.max(1) as f32 / h.max(1) as f32).clamp(0.3, 5.0))).collect()
}

/// `body` (DIPs on a sheet at `sheet`, DIPs on a window covering the screen `monitor`, `scale` px a DIP) on the screen
/// (physical px).
fn on_screen(body: Rect, sheet: Rect, scale: f32, monitor: RECT) -> RECT {
    let (x, y) = (monitor.left as f32 + (sheet.x + body.x) * scale, monitor.top as f32 + (sheet.y + body.y) * scale);
    RECT { left: x.round() as i32, top: y.round() as i32, right: (x + body.w * scale).round() as i32, bottom: (y + body.h * scale).round() as i32 }
}

/// `color` laid over the solid `ground`: what shows, solid.
fn over(color: Color, ground: Color) -> Color {
    let mix = |a: f32, b: f32| a * color.a + b * (1.0 - color.a);
    Color { r: mix(color.r, ground.r), g: mix(color.g, ground.g), b: mix(color.b, ground.b), a: 1.0 }
}

/// How light `color` is (0 black, 1 white).
fn luminance(color: Color) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

/// What `draw` draws, seen through as much as `opacity` says (1: as it is).
fn faded(frame: &Frame, opacity: f32, draw: impl FnOnce()) {
    if opacity >= 1.0 {
        draw();
        return;
    }
    let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
    let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity: opacity.max(0.0), ..Default::default() };
    unsafe { frame.dc.PushLayer(&layer, None) };
    draw();
    unsafe { frame.dc.PopLayer() };
}

fn point(lparam: LPARAM) -> (i32, i32) {
    ((lparam.0 & 0xFFFF) as u16 as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        ASKED => {
            with(Looker::asked);
            LRESULT(0)
        }
        LOADED => {
            with(Looker::loaded);
            LRESULT(0)
        }
        // (Made with its state let go of: the window's thread keeps handling what it is sent while a previewer is made.)
        PLAYED => {
            with(|looker| looker.played(wparam.0));
            LRESULT(0)
        }
        PANE => {
            if let Some(wanted) = with(Looker::pane_wanted).flatten() {
                let mut made = Some(pane::load(&wanted));
                if with(|looker| looker.pane_made(wanted.path, made.take().unwrap())).is_none() {
                    if let Some(Ok(handler)) = made {
                        let _ = unsafe { handler.Unload() };
                    }
                }
            }
            LRESULT(0)
        }
        // A click on it leaves the keyboard where it was.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            with(Looker::draw);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let (on_button, hidden) = with(|looker| {
                let on_button = looker.pointer.and_then(|on| looker.hit_at(on)).is_some_and(|hit| matches!(hit, Hit::Open | Hit::Copy | Hit::Cell(_) | Hit::Back | Hit::Track | Hit::Volume | Hit::Mute));
                (on_button, looker.hides_pointer())
            })
            .unwrap_or((false, false));
            // (Over a video playing with its controls gone: none, as a player shows none.)
            unsafe { SetCursor(if hidden { None } else { LoadCursorW(None, if on_button { IDC_HAND } else { IDC_ARROW }).ok() }) };
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            with(|looker| looker.moved(point(lparam)));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            with(|looker| looker.press(point(lparam)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with(|looker| looker.release(point(lparam)));
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            with(|looker| looker.double(point(lparam)));
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with(|looker| {
                looker.dragging = None;
                looker.pressed = None;
                looker.holding = None;
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let turns = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            // (The wheel's point is on the screen.)
            let (x, y) = point(lparam);
            with(|looker| looker.wheel(turns, (x - looker.monitor.left, y - looker.monitor.top)));
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                FRAME => with(Looker::frame_due),
                _ => with(Looker::watch),
            };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The look in each skin, light and dark (the rows), at words, a picture and what has no picture (the columns), as
    /// it rests (`cargo test --profile fast --features studio --lib -- --ignored look_looks` writes
    /// `target/looks/look.png`).
    #[cfg(feature = "studio")]
    #[test]
    #[ignore = "writes a picture to look at"]
    fn look_looks() {
        let _device = crate::ui::gfx::held().unwrap();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let gfx = gfx::current().unwrap();
        let (reads, _read) = std::sync::mpsc::channel();
        let file = |name: &str| PathBuf::from(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).components().collect::<PathBuf>();
        let words = "40分钟精讲：为什么说宋仁宗时代，是中国文人的黄金时代？ #历史#宋仁宗 #狸猫换太子 #人文创作焕新计划
@历史摆渡人
· 9月19日
【纯享合集】P9 织造局灯笼高高挂，杨金水暗回查真相 #大明王朝1566 #电视剧解说 #历史";
        let picture = file("icons/128x128@2x.png");
        let (shown, own) = decode(&picture, 7).expect("decoded");
        let item = |key: u64, title: &str, about: &str, shows: Shows, pastes: bool| Item { key, title: title.into(), about: about.into(), shows, from: None, open: Some(Ask::Open(key)), copy: Some(Ask::Copy(key)), pastes };
        let things = [
            (item(1, "文字", "129 字 · 40 分钟前", Shows::Words(words.into()), true), Read { body: Body::Words { text: words.into(), fixed: false, cut: false }, about: None }, WORDS),
            (item(2, "128x128@2x.png", "256 × 256 · PNG 图像 · 31 KB", Shows::File(picture.clone()), false), Read { body: Body::Picture(shown, own), about: None }, (420.0, 400.0)),
            (item(3, "Cargo.toml", "TOML 文件 · 6 KB · 20 分钟前", Shows::File(file("Cargo.toml")), false), Read { body: Body::Other(Some(file("Cargo.toml"))), about: Some("TOML 文件 · 6 KB".into()) }, OTHER),
        ];
        let looks = [(Skin::Glass, false), (Skin::Glass, true), (Skin::Fluent, false), (Skin::Fluent, true), (Skin::Paper, false), (Skin::Paper, true)];
        let cell = (WORDS.0 + 2.0 * SHADOW + 20.0, WORDS.1.max(400.0) + 2.0 * SHADOW + 20.0);
        let px = 1.5f32;
        let (w, h) = ((3.0 * cell.0 * px) as u32, (looks.len() as f32 * cell.1 * px) as u32);
        let pixels = gfx
            .draw_offscreen((w, h), px, |frame| {
                frame.fill(Color::hex(0x7C93B4, 1.0), 0.0, 0.0, 3.0 * cell.0, looks.len() as f32 * cell.1);
                for (row, &(skin, dark)) in looks.iter().enumerate() {
                    if dark {
                        frame.fill(Color::hex(0x1B2433, 1.0), 0.0, row as f32 * cell.1, 3.0 * cell.0, cell.1);
                    }
                    for (column, (item, read, size)) in things.iter().enumerate() {
                        let mut looker = Looker::new(HWND::default(), gfx.clone(), reads.clone());
                        (looker.skin, looker.dark, looker.lang, looker.scale) = (skin, dark, Lang::Zh, px);
                        looker.items = vec![item.clone(), item.clone(), item.clone(), item.clone()];
                        looker.read = vec![Some(read.clone()), None, None, None];
                        let at = Rect { x: column as f32 * cell.0 + SHADOW + 10.0, y: row as f32 * cell.1 + SHADOW + 10.0, w: size.0, h: size.1 };
                        looker.paint(frame, skin != Skin::Paper, Shape { at, size: *size, opacity: 1.0, body: 1.0 });
                        frame.place(Matrix3x2::identity());
                    }
                }
            })
            .expect("drawn");
        let out = PathBuf::from(format!("{}/target/looks", env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&out).unwrap();
        crate::studio::save_png(&out.join("look.png"), w, h, &pixels).unwrap();
    }

    /// Its things had anew as they show now (a tile clicked of another list, or scrolled to): what was read of those
    /// still among them kept at their places now, none for those new.
    #[test]
    fn what_was_read_is_carried_to_the_things_had_anew() {
        let item = |key: u64| Item { key, title: String::new(), about: String::new(), shows: Shows::Words(String::new()), from: None, open: None, copy: None, pastes: false };
        let read = |text: &str| Some(Read { body: Body::Words { text: text.into(), fixed: false, cut: false }, about: None });
        let words = |read: &Option<Read>| match read {
            Some(Read { body: Body::Words { text, .. }, .. }) => Some(text.clone()),
            _ => None,
        };
        let carried = carried(&[item(1), item(2), item(3)], vec![read("one"), None, read("three")], &[item(3), item(4), item(1), item(2)]);
        assert_eq!(carried.iter().map(words).collect::<Vec<_>>(), [Some("three".into()), None, Some("one".into()), None]);
    }

    /// A file's words are read as they are marked (UTF-16 either way, UTF-8), else as UTF-8, else in the system's own
    /// code page; a file with a nought among its bytes holds no words.
    #[test]
    fn words_are_read_in_what_they_are_written_in() {
        assert_eq!(words_in("fn main() {}\n".as_bytes(), false).as_deref(), Some("fn main() {}\n"));
        assert_eq!(words_in(&[0xEF, 0xBB, 0xBF, b'a'], false).as_deref(), Some("a"));
        assert_eq!(words_in(&[0xFF, 0xFE, b'h', 0, b'i', 0], false).as_deref(), Some("hi"));
        assert_eq!(words_in(&[0xFE, 0xFF, 0, b'h', 0, b'i'], false).as_deref(), Some("hi"));
        assert_eq!(words_in(&[0x89, b'P', b'N', b'G', 0, 0], false), None);
        // (Cut off in the middle of a character: the words before it.)
        let cut = "中文".as_bytes();
        assert_eq!(words_in(&cut[..4], true).as_deref(), Some("中"));
    }

    /// Words the station keeps in a file (named by its number, no kind in its name) are read as the plain words they
    /// were copied as, as the same words on the station's list are, not as code in a fixed face.
    #[test]
    fn words_kept_in_a_file_are_plain_words() {
        let folder = std::env::temp_dir().join(format!("glance-look-kept-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let kept = folder.join("18e12bb87ce65488-0");
        std::fs::write(&kept, "40分钟精讲 #历史
@历史摆渡人").unwrap();
        let read = read(&Shows::Kept(kept.clone()), Lang::Zh, 1.0, 1);
        assert!(matches!(&read.body, Body::Words { text, fixed: false, cut: false } if text == "40分钟精讲 #历史
@历史摆渡人"));
        // (Gone from the folder: said so, not shown as some file.)
        std::fs::remove_file(&kept).unwrap();
        let gone = super::read(&Shows::Kept(kept), Lang::Zh, 1.0, 1);
        assert!(matches!(gone.body, Body::Other(None)));
        let _ = std::fs::remove_dir(&folder);
    }

    /// What a file's name says it is.
    #[test]
    fn kinds_are_told_by_names() {
        assert_eq!(kind_of(Path::new(r"C:\a\Shot.PNG")), Kind::Picture);
        assert_eq!(kind_of(Path::new(r"C:\a\Report.pdf")), Kind::Page);
        assert_eq!(kind_of(Path::new(r"C:\a\clip.mp4")), Kind::Video);
        assert_eq!(kind_of(Path::new(r"C:\a\song.FLAC")), Kind::Sound);
        assert_eq!(kind_of(Path::new(r"C:\a\volume.rs")), Kind::Unknown);
        assert_eq!(kind_of(Path::new(r"C:\a\LICENSE")), Kind::Unknown);
    }

    /// A picture is measured from its header at its own size, and a thing that is neither a picture nor a video is not.
    #[test]
    fn a_picture_is_measured_before_it_is_read() {
        // (As on the reading thread.)
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let file = |name: &str| PathBuf::from(format!("{}/{name}", env!("CARGO_MANIFEST_DIR")));
        assert_eq!(measure(&Shows::File(file("icons/128x128@2x.png"))), Some((256, 256)));
        assert_eq!(measure(&Shows::File(file("Cargo.toml"))), None);
        assert_eq!(measure(&Shows::Words("words".into())), None);
    }

    /// A picture's or a video's sheet is as large from the moment it is measured as once what it shows is read: a
    /// video's picture made smaller than its frames (as the shell makes it, no larger than asked), or a small picture,
    /// do not make it another size then.
    /// A page is shown as a page's shape: as tall as it may be, or, on a screen taller than it is wide (where so tall it
    /// would be wider than it may be), as wide as it may be and no taller than a page that wide.
    #[test]
    fn a_page_keeps_a_page_shape() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = gfx::current().unwrap();
        let (reads, _read) = std::sync::mpsc::channel();
        let item = Item { key: 1, title: "a.pdf".into(), about: String::new(), shows: Shows::File(PathBuf::from(r"C:.pdf")), from: None, open: None, copy: None, pastes: false };
        let mut looker = Looker::new(HWND::default(), gfx, reads);
        (looker.items, looker.read, looker.measured) = (vec![item], vec![None], vec![None]);
        let (top, pad) = (looker.body_top(), looker.inset());
        for most in [(1400.0, 900.0), (750.0, 1600.0)] {
            let (w, h) = looker.wanted(most);
            assert!(w <= most.0 && h <= most.1, "{most:?}: {w} × {h}");
            let shape = (h - top - pad) / (w - 2.0 * pad - PAGE_ROOM);
            assert!((shape - PAGE).abs() < 0.01, "{most:?}: {w} × {h} is a page's shape ({shape})");
        }
    }

    #[test]
    fn a_sheet_keeps_its_size_as_what_it_shows_is_read() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = gfx::current().unwrap();
        let (reads, _read) = std::sync::mpsc::channel();
        let item = |name: &str| Item { key: 1, title: name.into(), about: String::new(), shows: Shows::File(PathBuf::from(format!(r"C:\a\{name}"))), from: None, open: None, copy: None, pastes: false };
        let picture = |size: (u32, u32)| Picture { key: 1, size, pixels: std::sync::Arc::new(Vec::new()) };
        let most = (1400.0, 900.0);
        for (name, size, read) in [
            ("clip.mp4", (1920, 1080), Body::Live { picture: Some(picture((1600, 900))), previewer: windows::core::GUID::zeroed() }),
            ("clip.mkv", (1280, 720), Body::Pictured(picture((640, 360)))),
            ("tall.mp4", (1080, 1920), Body::Live { picture: None, previewer: windows::core::GUID::zeroed() }),
            ("small.png", (300, 200), Body::Picture(picture((300, 200)), (300, 200))),
        ] {
            let mut looker = Looker::new(HWND::default(), gfx.clone(), reads.clone());
            looker.scale = 1.5;
            looker.items = vec![item(name)];
            (looker.read, looker.measured) = (vec![None], vec![Some(size)]);
            let measured = looker.wanted(most);
            looker.read = vec![Some(Read { body: read, about: None })];
            assert_eq!(looker.wanted(most), measured, "{name}");
        }
    }

    /// The camera's orientations each turn the picture upright, the four that turn it a quarter swapping its sides.
    #[test]
    fn pictures_are_turned_as_they_were_held() {
        assert!(turn_of(1).is_none());
        assert!(turn_of(0).is_none());
        for orientation in 2..=8 {
            assert!(turn_of(orientation).is_some(), "{orientation}");
        }
    }

    /// A sheet's body on the screen: at the sheet's place on its window, on the screen the window covers, in px.
    #[test]
    fn a_body_is_found_on_the_screen() {
        let monitor = RECT { left: -1920, top: 100, right: 0, bottom: 1180 };
        let body = Rect { x: 20.0, y: 80.0, w: 400.0, h: 300.0 };
        let sheet = Rect { x: 100.0, y: 50.0, w: 440.0, h: 400.0 };
        assert_eq!(on_screen(body, sheet, 1.5, monitor), RECT { left: -1920 + 180, top: 100 + 195, right: -1920 + 780, bottom: 100 + 645 });
    }

    /// Between two boxes, and a box made smaller about its middle.
    /// Files together are laid out in rows that fill the body's width, each picture of its own shape, none over another,
    /// the last row as tall as a row is wanted.
    #[test]
    fn a_group_is_laid_out_in_rows_that_fill_it() {
        let shapes = [1.5, 0.75, 1.0, 2.0, 1.33, 0.6, 1.78, 1.0, 0.5];
        let inner = 600.0;
        let (cells, tall) = group_cells(&shapes, inner);
        assert_eq!(cells.len(), shapes.len());
        for (cell, shape) in cells.iter().zip(shapes) {
            assert!((cell.w / cell.h - shape).abs() < 1e-4, "{cell:?} keeps its shape {shape}");
            assert!(cell.h <= GROUP_ROW + 1e-4);
        }
        let mut rows: Vec<f32> = cells.iter().map(|cell| cell.y).collect();
        rows.dedup();
        assert!(rows.len() > 1);
        for (number, row) in rows.iter().enumerate() {
            let of_row: Vec<&Rect> = cells.iter().filter(|cell| cell.y == *row).collect();
            let right = of_row.iter().map(|cell| cell.x + cell.w).fold(0.0, f32::max);
            if number + 1 < rows.len() {
                assert!((right - inner).abs() < 1e-3, "row {number} fills the width: {right}");
            } else {
                assert!((of_row[0].h - GROUP_ROW).abs() < 1e-4 && right <= inner);
            }
        }
        for (a, one) in cells.iter().enumerate() {
            for other in &cells[a + 1..] {
                let apart = one.x + one.w <= other.x || other.x + other.w <= one.x || one.y + one.h + GROUP_NAME <= other.y || other.y + other.h + GROUP_NAME <= one.y;
                assert!(apart, "{one:?} and {other:?} apart");
            }
        }
        let last = cells.last().unwrap();
        assert!((tall - (last.y + last.h + GROUP_NAME)).abs() < 1e-4);
        // (One wider than the body: a row of its own, as wide.)
        let (cells, _) = group_cells(&[5.0, 1.0], 600.0);
        assert!((cells[0].w - 600.0).abs() < 1e-3 && cells[1].y > cells[0].y);
    }

    /// Times are told as a clock shows them.
    #[test]
    fn times_are_told_as_a_clock_shows_them() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(62.9), "1:02");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-3.0), "0:00");
    }

    #[test]
    fn boxes_move_between() {
        let (a, b) = (Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Rect { x: 100.0, y: 50.0, w: 210.0, h: 110.0 });
        assert_eq!(between(a, b, 0.0), a);
        assert_eq!(between(a, b, 1.0), b);
        assert_eq!(between(a, b, 0.5), Rect { x: 50.0, y: 25.0, w: 110.0, h: 60.0 });
        assert_eq!(shrunk(Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }, 0.5), Rect { x: 25.0, y: 12.5, w: 50.0, h: 25.0 });
    }
}
