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
    /// What `id` holds opened as Explorer opens it (a web address in the browser); shown in Explorer, selected there.
    Open(u64),
    Show(u64),
    /// The settings changed (see `settings::StationSettings`).
    Settings,
    /// Something dragged is over a station now, or no longer.
    Over(bool),
    AppsOver(bool),
    /// What `id` holds dragged off it (pressed on it and moved: see `drag_begins`), pressed on a window of the
    /// thread numbered so.
    DragOut(u64, u32),
    /// Files not on it (Recent files'): put on the clipboard, opened, shown in Explorer; dragged off (pressed on a
    /// window of the thread numbered so).
    Files(Vec<std::path::PathBuf>, FilesAct),
    DragFiles(Vec<std::path::PathBuf>, u32),
    /// Text put on the clipboard (a file's path), and what it was copied from, to be marked copied.
    CopyText(String, Option<Copied>),
    /// What is kept at hand opened, as the shell opens it (see `apps`).
    Launch(String),
}

/// What is done with files not on the station (see `Ask::Files`): put on the clipboard (copied from the files come
/// lately of the entry, to be marked copied there, where they were), opened, shown in Explorer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilesAct {
    Copy(Option<super::recent::Entry>),
    Open,
    Show,
}

/// What was put on the clipboard from where it shows (marked copied there a moment: see `Shown::copied`): a thing on
/// the station or copied lately, by its number, or files come lately, by their entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Copied {
    Item(u64),
    Recent(super::recent::Entry),
}

/// How long what was copied is marked so (see `Shown::copied`).
pub const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1400);

/// What is to be dragged off the station: a thing on it, by its number, or files not on it.
enum Leaving {
    Item(u64),
    Files(Vec<std::path::PathBuf>),
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
    /// What was last put on the clipboard from where it shows, and when (the clipboard took it): marked copied there
    /// for `COPIED_FOR`.
    pub copied: Option<(Copied, std::time::Instant)>,
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

/// Where what is dropped goes: onto the station, or into what is kept at hand (see `apps`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lands {
    Station,
    Apps,
}

/// What a window catching drops saw (see `catch`; on the station's thread, perhaps as it does something already: so
/// told it by message): a drag over a strip along `edge` opens the station there, as a push into the edge does.
pub fn caught(dragged: Dragged, edge: Option<crate::settings::Edge>, lands: Lands) {
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
    #[cfg(any(test, feature = "studio"))]
    if let Some(pretended) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

/// For pictures: as if the station held `items`, shown as `view` (its thread not started).
#[cfg(any(test, feature = "studio"))]
pub fn pretend(items: Vec<Item>, view: crate::settings::ShelfView) {
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some(Arc::new(Shown { items, history: true, view, copies_view: view, recent_view: view, refused: None, over: false, apps_over: false, copied: None })));
}

/// For pictures: what is pretended (see `pretend`) with Recently copied shown as `copies` and Recent files as `recent`.
#[cfg(any(test, feature = "studio"))]
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
    // (Its graphics let go of before the thread ends: see `gfx::held`.)
    let Ok(_held) = crate::ui::gfx::held() else { return };
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
    copied: Option<(Copied, std::time::Instant)>,
    /// Where drops are caught while something is dragged (see `catch`).
    catchers: super::catch::Catchers,
    /// What is to be dragged off it, asked, and the thread it was pressed on (dragged once it is done answering: see
    /// `drag_off`).
    dragging: Option<(Leaving, u32)>,
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
        let mut keep = Keep::open(Folder::own(), settings.rules(), settings.always_copy);
        keep.expire(super::now_ms());
        STATION.with_borrow_mut(|station| *station = Some(Station { keep, window, asks, listening: false, taken: 0, ours: 0, tries: 0, over: false, apps_over: false, copied: None, catchers: Default::default(), dragging: None }));
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

/// What was asked to be dragged off the station, dragged (outside `with_station`: the drag's own loop passes the
/// station's messages on meanwhile, which find it free); its own files held meanwhile, and taken off after, where the
/// settings say so. Called after each of the station's messages (those the drag's own loop passes on among them):
/// nothing asked, nothing done.
fn drag_off() {
    let mut given = None;
    let mut thread = 0;
    with_station(|station| {
        given = station.dragging.take().map(|(dragged, pressed_on)| {
            thread = pressed_on;
            match dragged {
                Leaving::Item(id) => station.drag_given(id).map(|(id, content, stays_here, held)| (Some(id), content, stays_here, held)),
                // (Files not its own: given where they are, never taken off anything.)
                Leaving::Files(paths) => Some((None, Content::Files(paths), false, Vec::new())),
            }
        })
    });
    // (Asked, and not given: the drag asked for is over, as given up; one under way, or none, is left as it is.)
    let Some(given) = given else { return };
    let Some((id, content, stays_here, held)) = given else {
        super::drag::ended(super::drag::Dragged::Not);
        return;
    };
    let dragged = super::drag::drag(&content, stays_here, thread);
    with_station(|station| {
        for name in &held {
            station.keep.folder.let_go(name);
        }
        let remove = crate::app().settings.lock().unwrap().station.remove_after_drag;
        if let (true, Some(id), Some(super::drag::Dragged::Taken)) = (remove, id, dragged) {
            station.keep.remove(id);
        }
        station.show();
    });
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
                Ask::Copy(id) => {
                    let given = self.keep.shelf.get(id).and_then(|item| Some((self.keep.content_of(item)?, item.stays_here)));
                    if let Some((content, stays_here)) = given {
                        self.copy(&content, stays_here, Some(Copied::Item(id)));
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
                Ask::DragOut(id, thread) => self.dragging = Some((Leaving::Item(id), thread)),
                Ask::DragFiles(paths, thread) => self.dragging = Some((Leaving::Files(paths), thread)),
                Ask::Files(paths, act) => match act {
                    FilesAct::Copy(entry) => self.copy(&Content::Files(paths), false, entry.map(Copied::Recent)),
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
                Ask::CopyText(text, from) => self.copy(&Content::Text(text), false, from),
            }
        }
        self.show();
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

    /// What a drag off it of `id` gives (see `drag_off`): its number, what it gives, whether it stays on this PC, and
    /// the files of its own it holds meanwhile.
    fn drag_given(&mut self, id: u64) -> Option<(u64, Content, bool, Vec<String>)> {
        let item = self.keep.shelf.get(id)?.clone();
        let content = self.opened(id)?;
        let held: Vec<String> = item.what.own_files().into_iter().map(str::to_string).collect();
        for name in &held {
            self.keep.folder.hold(name);
        }
        Some((id, content, item.stays_here, held))
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
    fn copy(&mut self, content: &Content, stays_here: bool, from: Option<Copied>) {
        if let Some(sequence) = clipboard::put(self.window, content, stays_here) {
            self.ours = sequence;
            self.copied = from.map(|from| (from, std::time::Instant::now()));
        }
    }

    fn show(&self) {
        let settings = crate::app().settings.lock().unwrap().station;
        let shown = Shown { items: self.keep.shelf.items.clone(), history: settings.history, view: settings.view, copies_view: settings.copies_view, recent_view: settings.recent_view, refused: self.keep.refused, over: self.over, apps_over: self.apps_over, copied: self.copied };
        *SHOWN.lock().unwrap() = Some(Arc::new(shown));
        if let Some(told) = TOLD.get() {
            told();
        }
    }
}

#[cfg(test)]
mod tests {
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
