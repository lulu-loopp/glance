//! A look at a thing, larger: what is under the pointer on a station (the panel's, a widget's: put on it, copied
//! lately, come lately), or picked in search, called up by the preview's shortcut. It grows out of its tile to a
//! sheet beside what it was called up from, drawn in the skin the panel is (its glass frosted over the screen,
//! Windows 11's acrylic, chart paper), light or dark as the panel is: a picture whole (zoomed by the wheel about the
//! pointer, moved by a drag, between fitted and its own pixels by a double click), words all of them, a document or
//! a video as Explorer's preview pane shows it (live, by its kind's previewer: see `pane`), else as Explorer pictures
//! it, else its icon and what it is. It goes back into its tile as the shortcut or Esc
//! is pressed again, or as anything but it and what it was called up from is clicked.
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

/// A key of its own, handed to it by what called it up: Esc (and its shortcut), the arrows, Enter, Ctrl+C.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Close,
    Left,
    Right,
    Up,
    Down,
    Open,
    Copy,
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
/// The window it was called up from while it is up (0: it is not), and the key of what it shows.
static OWNER: AtomicIsize = AtomicIsize::new(0);
static SHOWN: AtomicU64 = AtomicU64::new(0);
/// Whether its shortcut was wanted but another program had it as it was last taken (see `key_was_taken`).
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Its messages: something asked, something read, a previewer wanted for what it shows (see `pane`).
const ASKED: u32 = WM_APP + 1;
const LOADED: u32 = WM_APP + 2;
const PANE: u32 = WM_APP + 3;
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
const MOVE_EASE: Easing = Easing(0.2, 0.8, 0.2, 1.0);
const FADE_IN: Duration = Duration::from_millis(150);
const CLOSE_FOR: Duration = Duration::from_millis(200);
const CLOSE_EASE: Easing = Easing(0.4, 0.0, 0.7, 1.0);
const CLOSE_STILL: (f32, f32) = (0.7, 0.85);

/// Its parts (DIPs): its buttons (the bar, and the room round its parts: see `bar`, `inset`); its least width for a picture; its sizes for
/// words, and for what has no picture; how much of the screen it takes at most, and how far from the screen's edges and
/// what it was called up from it keeps.
/// (Its bar: its name's two lines, `NAME` tall, and `UNDER_NAME` under them; the room round its parts is its corners'
/// radius at least, see `inset`.)
const NAME: f32 = 36.0;
const UNDER_NAME: f32 = 12.0;
const INSET_LEAST: f32 = 20.0;
const BUTTON: f32 = 30.0;
const PICTURE_LEAST: f32 = 320.0;
const WORDS: (f32, f32) = (560.0, 340.0);
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
    /// Files together: each one's name and where it is.
    Files(Vec<(PathBuf, String, String)>),
    /// What has no picture: its icon (made as it shows), what kind of file it is.
    Other(Option<PathBuf>),
}

/// A thing read: what its body shows, and what it is said to be (a file's: its kind, size, when it was written).
#[derive(Clone)]
struct Read {
    body: Body,
    about: Option<String>,
}

/// What its reading thread is asked: the thing `at` of the look numbered `number` read, in `lang`, its pictures for a
/// screen `scale` px a DIP.
struct Reading {
    number: u64,
    at: usize,
    shows: Shows,
    lang: Lang,
    scale: f32,
}

/// What was read, for which look and which thing.
static READ: Mutex<Vec<(u64, usize, Read)>> = Mutex::new(Vec::new());
/// How large a thing is seen (see `measure`), for which look and which thing: known before it is read.
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
        if let Some(size) = measure(&asked.shows) {
            MEASURED.lock().unwrap().push((asked.number, asked.at, size));
            let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LOADED, WPARAM(0), LPARAM(0)) };
        }
        let read = read(&asked.shows, asked.lang, asked.scale, asked.number ^ ((asked.at as u64) << 40));
        READ.lock().unwrap().push((asked.number, asked.at, read));
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), LOADED, WPARAM(0), LPARAM(0)) };
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
        Shows::Files(paths) => {
            let rows = paths
                .iter()
                .map(|path| {
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                    (path.clone(), name, path.parent().map(|folder| folder.display().to_string()).unwrap_or_default())
                })
                .collect();
            Read { body: Body::Files(rows), about: None }
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
    // (Its kind's previewer, where it has one, and a picture standing for it meanwhile. Else only the picture: as
    // large as a page may be shown on this screen, as Explorer pictures it, its first page, a frame.)
    let previewer = if kind == Kind::Picture { None } else { pane::previewer(path) };
    let side = (1600.0 * scale.max(1.0)).min(2400.0) as u32;
    let pictured = previewer.is_some() || matches!(kind, Kind::Page | Kind::Video | Kind::Picture);
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
        self.read = vec![None; opening.items.len()];
        self.measured = vec![None; opening.items.len()];
        self.asked = vec![false; opening.items.len()];
        self.items = opening.items;
        self.at = at;
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
        self.items.clear();
        self.read.clear();
        self.measured.clear();
        self.asked.clear();
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
        self.read = read;
        self.items = items;
        self.at = at;
        if key == shown {
            self.read_around();
            return self.draw();
        }
        self.moved_from(was);
    }

    /// The thing now at `at` shown, moving from where the sheet was (`was`).
    fn moved_from(&mut self, was: Rect) {
        let (at, now) = (self.at, Instant::now());
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

    fn reset_view(&mut self) {
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
                let _ = self.reads.send(Reading { number: self.number, at: place, shows: self.items[place].shows.clone(), lang: self.lang, scale: self.scale });
            }
        }
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
        for (number, at, read) in read {
            if number != self.number || at >= self.read.len() || !self.asked[at] {
                continue;
            }
            if at == self.at {
                resized = true;
            }
            self.read[at] = Some(read);
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

    fn body(&self) -> Option<&Body> {
        self.read.get(self.at)?.as_ref().map(|read| &read.body)
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
        let work = self.on_window(self.work);
        let most = (room.w.min(MOST.0 * work.w), room.h.min(MOST.1 * work.h));
        let (w, h) = self.wanted(most);
        let (w, h) = (w.min(room.w).max(1.0), h.min(room.h).max(1.0));
        Rect { x: (room.x + (room.w - w) / 2.0).round(), y: (room.y + (room.h - h) / 2.0).round(), w: w.round(), h: h.round() }
    }

    /// How far in from its edges its parts are, all round (DIPs): as far as the centres of its corners' arcs, so that
    /// what is at a corner is concentric with it (glass's round ones), and no less than `INSET_LEAST`.
    fn inset(&self) -> f32 {
        Theme::new(self.skin, self.dark).radius.max(INSET_LEAST)
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
        let page = || {
            let h = most.1;
            (((h - top - pad) / PAGE + 2.0 * pad + PAGE_ROOM).min(most.0).max(PICTURE_LEAST), h)
        };
        let item = &self.items[self.at];
        let measured = self.measured.get(self.at).copied().flatten();
        match self.body() {
            Some(Body::Picture(_, own)) => pictured(*own, true),
            // (A video's picture stands for it, made no larger than may be asked: it is shown at its frames' size.)
            Some(Body::Pictured(picture) | Body::Live { picture: Some(picture), .. }) => pictured(measured.unwrap_or(picture.size), true),
            Some(Body::Live { picture: None, .. }) => measured.map_or_else(page, |size| pictured(size, true)),
            Some(Body::Words { .. } | Body::Files(_)) => (WORDS.0.min(most.0), WORDS.1.min(most.1)),
            Some(Body::Other(_)) => OTHER,
            // (Not read yet: as large as it was measured.)
            None => match measured {
                Some(size) => pictured(size, true),
                // (Neither read nor measured yet, the first moment it opens: as large as what its name says it is wants, a
                // picture of the shape its tile's is.)
                None => match kind_shown(&item.shows) {
                    Kind::Picture | Kind::Video => match &item.shows {
                        Shows::File(path) => crate::station::thumbs::of(path, 256).map_or(most, |thumb| pictured(thumb.size, false)),
                        _ => most,
                    },
                    Kind::Page => page(),
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
        unsafe {
            let _ = KillTimer(Some(self.hwnd), FRAME);
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
    }

    // ----- Its previewer -----

    /// The file shown, and its previewer, where one shows it: while the sheet rests.
    fn live(&self) -> Option<(PathBuf, windows::core::GUID)> {
        match (self.body()?, &self.items[self.at].shows) {
            (Body::Live { previewer, .. }, Shows::File(path)) if matches!(self.motion, Motion::Resting) => Some((path.clone(), *previewer)),
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
                let shown = matches!(&self.items[self.at].shows, Shows::File(file) if *file == path);
                if let Some(read) = self.read.get_mut(self.at).and_then(Option::as_mut).filter(|_| shown) {
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
        let about = self.read.get(self.at).and_then(|read| read.as_ref()?.about.clone()).unwrap_or_else(|| item.about.clone());
        let (title, small) = (self.font(13.0, 650.0), self.font(11.5, 400.0));
        let (title_text, open_label, pastes, opens, copies) = (item.title.clone(), if item.pastes { lang.pick("粘贴", "Paste") } else { lang.pick("打开", "Open") }, item.pastes, item.open.is_some(), item.copy.is_some());
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
        // Which of how many it is, before them.
        if self.items.len() > 1 {
            let said = format!("{} / {}", self.at + 1, self.items.len());
            let font = self.font(11.5, 400.0);
            let wide = frame.measure(&said, font);
            let (ascent, descent) = frame.gfx.baseline(font);
            right -= 4.0 + wide;
            frame.text(&said, font, theme.text3, right, pad + (NAME - ascent - descent) / 2.0, wide + 1.0, Align::Start);
            right -= 12.0;
        }
        // What it is called, and what it is, under that, as far down from its top as from its side.
        let room = (right - pad - 8.0).max(0.0);
        let top = pad;
        frame.text(&title_text, title, theme.text, pad, top, room, Align::Start);
        frame.text(&about, small, theme.text3, pad, top + 18.0, room, Align::Start);
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

    /// Its body: what the thing shows.
    fn paint_body(&mut self, frame: &Frame, theme: &Theme, (w, h): (f32, f32)) {
        let body = self.body_at((w, h));
        self.hits.push((body, Hit::Body));
        let radius = self.body_radius();
        let Some(read) = self.read.get(self.at).cloned().flatten() else {
            // (Being read: its tile's picture, where it has one, else nothing yet.)
            if let Shows::File(path) = &self.items[self.at].shows {
                if let Some(thumb) = crate::station::thumbs::of(path, 256) {
                    let (x, y, pw, ph) = thumb.fitted_in(body.x, body.y, body.w, body.h);
                    frame.picture(&thumb, x, y, pw, ph, radius, 1.0);
                }
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
            Body::Files(rows) => {
                const ROW: f32 = 40.0;
                self.scroll_most = (rows.len() as f32 * ROW - body.h).max(0.0);
                self.scroll = self.scroll.clamp(0.0, self.scroll_most);
                frame.clip(body.x, body.y, body.w, body.h);
                let (name_font, where_font) = (self.font(13.0, 500.0), self.font(11.5, 400.0));
                let side = (24.0 * self.scale).round() as u32;
                for (place, (path, name, folder)) in rows.iter().enumerate() {
                    let y = body.y + place as f32 * ROW - self.scroll;
                    if y + ROW < body.y || y > body.y + body.h {
                        continue;
                    }
                    if let Some(icon) = crate::station::thumbs::icon(path, side) {
                        frame.picture(&icon, body.x + 4.0, y + 8.0, 24.0, 24.0, 0.0, 1.0);
                    }
                    frame.text(name, name_font, theme.text, body.x + 38.0, y + 4.0, body.w - 42.0, Align::Start);
                    frame.text(folder, where_font, theme.text3, body.x + 38.0, y + 21.0, body.w - 42.0, Align::Start);
                }
                frame.unclip();
            }
            Body::Other(path) => {
                let item = &self.items[self.at];
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
                centred(&item.title, self.font(13.0, 600.0), theme.text, y);
                if let Some(about) = &read.about {
                    centred(about, self.font(12.0, 400.0), theme.text2, y + 22.0);
                }
                if item.open.is_some() {
                    let said = self.lang.pick("没有可用的预览，按 Enter 用默认程序打开", "No preview is available. Press Enter to open it in its program");
                    centred(said, self.font(12.0, 400.0), theme.text3, y + 42.0);
                }
            }
        }
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
        self.hits.iter().find(|(at, _)| x >= at.x && x < at.x + at.w && y >= at.y && y < at.y + at.h).map(|(_, hit)| *hit)
    }

    /// Whether the picture shown may be zoomed or moved: a picture, at rest.
    fn pictured(&self) -> bool {
        matches!(self.body(), Some(Body::Picture(..) | Body::Pictured(..))) && matches!(self.motion, Motion::Resting)
    }

    fn moved(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        self.pointer = Some(on);
        if let Some((from, pan)) = self.dragging {
            self.pan = (pan.0 + on.0 - from.0, pan.1 + on.1 - from.1);
            self.keep_in_view();
        }
        self.draw();
    }

    fn press(&mut self, at: (i32, i32)) {
        let on = self.on_sheet(at);
        self.pressed = self.hit_at(on);
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
        match pressed.filter(|pressed| self.hit_at(on) == Some(*pressed)) {
            Some(Hit::Open) => self.key(Key::Open),
            Some(Hit::Copy) => self.key(Key::Copy),
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
        } else if matches!(self.body(), Some(Body::Words { .. } | Body::Files(_))) {
            self.scroll = (self.scroll - turns * 3.0 * 22.0).clamp(0.0, self.scroll_most);
        }
        self.draw();
    }

    // ----- The keys -----

    fn key(&mut self, key: Key) {
        if !self.up() {
            return;
        }
        match key {
            Key::Close => self.close(),
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

/// `rect` made `k` as large about its middle.
fn shrunk(rect: Rect, k: f32) -> Rect {
    Rect { x: rect.x + rect.w * (1.0 - k) / 2.0, y: rect.y + rect.h * (1.0 - k) / 2.0, w: rect.w * k, h: rect.h * k }
}

/// The box `p` of the way from `a` to `b`.
fn between(a: Rect, b: Rect, p: f32) -> Rect {
    let mix = |a: f32, b: f32| a + (b - a) * p;
    Rect { x: mix(a.x, b.x), y: mix(a.y, b.y), w: mix(a.w, b.w), h: mix(a.h, b.h) }
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
            let on_button = with(|looker| looker.pointer.and_then(|on| looker.hit_at(on)).is_some_and(|hit| matches!(hit, Hit::Open | Hit::Copy))).unwrap_or(false);
            unsafe { SetCursor(LoadCursorW(None, if on_button { IDC_HAND } else { IDC_ARROW }).ok()) };
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
                FRAME => with(Looker::draw),
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
    #[test]
    fn boxes_move_between() {
        let (a, b) = (Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Rect { x: 100.0, y: 50.0, w: 210.0, h: 110.0 });
        assert_eq!(between(a, b, 0.0), a);
        assert_eq!(between(a, b, 1.0), b);
        assert_eq!(between(a, b, 0.5), Rect { x: 50.0, y: 25.0, w: 110.0, h: 60.0 });
        assert_eq!(shrunk(Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }, 0.5), Rect { x: 25.0, y: 12.5, w: 50.0, h: 25.0 });
    }
}
