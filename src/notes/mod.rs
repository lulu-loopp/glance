//! Notes: thoughts, to-dos and things to remember, a Markdown file each in a folder of the user's:
//! `Documents\Glance 便签` unless another is chosen. Listed here as the folder
//! holds them, read again as it changes (by Glance, or by another program, or by a sync); written by the page they
//! are edited on (see `ui::beside`), as `store` says.

pub mod edit;
pub mod html;
pub mod table;
pub mod markdown;
pub mod store;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub use store::{Colour, Folder, Note, Written};

/// How often the folder is looked for while neither it nor any folder above it is there (its drive gone): nothing
/// to be told of by then.
const LOOKED_FOR_EVERY: Duration = Duration::from_secs(5);

static SHOWN: Mutex<Option<Arc<Vec<Note>>>> = Mutex::new(None);
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// What the thread that follows the folder waits on besides the folder: set, it reads it again (see `read_again`).
static AGAIN: OnceLock<Event> = OnceLock::new();

/// An event the system keeps (set by one thread, waited on by another).
struct Event(windows::Win32::Foundation::HANDLE);
// (A handle names it: set and waited on from any thread.)
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    /// One unset, set again as it is waited on.
    fn new() -> Event {
        Event(unsafe { windows::Win32::System::Threading::CreateEventW(None, false, false, None) }.expect("an event"))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

fn again() -> &'static Event {
    AGAIN.get_or_init(Event::new)
}

/// A note's link followed, `to` where it goes, in the note's folder `folder`: a web address (or an e-mail's) in what
/// opens it; else a file of the PC's (where it is, or from the folder), opened as Explorer opens it, where it is there
/// and not of a kind Windows holds unsafe to open (a program, a script): those, and anything else, not followed. On a
/// thread of its own (the shell may take a while).
pub fn follow_link(to: &str, folder: &std::path::Path) {
    let lower = to.to_ascii_lowercase();
    let target = if ["http://", "https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme)) {
        to.to_string()
    } else {
        let path = folder.join(decoded(to));
        let safe = path.exists() && !unsafe { windows::Win32::UI::Shell::AssocIsDangerous(&windows::core::HSTRING::from(path.as_os_str())) }.as_bool();
        if !safe {
            crate::journal::note(format!("notes: a link not followed, to {to}"));
            return;
        }
        path.to_string_lossy().into_owned()
    };
    let _ = std::thread::Builder::new().name("glance: link".into()).spawn(move || unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), &windows::core::HSTRING::from(target.as_str()), None, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
        windows::Win32::System::Com::CoUninitialize();
    });
}

/// A link's path as Markdown writes one (`%20` a space): its bytes written as `%` and two hex digits as they are.
fn decoded(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[at], bytes.get(at + 1).copied().and_then(hex), bytes.get(at + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, ..) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(any(test, feature = "studio"))]
thread_local! {
    /// What this thread's pictures pretend (see `pretend`): each test's its own, as tests run side by side.
    static PRETENDED: std::cell::RefCell<Option<Arc<Vec<Note>>>> = const { std::cell::RefCell::new(None) };
}

/// The notes, the last written first (none till first read).
pub fn shown() -> Arc<Vec<Note>> {
    #[cfg(any(test, feature = "studio"))]
    if let Some(pretended) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

static NUMBERS: crate::numbered::Numbered = crate::numbered::Numbered::new();

/// The number a press on the note `name` names it by (see `numbered`).
pub fn number_of(name: &str) -> u64 {
    NUMBERS.of(name)
}

/// The name of the note shown that `number` stands for (see `number_of`): none where it is gone.
pub fn named(number: u64) -> Option<String> {
    shown().iter().find(|note| number_of(&note.name) == number).map(|note| note.name.clone())
}

/// For pictures: as if the folder held these.
#[cfg(any(test, feature = "studio"))]
pub fn pretend(notes: Vec<Note>) {
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some(Arc::new(notes)));
}

/// The notes' folder as the settings have it (none chosen yet: the one in the language shown, kept as chosen once the
/// notes are first read, so it stays as the language changes), and what a conflict's copy is called in it.
pub fn folder() -> Folder {
    let chosen = crate::app().settings.lock().unwrap().notes_folder.clone();
    folder_at(chosen.unwrap_or_else(|| default_folder(crate::update::language())))
}

/// The notes' folder at `path`, a conflict's copy called as the language shown says.
pub fn folder_at(path: PathBuf) -> Folder {
    Folder { path, apart: crate::update::language().pick(APART[0], APART[1]) }
}

/// What a conflict's copy of a note is called after its name, in each language (see `store::Written::Apart`).
const APART: [&str; 2] = ["（冲突副本）", " (conflicted copy)"];

/// Whether the note `name` is a conflict's copy (named so in either language, numbered or not).
pub fn is_apart(name: &str) -> bool {
    let stem = std::path::Path::new(name).file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    // (A second copy of the same note is numbered after its name: "（冲突副本） 2".)
    let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
    APART.iter().any(|apart| stem.ends_with(apart.trim_start()))
}

/// `Documents\Glance 便签` (`Glance Notes` in English).
#[cfg(windows)]
fn default_folder(lang: crate::ui::text::Lang) -> PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let documents = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok().and_then(|found| {
            let path = found.to_string().ok().map(PathBuf::from);
            windows::Win32::System::Com::CoTaskMemFree(Some(found.0 as *const _));
            path
        })
    };
    documents.unwrap_or_default().join(lang.pick("Glance 便签", "Glance Notes"))
}

/// How the notes show, as the settings keep it (read as the panel is drawn: kept beside the notes, not locked for).
pub fn view() -> crate::settings::NotesView {
    *VIEW.lock().unwrap()
}

/// Whether notes show their Markdown as written (see `settings::Settings::notes_source`), as the settings keep it.
pub fn source() -> bool {
    SOURCE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Notes shown with their Markdown as written, or as it reads, at once (kept in the settings by the one who asks).
pub fn show_source(source: bool) {
    SOURCE.store(source, std::sync::atomic::Ordering::Relaxed);
}

static SOURCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How note `text` shows: as it reads, or, its source asked for, as written (nothing put away).
pub fn shown_as(text: &str) -> markdown::Shown {
    match source() {
        true => markdown::Shown::default(),
        false => markdown::Shown::of(text),
    }
}

/// How the notes show, taken from the settings as they change; the folder they are in followed as it changes.
pub fn follow_settings(settings: &crate::settings::Settings) {
    *VIEW.lock().unwrap() = settings.notes_view;
    show_source(settings.notes_source);
    let mut chosen = CHOSEN.lock().unwrap();
    if *chosen != settings.notes_folder {
        chosen.clone_from(&settings.notes_folder);
        read_again();
    }
}

/// The notes' folder as the settings last had it (see `follow_settings`).
static CHOSEN: Mutex<Option<PathBuf>> = Mutex::new(None);

static VIEW: Mutex<crate::settings::NotesView> = Mutex::new(crate::settings::NotesView::Deck);

/// The folder read again at once (written here, or another chosen: what shows of it follows at once).
pub fn read_again() {
    unsafe {
        let _ = windows::Win32::System::Threading::SetEvent(again().0);
    }
}

/// Starts following the folder on a thread of its own; `told` is called whenever the notes changed.
pub fn start(told: impl Fn() + Send + Sync + 'static) {
    if TOLD.set(Box::new(told)).is_err() {
        return;
    }
    std::thread::Builder::new().name("glance: notes".into()).spawn(follow).expect("a thread started");
}

fn follow() {
    // The folder in the language shown kept as chosen, where none is yet.
    if crate::app().settings.lock().unwrap().notes_folder.is_none() {
        let path = default_folder(crate::update::language());
        crate::app().change(move |settings| {
            if settings.notes_folder.is_none() {
                settings.notes_folder = Some(path);
            }
        });
    }
    // What the folder held as last read: each note's name and file.
    let mut held: Vec<(String, store::Stamp, Colour)> = Vec::new();
    // The folder followed (or, while it is not there, the nearest above it that is, for it to come).
    let mut watched: Option<Watch> = None;
    loop {
        let folder = folder();
        // Followed before it is read: what changes while it is read is told after.
        // (The same folder: one renamed away and another made in its place is not, nor followed as it.)
        let nearest = folder.path.ancestors().find(|path| path.is_dir()).map(std::path::Path::to_path_buf);
        if !watched.as_ref().is_some_and(|watch| nearest.as_deref().is_some_and(|nearest| watch.follows(nearest))) {
            watched = nearest.and_then(|path| Watch::start(path, &folder.path));
        }
        let notes = folder.notes();
        let now: Vec<(String, store::Stamp, Colour)> = notes.iter().map(|note| (note.name.clone(), note.stamp, note.colour)).collect();
        if now != held || SHOWN.lock().unwrap().is_none() {
            held = now;
            *SHOWN.lock().unwrap() = Some(Arc::new(notes));
            if let Some(told) = TOLD.get() {
                told();
            }
        }
        // Read again as the folder changes (by Glance, another program, a sync), or as Glance asks.
        if let Some(watch) = &watched {
            if !watch.wait(again()) {
                watched = None;
            }
        } else {
            unsafe {
                let _ = windows::Win32::System::Threading::WaitForSingleObject(again().0, LOOKED_FOR_EVERY.as_millis() as u32);
            }
        }
    }
}

/// What the system tells of a folder's changes: the notes' folder's (a note come, gone, renamed or written), or one
/// above it while it is not there (a folder come in it, which may be it, or on the way to it). The notes' folder
/// followed, the folders in the one above it are followed too: the system tells of what changes in a folder, not of
/// the folder itself (renamed, or gone).
struct Watch {
    path: PathBuf,
    /// Which folder it is (see `elevation::folder_id`).
    id: Option<(u32, u64)>,
    changed: windows::Win32::Foundation::HANDLE,
    above: Option<windows::Win32::Foundation::HANDLE>,
}

impl Watch {
    /// `path` followed, as the notes' folder `notes` (or, another, as one above it).
    fn start(path: PathBuf, notes: &std::path::Path) -> Option<Watch> {
        use windows::Win32::Storage::FileSystem::{FindFirstChangeNotificationW, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE};
        let what = if path == notes { FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_DIR_NAME } else { FILE_NOTIFY_CHANGE_DIR_NAME };
        let id = crate::elevation::folder_id(&path);
        let wide = windows::core::HSTRING::from(path.as_os_str());
        let changed = unsafe { FindFirstChangeNotificationW(&wide, false, what) }.ok()?;
        let above = path.parent().filter(|_| path == notes).and_then(|above| unsafe { FindFirstChangeNotificationW(&windows::core::HSTRING::from(above.as_os_str()), false, FILE_NOTIFY_CHANGE_DIR_NAME) }.ok());
        Some(Watch { path, id, changed, above })
    }

    /// Whether it follows the folder at `path` now: that folder, not one renamed away with another made in its place.
    fn follows(&self, path: &std::path::Path) -> bool {
        self.path == path && self.id.is_some() && crate::elevation::folder_id(path) == self.id
    }

    /// Till the folder changes, or `asked` is set (Glance asks for it to be read again: see `again`); whether it is still
    /// followed (followed again before it is read, see `follow`: one gone, or renamed and another made in its place, is
    /// not).
    fn wait(&self, asked: &Event) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
        let mut handles = vec![asked.0, self.changed];
        handles.extend(self.above);
        let came = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        // (Asked again: followed as it was. A folder changed: followed on as long as it can be.)
        match came.0.checked_sub(WAIT_OBJECT_0.0).map(|at| at as usize) {
            Some(at) if (1..handles.len()).contains(&at) => unsafe { windows::Win32::Storage::FileSystem::FindNextChangeNotification(handles[at]) }.is_ok(),
            _ => true,
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        for changed in std::iter::once(self.changed).chain(self.above) {
            let _ = unsafe { windows::Win32::Storage::FileSystem::FindCloseChangeNotification(changed) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note written into the folder followed, and a folder come where none was (the notes' folder made, followed
    /// meanwhile through the one above it), are told of as they come, not looked for every so often (the folder was
    /// read whole every second, its thread woken twenty times a second).
    #[test]
    fn the_folder_tells_of_what_changes_in_it() {
        let root = std::env::temp_dir().join(format!("glance-notes-watch-{}", std::process::id()));
        let notes = root.join("Glance Notes");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Waited on as Glance's notes' own thread waits, asked only by this test: Glance's notes asked to be read again
        // meanwhile (as other tests do, all at once) tells nothing of this folder.
        let asked = Event::new();
        let told_within = |watch: &Watch, change: Box<dyn FnOnce() + Send>| {
            let changing = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                change();
            });
            read_again();
            let began = std::time::Instant::now();
            assert!(watch.wait(&asked), "followed still");
            // (Timed as the wait ends, not as the change is over.)
            let told = began.elapsed();
            changing.join().unwrap();
            told
        };
        // Not there: the folder above it followed, for it to come.
        let above = Watch::start(root.clone(), &notes).expect("followed");
        let made = told_within(&above, Box::new({
            let notes = notes.clone();
            move || std::fs::create_dir(&notes).unwrap()
        }));
        // Come: followed itself, a note written into it told of.
        let watch = Watch::start(notes.clone(), &notes).expect("followed");
        let written = told_within(&watch, Box::new({
            let notes = notes.clone();
            move || std::fs::write(notes.join("a.md"), "a note").unwrap()
        }));
        assert!(watch.follows(&notes));
        // Renamed away and another made in its place: told of, and not taken for it (followed afresh, see `follow`).
        told_within(&watch, Box::new({
            let (notes, old) = (notes.clone(), root.join("Glance Notes before"));
            move || {
                std::fs::rename(&notes, &old).unwrap();
                std::fs::create_dir(&notes).unwrap();
            }
        }));
        assert!(!watch.follows(&notes));
        assert!(Watch::start(notes.clone(), &notes).unwrap().follows(&notes));
        drop((above, watch));
        let _ = std::fs::remove_dir_all(&root);
        assert!(made >= Duration::from_millis(100) && made < Duration::from_secs(2), "{made:?}");
        assert!(written >= Duration::from_millis(100) && written < Duration::from_secs(2), "{written:?}");
    }

    /// The notes' folder followed, renamed (by another program, a sync) is told of as it is: the system tells of
    /// what changes in a folder followed, never of the folder itself, and one renamed was followed on under its new
    /// name, its old notes shown, and one made in its place never read.
    #[test]
    fn the_folder_renamed_is_told_of() {
        let root = std::env::temp_dir().join(format!("glance-notes-renamed-{}", std::process::id()));
        let notes = root.join("Glance Notes");
        let elsewhere = root.join("Old Notes");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&notes).unwrap();
        let watch = Watch::start(notes.clone(), &notes).expect("followed");
        let changing = std::thread::spawn({
            let (notes, elsewhere) = (notes.clone(), elsewhere.clone());
            move || {
                std::thread::sleep(Duration::from_millis(150));
                std::fs::rename(&notes, &elsewhere).unwrap();
                // (Not told of: a note written where it went, which is, ends the wait.)
                std::thread::sleep(Duration::from_secs(2));
                std::fs::write(elsewhere.join("a.md"), "a note").unwrap();
            }
        });
        // (Asked only by this test, as above: Glance's notes asked to be read again meanwhile tell nothing of it.)
        let asked = Event::new();
        read_again();
        let began = std::time::Instant::now();
        watch.wait(&asked);
        let told = began.elapsed();
        changing.join().unwrap();
        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
        assert!(told >= Duration::from_millis(100) && told < Duration::from_secs(2), "{told:?}");
    }

    /// A conflict's copy known by its name, in either language, a second one numbered; a note merely about conflicts
    /// not.
    #[test]
    fn knows_a_conflicts_copy() {
        assert!(is_apart("周会（冲突副本）.md"));
        assert!(is_apart("周会（冲突副本） 2.md"));
        assert!(is_apart("Meeting notes (conflicted copy).md"));
        assert!(is_apart("Meeting notes (conflicted copy) 3.md"));
        assert!(!is_apart("Meeting notes.md"));
        assert!(!is_apart("conflicted copy rules.md"));
    }
}
