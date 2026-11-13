//! The station's own thread: a single-threaded apartment with OLE (what dragging and dropping between programs needs,
//! which the windows' threads, in the multithreaded one, cannot do), its own message loop, the station's shelf and
//! folder held there alone. Other threads only ask it (`ask`) and read what it shows (`shown`), never its state: a
//! message there, a copy of what it holds back.
//!
//! What is copied is heard here (Recently copied), as the settings say: read as it changes, by its sequence number
//! (see `clipboard`), the clipboard read again in a moment where another program holds it.

use super::clipboard::{self, Busy, Content, Why};
use super::drop::Dragged;
use super::folder::Folder;
use super::keep::{Keep, Refused};
use super::shelf::Item;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, RegisterClassW, SetTimer, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLIPBOARDUPDATE, WM_TIMER,
    WNDCLASSW,
};

/// What the station is asked.
#[derive(Clone, Debug)]
pub enum Ask {
    /// What is on the clipboard put on it (Ctrl+V on it).
    Paste,
    /// What was dropped on it (see `drop`).
    Take(Content),
    Pin(u64, bool),
    Remove(u64),
    /// All that is not pinned taken off: of what was put on it, or (with `true`) of what was copied lately.
    Clear(bool),
    /// What `id` holds put on the clipboard.
    Copy(u64),
    /// What `id` holds put on the clipboard, and pasted where the keyboard is (Ctrl+V sent to the program in front).
    PasteOut(u64),
    /// What `id` holds opened as Explorer opens it (a web address in the browser); shown in Explorer, selected there.
    Open(u64),
    Show(u64),
    /// The settings changed (see `settings::StationSettings`).
    Settings,
    /// Something dragged is over a station now, or no longer.
    Over(bool),
    AppsOver(bool),
    /// What is given dragged off it as one (pressed on it and moved: see `drag_begins`), pressed on a window of the
    /// thread numbered so.
    DragOut(Vec<Given>, u32),
    /// What is given (chosen together: see `select`) put on the clipboard as one, or shown in Explorer, those of it
    /// that are files.
    Together(Vec<Given>, Together),
    /// Files not on it (Recent files'): put on the clipboard, opened, shown in Explorer.
    Files(Vec<std::path::PathBuf>, FilesAct),
    /// Text put on the clipboard (a file's path), and what it was copied from, to be marked copied.
    CopyText(String, Option<Copied>),
    /// What is kept at hand opened, as the shell opens it (see `apps`).
    Launch(String),
    /// What came from devices nearby taken in (see `nearby`), and (`Some`) the sending that ended, no longer coming.
    Arrivals(Option<String>),
    /// What a device nearby is sending, as it comes (none: nothing, or the helper gone).
    Receiving(Option<crate::nearby::Receiving>),
    /// What came from a device nearby (these things on it) drawn as coming in from `at` on: carried onto the station,
    /// it comes in as it lands (see `ui::peek`).
    Landing(Vec<u64>, std::time::Instant),
    /// A sending to a device nearby over (see `send`): its own files it held let go of.
    Sent(u64),
}

/// What is done with files not on the station (see `Ask::Files`): put on the clipboard (copied from the files come
/// lately of the entry, to be marked copied there, where they were), opened, shown in Explorer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilesAct {
    Copy(Option<super::recent::Entry>),
    Open,
    Show,
}

/// What is done with things chosen together (see `Ask::Together`): put on the clipboard as one, or shown in Explorer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Together {
    Copy,
    Show,
}

/// What was put on the clipboard from where it shows (marked copied there a moment: see `Shown::copied`): a thing on
/// the station or copied lately, by its number, or files come lately, by their entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Copied {
    Item(u64),
    Recent(super::recent::Entry),
}

/// How long what came from a device nearby is shown coming in (see `Shown::arrived`): its picture fading in, then a
/// glow round it.
pub const ARRIVED_FOR: std::time::Duration = std::time::Duration::from_millis(1100);

/// How long what was copied is marked so (see `Shown::copied`).
pub const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1400);

/// What the station is asked to give (dragged off, copied, shown together): a thing on it, by its number, or files
/// not on it.
#[derive(Clone, Debug, PartialEq)]
pub enum Given {
    Item(u64),
    Files(Vec<std::path::PathBuf>),
}

/// What leaves the station as it is dragged off (see `drag_off`): the thing on it it is (none: files not on it), what
/// it gives, whether it is never to leave this PC, and the files of its own it holds meanwhile.
struct Leaving {
    id: Option<u64>,
    content: Content,
    stays_here: bool,
    held: Vec<String>,
}

/// What the station shows: what it holds, oldest first, and how it stands.
#[derive(Clone, Debug, Default)]
pub struct Shown {
    pub items: Vec<Item>,
    /// Recently copied on.
    pub history: bool,
    /// How what is on it shows, what was copied, and the files come lately.
    pub view: crate::settings::ShelfView,
    pub copies_view: crate::settings::ShelfView,
    pub recent_view: crate::settings::ShelfView,
    /// Why what was last given could not be taken, where it could not.
    pub refused: Option<Refused>,
    /// Something is dragged over a station, to be dropped; over what is kept at hand, to be kept there.
    pub over: bool,
    pub apps_over: bool,
    /// What was last put on the clipboard from where it shows (several, put there together), and when (the clipboard
    /// took it): marked copied there for `COPIED_FOR`.
    pub copied: Option<(Vec<Copied>, std::time::Instant)>,
    /// What a device nearby is sending, as it comes: shown first on it, till it has come.
    pub receiving: Option<crate::nearby::Receiving>,
    /// What last came from devices nearby (its number on it), and when: shown coming in for `ARRIVED_FOR`.
    pub arrived: Option<(Vec<u64>, std::time::Instant)>,
}

impl Shown {
    /// Whether what is on it, in its order, is other than what `before` showed: a thing put on it, taken off, moved, or
    /// something coming from a device nearby, or come. (What was copied lately is not: it comes as anything is copied,
    /// and on a page of its own.)
    pub fn moved_from(&self, before: &Shown) -> bool {
        let order = |shown: &Shown| shown.items.iter().filter(|item| !item.copied).map(|item| item.id).collect::<Vec<u64>>();
        let coming = |shown: &Shown| shown.receiving.as_ref().map(|receiving| receiving.session.clone());
        order(self) != order(before) || coming(self) != coming(before)
    }
}

thread_local! {
    /// What this thread draws the station as, for a moment (see `as_was`).
    static AS_WAS: std::cell::RefCell<Option<Arc<Shown>>> = const { std::cell::RefCell::new(None) };
}

/// What `draw` gives, the station drawn as `was` showed it (what a drawing turns from, as it changed since: see
/// `render::Turn`).
pub fn as_was<T>(was: Arc<Shown>, draw: impl FnOnce() -> T) -> T {
    let before = AS_WAS.with(|as_was| as_was.borrow_mut().replace(was));
    let drawn = draw();
    AS_WAS.with(|as_was| *as_was.borrow_mut() = before);
    drawn
}

/// Its window's message: asked something (see `ask`).
const ASKED: u32 = WM_APP + 1;
/// Its timers: the clipboard read again (held by another program as it changed), what has stayed its days taken off.
const AGAIN: usize = 1;
const EXPIRE: usize = 2;
const WATCH: usize = 3;
/// How often it looks whether something is dragged (see `catch`, ms).
const WATCH_EVERY_MS: u32 = 100;
/// How soon the clipboard is read again, and how many times at most.
const AGAIN_AFTER_MS: u32 = 60;
const AGAIN_MOST: u32 = 8;
/// How often what has stayed its days is looked for (ms).
const EXPIRE_EVERY_MS: u32 = 60 * 60 * 1000;

static ASKS: OnceLock<Mutex<Sender<Ask>>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static SHOWN: Mutex<Option<Arc<Shown>>> = Mutex::new(None);
/// Where each window shows a station, as it was last drawn (screen px): where what is dragged over it may be dropped.
static AREAS: Mutex<Vec<(isize, windows::Win32::Foundation::RECT, Lands)>> = Mutex::new(Vec::new());

/// The window `window` shows a station, or quick launch, in `areas` (screen px, each with where what is dropped there
/// goes; none: none), as it was just drawn.
pub fn shows_at(window: HWND, areas: Vec<(windows::Win32::Foundation::RECT, Lands)>) {
    let mut held = AREAS.lock().unwrap();
    held.retain(|(of, ..)| *of != window.0 as isize);
    held.extend(areas.into_iter().map(|(area, lands)| (window.0 as isize, area, lands)));
}

/// Where stations and quick launch are shown (screen px), by the windows still shown that show them.
pub fn areas() -> Vec<(HWND, windows::Win32::Foundation::RECT, Lands)> {
    let mut held = AREAS.lock().unwrap();
    held.retain(|(of, ..)| unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(HWND(*of as *mut _)) }.as_bool());
    held.iter().map(|(of, area, lands)| (HWND(*of as *mut _), *area, *lands)).collect()
}

/// Where what is dropped goes: onto the station, or into what is kept at hand (see `apps`). Over where a page of the
/// workbench is chosen (its tab), that page: a drag over it shows the page there (see `SHOW_PAGE`), and one let go
/// on it goes where the page takes it (the station's, onto the station; the others take none). On a device nearby
/// on the strip (by its key, see `send::key_of`), sent to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lands {
    Station,
    Apps,
    Pages(crate::ui::view::Tab),
    Device(u64),
}

/// Posted to a window (the panel's, a widget's) as something dragged comes over where a page of its workbench is
/// chosen: that page shown (WPARAM: which, see `Tab::number`).
pub const SHOW_PAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 20;

/// What a window catching drops saw (see `catch`; on the station's thread, perhaps as it does something already: so
/// told it by message): a drag over a strip along `edge` opens the station there, as a push into the edge does; one
/// over where a page is chosen in `window` shows that page there.
pub fn caught(dragged: Dragged, edge: Option<crate::settings::Edge>, lands: Lands, window: Option<HWND>) {
    if lands == Lands::Apps {
        // Over it: what is kept at hand says it takes what is let go (see `Shown::apps_over`); gone, no more.
        ask(Ask::AppsOver(matches!(dragged, Dragged::Over(_))));
        // Files, folders and shortcuts dropped, and web addresses, kept at hand after what is there. (Told apart on a
        // thread of their own: a path on a drive slow to answer would else hold the station's thread, and the
        // clipboard with it.)
        if let Dragged::Dropped(given) = dragged {
            let paths: Vec<std::path::PathBuf> = given.iter().flat_map(|content| if let Content::Files(paths) = content { paths.clone() } else { Vec::new() }).collect();
            let urls: Vec<String> = given.iter().filter_map(|content| if let Content::Text(text) = content { Some(text.trim().to_string()) } else { None }).collect();
            std::thread::spawn(move || {
                // (In a COM apartment: a shortcut dropped is read for what it opens, see `apps::opens`.)
                let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
                let added: Vec<super::apps::App> = paths.iter().filter_map(|path| super::apps::App::of_path(path)).chain(urls.iter().filter_map(|url| super::apps::App::of_url(url))).collect();
                // (What opens what is kept already, however it is named, is not kept twice.)
                added.iter().for_each(|app| {
                    super::apps::opens(&app.target);
                });
                if !added.is_empty() {
                    super::apps::change(move |apps| {
                        for app in added {
                            if !apps.iter().any(|kept| super::apps::same(&kept.target, &app.target)) {
                                apps.push(app);
                            }
                        }
                    });
                }
            });
        }
        return;
    }
    if let Lands::Pages(page) = lands {
        if let (Dragged::Over(_), Some(window)) = (&dragged, window) {
            let _ = unsafe { PostMessageW(Some(window), SHOW_PAGE, WPARAM(page.number()), LPARAM(0)) };
        }
        // (A page other than the station's takes nothing: only shown.)
        if page != crate::ui::view::Tab::Shelf {
            return;
        }
    }
    match dragged {
        Dragged::Over(at) => {
            if let Some(edge) = edge {
                crate::app().controller.pushed(edge, (at.x, at.y));
            }
            ask(Ask::Over(true));
        }
        Dragged::Gone => ask(Ask::Over(false)),
        Dragged::Dropped(given) => {
            for content in given {
                ask(Ask::Take(content));
            }
            ask(Ask::Over(false));
        }
    }
}

/// Called (on the station's thread) as what it shows changes: to wake what shows it.
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Starts the station's thread; `told` is called as what it shows changes.
pub fn start(told: impl Fn() + Send + Sync + 'static) {
    let _ = TOLD.set(Box::new(told));
    let (sender, asks) = std::sync::mpsc::channel();
    if ASKS.set(Mutex::new(sender)).is_err() {
        return;
    }
    std::thread::Builder::new().name("glance: station".into()).spawn(move || run(asks)).expect("a thread started");
}

/// Asks the station `ask` (done on its thread, soon).
pub fn ask(ask: Ask) {
    let Some(asks) = ASKS.get() else { return };
    if asks.lock().unwrap().send(ask).is_ok() {
        let window = WINDOW.load(Ordering::Acquire);
        if window != 0 {
            let _ = unsafe { PostMessageW(Some(HWND(window as *mut _)), ASKED, WPARAM(0), LPARAM(0)) };
        }
    }
}

#[cfg(any(test, feature = "studio"))]
thread_local! {
    /// What this thread's pictures pretend (see `pretend`): each test's its own, as tests run side by side.
    static PRETENDED: std::cell::RefCell<Option<Arc<Shown>>> = const { std::cell::RefCell::new(None) };
}

/// What the station shows now.
pub fn shown() -> Arc<Shown> {
    if let Some(was) = AS_WAS.with(|as_was| as_was.borrow().clone()) {
        return was;
    }
    #[cfg(any(test, feature = "studio"))]
    if let Some(pretended) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

/// For pictures: as if the station held `items`, shown as `view` (its thread not started).
#[cfg(any(test, feature = "studio"))]
pub fn pretend(items: Vec<Item>, view: crate::settings::ShelfView) {
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some(Arc::new(Shown { items, history: true, view, copies_view: view, recent_view: view, refused: None, over: false, apps_over: false, copied: None, receiving: None, arrived: None })));
}

/// For tests: what is pretended (see `pretend`) just put on the clipboard as `copied` (see `Shown::copied`).
#[cfg(test)]
pub fn pretend_copied(copied: Vec<Copied>) {
    PRETENDED.with(|pretended| {
        if let Some(shown) = pretended.borrow_mut().as_mut() {
            Arc::make_mut(shown).copied = Some((copied, std::time::Instant::now()));
        }
    });
}

/// For pictures: what is pretended (see `pretend`) with Recently copied shown as `copies` and Recent files as `recent`.
#[cfg(feature = "studio")]
pub fn pretend_pages(copies: crate::settings::ShelfView, recent: crate::settings::ShelfView) {
    PRETENDED.with(|pretended| {
        if let Some(shown) = pretended.borrow_mut().as_mut() {
            let shown = Arc::make_mut(shown);
            shown.copies_view = copies;
            shown.recent_view = recent;
        }
    });
}

/// `given` opened as Explorer opens it: each file (a folder in Explorer), or a web address in the browser.
fn open(given: &Content) {
    let shell = |what: &std::ffi::OsStr| unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(None, w!("open"), &windows::core::HSTRING::from(what), None, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
    };
    match given {
        Content::Files(paths) => paths.iter().for_each(|path| shell(path.as_os_str())),
        Content::Text(text) if text.starts_with("https://") || text.starts_with("http://") => shell(std::ffi::OsStr::new(text)),
        // Words: as a text file of their own (named by their first line, as they are dragged off), in what opens one.
        Content::Text(text) => {
            if let Some(file) = text_file(text) {
                shell(file.as_os_str());
            }
        }
        Content::Png(_) | Content::Saved { .. } => {}
    }
}

/// Ctrl+V pressed for the program in front, where the keyboard is: what was just put on the clipboard pasted there.
fn paste_in_front() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_V};
    let key = |key: VIRTUAL_KEY, up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: key, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() } } };
    let keys = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
    unsafe { SendInput(&keys, std::mem::size_of::<INPUT>() as i32) };
}

/// `text` written as a text file to be opened (UTF-8, marked so; named by its first line, see `drag::text_name`), in a
/// folder of Windows' temporary ones that is its own (one for each text: two of the same first line are two files).
/// Opened before and changed since (written in, and saved), that file is left as it is: another holds the text.
fn text_file(text: &str) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut digest);
    let folder = std::env::temp_dir().join("Glance").join("opened").join(format!("{:016x}", digest.finish()));
    std::fs::create_dir_all(&folder).ok()?;
    let name = std::path::PathBuf::from(super::drag::text_name(text));
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let extension = name.extension().unwrap_or_default().to_string_lossy();
    let contents: Vec<u8> = [0xEF, 0xBB, 0xBF].into_iter().chain(text.bytes()).collect();
    for again in 1u32.. {
        let file = if again == 1 { folder.join(&name) } else { folder.join(format!("{stem} ({again}).{extension}")) };
        match std::fs::read(&file) {
            Ok(held) if held == contents => return Some(file),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&file, &contents).ok()?;
                return Some(file);
            }
            Err(_) => return None,
        }
    }
    unreachable!("a name is found")
}

/// `act` done with the shell (opening, showing in Explorer) on a thread of its own, a single-threaded apartment as the
/// shell wants: the station's thread never waits on Explorer (showing a file in a library's folder could keep it waiting
/// for good, and with it everything the station does).
/// The reader told that `app`, kept at hand, is no longer where it was, and asked whether it is to be taken off.
fn ask_gone(app: &super::apps::App) {
    let lang = crate::update::language();
    let title = match lang {
        crate::ui::text::Lang::Zh => format!("找不到“{}”", app.name),
        crate::ui::text::Lang::En => format!("\"{}\" is not found", app.name),
    };
    let asked = crate::ui::ask::Asked {
        title: &title,
        intro: lang.pick("它可能已被移动、重命名或删除。可以将它从快捷 App 中移除。", "It may have been moved, renamed or deleted. It can be taken off quick launch."),
        own: None,
        option: None,
        sent: None,
        from_end: false,
        yes: lang.pick("移除", "Take off"),
        no: lang.pick("取消", "Cancel"),
        dark: crate::os::apps_dark(),
    };
    // (Its graphics, made for the screen it is asked on, let go of before the thread ends: see `gfx::held_till_done`.)
    let _held = crate::ui::gfx::held_till_done();
    if crate::ui::ask::show(windows::Win32::Foundation::HWND::default(), &asked).is_some() {
        let gone = app.clone();
        super::apps::change(move |apps| apps.retain(|kept| *kept != gone));
    }
}

fn by_the_shell(act: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("glance: shell".into()).spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        act();
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    });
}

/// `paths` shown in Explorer, selected: each folder they are in opened with those of them in it selected.
fn show_in_explorer(paths: &[std::path::PathBuf]) {
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFindLastID, ILFree, SHOpenFolderAndSelectItems};
    let mut folders: Vec<(std::path::PathBuf, Vec<&std::path::PathBuf>)> = Vec::new();
    for path in paths {
        let Some(parent) = path.parent() else { continue };
        match folders.iter_mut().find(|(folder, _)| folder == parent) {
            Some((_, inside)) => inside.push(path),
            None => folders.push((parent.to_path_buf(), vec![path])),
        }
    }
    for (folder, inside) in folders {
        unsafe {
            let held = ILCreateFromPathW(&windows::core::HSTRING::from(folder.as_os_str()));
            if held.is_null() {
                continue;
            }
            let items: Vec<_> = inside.iter().map(|path| ILCreateFromPathW(&windows::core::HSTRING::from(path.as_os_str()))).filter(|item| !item.is_null()).collect();
            let last: Vec<_> = items.iter().map(|item| ILFindLastID(*item) as *const _).collect();
            let _ = SHOpenFolderAndSelectItems(held, Some(&last), 0);
            items.into_iter().for_each(|item| ILFree(Some(item)));
            ILFree(Some(held));
        }
    }
}

/// The station on its thread.
struct Station {
    keep: Keep,
    window: HWND,
    asks: Receiver<Ask>,
    /// Listening to the clipboard.
    listening: bool,
    /// The clipboard's sequence number of the last copy taken, and of what the station put there itself.
    taken: u32,
    ours: u32,
    /// Times the clipboard was found held by another since it changed.
    tries: u32,
    /// Something dragged over a station, to be dropped (see `drop`).
    over: bool,
    apps_over: bool,
    /// What was last put on the clipboard from where it shows, and when (see `Shown::copied`).
    copied: Option<(Vec<Copied>, std::time::Instant)>,
    /// Where drops are caught while something is dragged (see `catch`).
    catchers: super::catch::Catchers,
    /// What is to be dragged off it, asked, and the thread it was pressed on (dragged once it is done answering: see
    /// `drag_off`).
    dragging: Option<(Vec<Given>, u32)>,
    /// What a device nearby is sending, and what came last, and when (see `Shown`).
    receiving: Option<crate::nearby::Receiving>,
    arrived: Option<(Vec<u64>, std::time::Instant)>,
    /// Its own files each sending to a device nearby holds till it is over (see `send`), by the sending's number.
    sending: std::collections::HashMap<u64, Vec<String>>,
}

thread_local! {
    static STATION: std::cell::RefCell<Option<Station>> = const { std::cell::RefCell::new(None) };
}

fn run(asks: Receiver<Ask>) {
    unsafe {
        if windows::Win32::System::Ole::OleInitialize(None).is_err() {
            crate::journal::note("station: OLE could not be begun; no station");
            return;
        }
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceStation"), ..Default::default() };
        RegisterClassW(&class);
        let Ok(window) = CreateWindowExW(WINDOW_EX_STYLE(0), class.lpszClassName, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) else {
            crate::journal::note("station: its window could not be made; no station");
            return;
        };
        let settings = crate::app().settings.lock().unwrap().station;
        // (What an earlier Glance wrote to send, its sendings long over.)
        let _ = std::fs::remove_dir_all(super::send::folder());
        let mut keep = Keep::open(Folder::own(), settings.rules(), settings.always_copy);
        keep.expire(super::now_ms());
        STATION.with_borrow_mut(|station| *station = Some(Station { keep, window, asks, listening: false, taken: 0, ours: 0, tries: 0, over: false, apps_over: false, copied: None, catchers: Default::default(), dragging: None, receiving: None, arrived: None, sending: Default::default() }));
        WINDOW.store(window.0 as isize, Ordering::Release);
        let _ = SetTimer(Some(window), EXPIRE, EXPIRE_EVERY_MS, None);
        let _ = SetTimer(Some(window), WATCH, WATCH_EVERY_MS, None);
        with_station(|station| {
            station.follow_settings();
            station.answer();
        });
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            DispatchMessageW(&message);
        }
    }
}

/// What was asked to be dragged off the station, dragged as one (outside `with_station`: the drag's own loop passes the
/// station's messages on meanwhile, which find it free), its own files held meanwhile, and what of it is on it taken
/// off after, where the settings say so. Called after each of the station's messages (those the drag's own loop passes
/// on among them): nothing asked, nothing done.
fn drag_off() {
    let mut asked = None;
    with_station(|station| asked = station.dragging.take().map(|(given, pressed_on)| (given.into_iter().filter_map(|given| station.leaving(given)).collect::<Vec<Leaving>>(), pressed_on)));
    // (Asked, and nothing of it there to give: the drag asked for is over, as given up, one under way, or none, is
    // left as it is.)
    let Some((leaving, thread)) = asked else { return };
    if leaving.is_empty() {
        super::drag::ended(super::drag::Dragged::Not);
        return;
    }
    let contents: Vec<Content> = leaving.iter().map(|one| one.content.clone()).collect();
    let dragged = super::drag::drag(&contents, leaving.iter().any(|one| one.stays_here), thread);
    // Let go on a device nearby: sent to it, its own files held till the sending is over, and left on it.
    let sent = if dragged == Some(super::drag::Dragged::Taken) { super::send::taken() } else { None };
    with_station(|station| {
        let held = leaving.iter().flat_map(|one| one.held.iter().cloned());
        match &sent {
            Some((job, _)) => {
                station.sending.insert(*job, held.collect());
            }
            None => held.for_each(|name| station.keep.folder.let_go(&name)),
        }
        let remove = crate::app().settings.lock().unwrap().station.remove_after_drag;
        if remove && dragged == Some(super::drag::Dragged::Taken) && sent.is_none() {
            for id in leaving.iter().filter_map(|one| one.id) {
                station.keep.remove(id);
            }
        }
        station.show();
    });
    // (Its files told apart on a thread of their own: a folder's are listed one by one, on a drive slow to answer
    // perhaps.)
    if let Some((job, to)) = sent {
        std::thread::spawn(move || crate::nearby::send(job, &to, &super::send::outgoing(&contents, &super::send::folder_of(job))));
    }
    // (Said once it is shown as it is after: taken off it, it is gone as it is put down, see `carry`.)
    super::drag::ended(dragged.unwrap_or(super::drag::Dragged::Not));
}

/// Where a drag off a station begun at `from` is at `now` (screen px): whether it has gone far enough to be one (the
/// system's distance for a drag).
pub fn drag_begins(from: windows::Win32::Foundation::POINT, now: windows::Win32::Foundation::POINT) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDRAG, SM_CYDRAG};
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
    (now.x - from.x).abs() > w / 2 || (now.y - from.y).abs() > h / 2
}

/// Does `act` with the station; whether it could (not while it is acting already: a message sent to its window while
/// it acts, as the clipboard may send its own, is seen to after).
fn with_station(act: impl FnOnce(&mut Station)) -> bool {
    STATION.with(|station| match station.try_borrow_mut() {
        Ok(mut station) => {
            if let Some(station) = station.as_mut() {
                act(station);
            }
            true
        }
        Err(_) => false,
    })
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        ASKED | WM_CLIPBOARDUPDATE | WM_TIMER => {
            let done = with_station(|station| match message {
                ASKED => station.answer(),
                WM_CLIPBOARDUPDATE => {
                    station.tries = 0;
                    station.copied();
                }
                _ if wparam.0 == AGAIN => {
                    let _ = unsafe { KillTimer(Some(window), AGAIN) };
                    station.copied();
                }
                _ if wparam.0 == WATCH => station.catchers.follow(),
                _ => {
                    station.keep.expire(super::now_ms());
                    station.show();
                }
            });
            if !done {
                let _ = unsafe { PostMessageW(Some(window), message, wparam, lparam) };
            }
            drag_off();
        }
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
    LRESULT(0)
}

impl Station {
    /// Does what it was asked, in turn.
    fn answer(&mut self) {
        while let Ok(ask) = self.asks.try_recv() {
            match ask {
                Ask::Paste => match clipboard::read(self.window, Why::Pasted) {
                    Ok(Some(copied)) => {
                        let _ = self.keep.take(copied.content, false, copied.stays_here, super::now_ms());
                    }
                    Ok(None) => {}
                    Err(Busy) => crate::journal::note("station: the clipboard was held by another program as it was pasted"),
                },
                Ask::Take(content) => {
                    let _ = self.keep.take(content, false, false, super::now_ms());
                }
                Ask::Pin(id, pinned) => self.keep.pin(id, pinned),
                Ask::Remove(id) => self.keep.remove(id),
                Ask::Clear(copied) => self.keep.clear(copied),
                Ask::Copy(id) | Ask::PasteOut(id) => {
                    let given = self.keep.shelf.get(id).and_then(|item| Some((self.keep.content_of(item)?, item.stays_here)));
                    if let Some((content, stays_here)) = given {
                        if self.copy(&content, stays_here, vec![Copied::Item(id)]) && matches!(ask, Ask::PasteOut(_)) {
                            paste_in_front();
                        }
                    }
                }
                Ask::Open(id) => {
                    if let Some(given) = self.opened(id) {
                        by_the_shell(move || open(&given));
                    }
                }
                Ask::Show(id) => {
                    if let Some(Content::Files(paths)) = self.opened(id) {
                        by_the_shell(move || show_in_explorer(&paths));
                    }
                }
                Ask::Settings => self.follow_settings(),
                Ask::Over(over) => self.over = over,
                Ask::AppsOver(over) => self.apps_over = over,
                Ask::DragOut(given, thread) => self.dragging = Some((given, thread)),
                Ask::Files(paths, act) => match act {
                    FilesAct::Copy(entry) => {
                        self.copy(&Content::Files(paths), false, entry.map(Copied::Recent).into_iter().collect());
                    }
                    FilesAct::Open => by_the_shell(move || open(&Content::Files(paths))),
                    FilesAct::Show => by_the_shell(move || show_in_explorer(&paths)),
                },
                Ask::Launch(target) => by_the_shell(move || {
                    // A file or folder no longer where it was kept: asked whether it is to be taken off.
                    let kept = super::apps::shown().iter().find(|app| app.target == target).cloned();
                    if let Some(app) = kept.filter(|app| app.location().is_some_and(|path| !path.exists())) {
                        ask_gone(&app);
                        return;
                    }
                    unsafe {
                        windows::Win32::UI::Shell::ShellExecuteW(None, w!("open"), &windows::core::HSTRING::from(target.as_str()), None, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
                    }
                }),
                Ask::Arrivals(ended) => {
                    self.arrivals();
                    // (Shown coming till what came is on it: the two in one showing.)
                    if self.receiving.as_ref().is_some_and(|receiving| Some(&receiving.session) == ended.as_ref()) {
                        self.receiving = None;
                    }
                }
                Ask::Receiving(receiving) => self.receiving = receiving,
                Ask::Landing(ids, at) => self.arrived = Some((ids, at)),
                Ask::Sent(job) => {
                    for name in self.sending.remove(&job).into_iter().flatten() {
                        self.keep.folder.let_go(&name);
                    }
                    let _ = std::fs::remove_dir_all(super::send::folder_of(job));
                }
                Ask::CopyText(text, from) => {
                    self.copy(&Content::Text(text), false, from.into_iter().collect());
                }
                Ask::Together(given, Together::Copy) => {
                    let marks: Vec<Copied> = given.iter().filter_map(|given| if let Given::Item(id) = given { Some(Copied::Item(*id)) } else { None }).collect();
                    let leaving: Vec<Leaving> = given.into_iter().filter_map(|given| self.leaving(given)).collect();
                    let contents: Vec<Content> = leaving.iter().map(|one| one.content.clone()).collect();
                    let stays_here = leaving.iter().any(|one| one.stays_here);
                    for name in leaving.iter().flat_map(|one| &one.held) {
                        self.keep.folder.let_go(name);
                    }
                    if let Some(one) = super::drag::as_one(&contents) {
                        self.copy(&one, stays_here, marks);
                    }
                }
                Ask::Together(given, Together::Show) => {
                    let paths: Vec<std::path::PathBuf> = given
                        .into_iter()
                        .flat_map(|given| match given {
                            Given::Item(id) => match self.opened(id) {
                                Some(Content::Files(paths)) => paths,
                                _ => Vec::new(),
                            },
                            Given::Files(paths) => paths,
                        })
                        .collect();
                    if !paths.is_empty() {
                        by_the_shell(move || show_in_explorer(&paths));
                    }
                }
            }
        }
        self.show();
    }

    /// What came from devices nearby (each receipt the helper wrote) taken onto the shelf, files into recent files too;
    /// each receipt taken away once what it holds is kept. One whose files or words cannot be kept now (the station is
    /// full, or they cannot be moved) stays, with those after it, to be taken in again.
    fn arrivals(&mut self) {
        let Some(folder) = crate::nearby::helper_folder() else { return };
        let inbox = folder.join("inbox");
        let mut came = Vec::new();
        for (path, receipt) in crate::nearby::receipts(&folder) {
            let from = receipt.device();
            let alias = from.alias.clone();
            // (Words: each a thing of its own, never with files.)
            if !receipt.words.is_empty() {
                match self.keep.words_arrived(&receipt.words, &from, super::now_ms()) {
                    Ok(ids) => {
                        let _ = std::fs::remove_file(&path);
                        came.extend(ids);
                        continue;
                    }
                    Err(refused) => {
                        crate::journal::note(format!("nearby: words that came are not taken in now ({refused:?})"));
                        break;
                    }
                }
            }
            match self.keep.arrived(&receipt.session, &receipt.files_in(&inbox), from, super::now_ms()) {
                Ok(taken) => {
                    let listed: Vec<(std::path::PathBuf, u64)> = taken.iter().filter_map(|(arrived, size)| Some((self.keep.folder.named(&arrived.file, &arrived.name)?, *size))).collect();
                    super::recent::arrived(&listed, &alias);
                    let _ = std::fs::remove_file(&path);
                    // (What holds them: the newest, last.)
                    if !taken.is_empty() {
                        came.extend(self.keep.shelf.items.last().map(|item| item.id));
                    }
                }
                Err(refused) => {
                    crate::journal::note(format!("nearby: what came is not taken in now ({refused:?})"));
                    break;
                }
            }
        }
        if !came.is_empty() {
            came.dedup();
            self.arrived = Some((came, std::time::Instant::now()));
        }
    }

    /// `id` as it is opened, shown in Explorer or dragged off: a picture of its own as its file (what Explorer and the
    /// chat programs take where it is dropped, and what a viewer opens; copied, it is the picture), the rest as it is
    /// copied.
    fn opened(&self, id: u64) -> Option<Content> {
        let item = self.keep.shelf.get(id)?;
        match &item.what {
            super::shelf::What::Picture { file } => Some(Content::Files(vec![self.keep.folder.path(file)])),
            _ => self.keep.content_of(item),
        }
    }

    /// What `given` dragged off it gives (see `drag_off`), its own files held meanwhile: none where it is no longer
    /// on it. (Files not its own: given where they are, never taken off anything.)
    fn leaving(&mut self, given: Given) -> Option<Leaving> {
        let id = match given {
            Given::Item(id) => id,
            Given::Files(paths) => return Some(Leaving { id: None, content: Content::Files(paths), stays_here: false, held: Vec::new() }),
        };
        let item = self.keep.shelf.get(id)?.clone();
        let content = self.opened(id)?;
        let held: Vec<String> = item.what.own_files().into_iter().map(str::to_string).collect();
        for name in &held {
            self.keep.folder.hold(name);
        }
        Some(Leaving { id: Some(id), content, stays_here: item.stays_here, held })
    }

    /// The settings as they are now: the shelf's rules, and the clipboard listened to while Recently copied is on.
    fn follow_settings(&mut self) {
        let settings = crate::app().settings.lock().unwrap().station;
        self.keep.rules = settings.rules();
        self.keep.always_copy = settings.always_copy;
        let listen = settings.history;
        if listen != self.listening {
            let done = unsafe { if listen { AddClipboardFormatListener(self.window) } else { RemoveClipboardFormatListener(self.window) } };
            if done.is_ok() {
                self.listening = listen;
                // (What was copied before is no copy heard now.)
                self.taken = clipboard::sequence();
            }
        }
        // Rules made tighter: what is past them goes now.
        self.keep.expire(super::now_ms());
        // (How it shows, and whether its copies do, shown as they are now.)
        self.show();
    }

    /// The clipboard changed (or was held by another as it did, and is read again): what is on it taken into Recently
    /// copied, unless it was taken already or the station put it there itself.
    fn copied(&mut self) {
        let settings = crate::app().settings.lock().unwrap().station;
        if !settings.history {
            return;
        }
        let sequence = clipboard::sequence();
        if sequence == self.taken || sequence == self.ours {
            return;
        }
        match clipboard::read(self.window, Why::Changed) {
            Ok(copied) => {
                self.taken = sequence;
                // (What its copier asked kept out, or nothing the station keeps: nothing taken.)
                if let Some(copied) = copied.filter(|copied| copied.sequence != self.ours) {
                    self.taken = copied.sequence;
                    let _ = self.keep.take(copied.content, true, copied.stays_here, super::now_ms());
                    self.show();
                }
            }
            Err(Busy) => {
                self.tries += 1;
                if self.tries <= AGAIN_MOST {
                    let _ = unsafe { SetTimer(Some(self.window), AGAIN, AGAIN_AFTER_MS, None) };
                }
            }
        }
    }

    /// What it shows now, for the other threads, who are told.
    /// `content` put on the clipboard (not to leave this PC where `stays_here`): once it took it, what it was copied
    /// from marked copied.
    /// `content` put on the clipboard (`from` what was copied, marked so): whether it was (another program may hold the
    /// clipboard).
    fn copy(&mut self, content: &Content, stays_here: bool, from: Vec<Copied>) -> bool {
        let Some(sequence) = clipboard::put(self.window, content, stays_here) else { return false };
        self.ours = sequence;
        self.copied = (!from.is_empty()).then(|| (from, std::time::Instant::now()));
        true
    }

    fn show(&self) {
        // (What a device nearby may send it, as it has room now.)
        crate::nearby::room(self.keep.shelf.room(self.keep.rules));
        let settings = crate::app().settings.lock().unwrap().station;
        let shown = Shown { items: self.keep.shelf.items.clone(), history: settings.history, view: settings.view, copies_view: settings.copies_view, recent_view: settings.recent_view, refused: self.keep.refused, over: self.over, apps_over: self.apps_over, copied: self.copied.clone(), receiving: self.receiving.clone(), arrived: self.arrived.clone() };
        *SHOWN.lock().unwrap() = Some(Arc::new(shown));
        if let Some(told) = TOLD.get() {
            told();
        }
    }
}

#[cfg(test)]
mod tests {
    /// The station drawn turns from how it was when what is kept at hand is in another order, or something else
    /// begins to come from a device nearby (or it has come); what is copied meanwhile, and how much has come, are not
    /// turned for.
    #[test]
    fn the_station_turns_as_what_it_keeps_moves() {
        use super::Shown;
        use crate::station::shelf::{Item, What};
        let item = |id: u64, copied: bool| Item { id, what: What::Text { text: id.to_string() }, added_ms: 0, pinned: false, copied, stays_here: false, own_bytes: 0, from: None };
        let sender = crate::nearby::Sender { alias: "Z Fold".into(), model: None, kind: Some("mobile".into()), fingerprint: "AB".into(), verified: true, address: "10.0.0.5".parse().unwrap() };
        let receiving = |session: &str, done: u64| crate::nearby::Receiving { session: session.into(), from: sender.clone(), files: Vec::new(), done, of: 100 };
        let shown = |items: Vec<Item>, coming: Option<crate::nearby::Receiving>| Shown { items, receiving: coming, ..Default::default() };
        let was = shown(vec![item(1, false), item(2, false)], None);
        assert!(!shown(vec![item(1, false), item(2, false)], None).moved_from(&was));
        assert!(!shown(vec![item(1, false), item(9, true), item(2, false)], None).moved_from(&was));
        assert!(shown(vec![item(2, false), item(1, false)], None).moved_from(&was));
        assert!(shown(vec![item(1, false)], None).moved_from(&was));
        let coming = shown(vec![item(1, false), item(2, false)], Some(receiving("s1", 10)));
        assert!(coming.moved_from(&was));
        assert!(!shown(vec![item(1, false), item(2, false)], Some(receiving("s1", 60))).moved_from(&coming));
        assert!(shown(vec![item(1, false), item(2, false)], Some(receiving("s2", 0))).moved_from(&coming));
        assert!(shown(vec![item(3, false), item(1, false), item(2, false)], None).moved_from(&coming));
    }

    /// Words copied lately, opened: a text file of them, named by their first line, that holds them whole (marked
    /// UTF-8, as Notepad reads Chinese right); the same words twice, the same file.
    #[test]
    fn words_open_as_a_text_file() {
        let text = "先看一下安装脚本的用法\nRan 1 shell command";
        let file = super::text_file(text).expect("written");
        assert_eq!(file.file_name().unwrap().to_string_lossy(), "先看一下安装脚本的用法.txt");
        let held = std::fs::read(&file).unwrap();
        assert_eq!(&held[..3], &[0xEF, 0xBB, 0xBF]);
        assert_eq!(std::str::from_utf8(&held[3..]).unwrap(), text);
        assert_eq!(super::text_file(text).unwrap(), file);
        // Written in and saved, then opened again: what was saved is kept, the words open in a file beside it.
        std::fs::write(&file, "改过的").unwrap();
        let again = super::text_file(text).unwrap();
        assert_eq!(again.file_name().unwrap().to_string_lossy(), "先看一下安装脚本的用法 (2).txt");
        assert_eq!(std::str::from_utf8(&std::fs::read(&again).unwrap()[3..]).unwrap(), text);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "改过的");
        assert_eq!(super::text_file(text).unwrap(), again);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// A drag off the station under way is left as it is by the station seeing to its messages meanwhile (the drag's
    /// own loop passes them on), nothing asked to be dragged off it: it is not said over.
    #[test]
    fn a_drag_under_way_goes_on_through_the_stations_messages() {
        let drag = crate::station::drag::announce();
        super::drag_off();
        let going = crate::station::drag::going();
        assert_eq!((going.drag, going.ended), (drag, None));
        crate::station::drag::ended(crate::station::drag::Dragged::Not);
    }
}
