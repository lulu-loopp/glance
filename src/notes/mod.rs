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
use windows::Win32::Devices::DeviceAndDriverInstallation::HCMNOTIFICATION;

pub use store::{Colour, Folder, Note, Written};

/// How often the folder is looked for while neither it nor any folder above it is there (its drive gone): nothing
/// to be told of by then.
const LOOKED_FOR_EVERY: Duration = Duration::from_secs(5);

static SHOWN: Mutex<Option<Arc<Vec<Note>>>> = Mutex::new(None);
/// The same, but the to-do note (see `listed`).
static LISTED: Mutex<Option<Arc<Vec<Note>>>> = Mutex::new(None);
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// What the thread that follows the folder waits on besides the folder: set, it reads it again (see `read_again`).
static AGAIN: OnceLock<Event> = OnceLock::new();

/// An event the system keeps (set by one thread, waited on by another).
struct Event(windows::Win32::Foundation::HANDLE);
// (A handle names it: set and waited on from any thread.)
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

fn again() -> &'static Event {
    AGAIN.get_or_init(|| Event(unsafe { windows::Win32::System::Threading::CreateEventW(None, false, false, None) }.expect("an event")))
}

/// A note's link followed, `to` where it goes, in the note's folder `folder`: a web address (or an e-mail's) in what
/// opens it; else a file of the PC's (where it is, or from the folder), opened as Explorer opens it, where it is there
/// and not of a kind Windows holds unsafe to open (a program, a script): those, and anything else, not followed. On a
/// thread of its own (the shell may take a while, and a file's drive be slow to say it is there).
pub fn follow_link(to: &str, folder: &std::path::Path) {
    let lower = to.to_ascii_lowercase();
    let web = ["http://", "https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme));
    let target = if web { to.to_string() } else { folder.join(decoded(to)).to_string_lossy().into_owned() };
    let to = to.to_string();
    let _ = std::thread::Builder::new().name("glance: link".into()).spawn(move || unsafe {
        let path = windows::core::HSTRING::from(target.as_str());
        if !web && (!std::path::Path::new(&target).exists() || windows::Win32::UI::Shell::AssocIsDangerous(&path).as_bool()) {
            crate::journal::note(format!("notes: a link not followed, to {to}"));
            return;
        }
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), &path, None, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
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
    static PRETENDED: std::cell::RefCell<Option<(Arc<Vec<Note>>, Arc<Vec<Note>>)>> = const { std::cell::RefCell::new(None) };
}

/// The notes, the last written first (none till first read).
pub fn shown() -> Arc<Vec<Note>> {
    #[cfg(any(test, feature = "studio"))]
    if let Some((pretended, _)) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

/// The notes listed as notes: all but the to-do note, which is the to-dos' (see `todo`).
pub fn listed() -> Arc<Vec<Note>> {
    #[cfg(any(test, feature = "studio"))]
    if let Some((_, pretended)) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    LISTED.lock().unwrap().clone().unwrap_or_default()
}

/// `notes` but the to-do note.
fn but_todo(notes: &[Note]) -> Vec<Note> {
    let todo = crate::todo::note_of(notes).map(|note| note.name.clone());
    notes.iter().filter(|note| Some(&note.name) != todo.as_ref()).cloned().collect()
}

static NUMBERS: crate::numbered::Numbered = crate::numbered::Numbered::new();

/// The number a press on the note `name` names it by (see `numbered`).
pub fn number_of(name: &str) -> u64 {
    NUMBERS.of(name)
}

/// The name of the note listed that `number` stands for (see `number_of`): none where it is gone.
pub fn named(number: u64) -> Option<String> {
    listed().iter().find(|note| number_of(&note.name) == number).map(|note| note.name.clone())
}

/// For pictures: as if the folder held these.
#[cfg(any(test, feature = "studio"))]
pub fn pretend(notes: Vec<Note>) {
    let listed = Arc::new(but_todo(&notes));
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some((Arc::new(notes), listed)));
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
            *LISTED.lock().unwrap() = Some(Arc::new(but_todo(&notes)));
            *SHOWN.lock().unwrap() = Some(Arc::new(notes));
            if let Some(told) = TOLD.get() {
                told();
            }
        }
        // Read again as the folder changes (by Glance, another program, a sync), or as Glance asks.
        if let Some(watch) = &mut watched {
            if !watch.wait() {
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
/// the folder itself (renamed, or gone). Its drive asked to be taken out (a USB drive, see `removal`), all it holds
/// of it is let go of, for Windows to take it out.
struct Watch {
    path: PathBuf,
    /// Which folder it is (see `elevation::folder_id`).
    id: Option<(u32, u64)>,
    /// How its drive being taken out is heard (none where it cannot be), and what it holds of the folder, and what was
    /// heard (see `Leaving`).
    heard: Option<HCMNOTIFICATION>,
    leaving: Box<Leaving>,
}
// (Its hearing named by a handle: let go of from any thread.)
unsafe impl Send for Watch {}

/// What a folder followed holds of it: what tells of its changes (and of the one above it), and the folder held open
/// to hear through as its drive is asked to be taken out.
struct Holding {
    changed: windows::Win32::Foundation::HANDLE,
    above: Option<windows::Win32::Foundation::HANDLE>,
    folder: Option<windows::Win32::Foundation::HANDLE>,
}

// (Handles name what it holds: waited on and let go of from any thread.)
unsafe impl Send for Holding {}

impl Holding {
    fn close(self) {
        for changed in std::iter::once(self.changed).chain(self.above) {
            let _ = unsafe { windows::Win32::Storage::FileSystem::FindCloseChangeNotification(changed) };
        }
        if let Some(folder) = self.folder {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(folder) };
        }
    }
}

/// How long Windows asking to take a followed folder's drive out waits, at most, for the thread following it to let
/// go of it as it waits on it (see `Watch::wait`): woken, it lets go at once.
const LET_GO_WAIT: Duration = Duration::from_secs(5);

/// A followed folder's drive being taken out (see `removal`): what the thread following it waits on besides the
/// folder (set, it lets go), and how things are: what it holds (none once let go of), whether the thread waits on it
/// now, and how far the taking out went.
struct Leaving {
    asked: Event,
    now: Mutex<Now>,
    moved: std::sync::Condvar,
}

struct Now {
    holding: Option<Holding>,
    waiting: bool,
    stage: Stage,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Held,
    /// Asked to be let go of (its drive to be taken out), and let go of.
    Asked,
    LetGo,
    /// Not taken out after all (another program held it): followed again.
    Stays,
    /// Taken out, or about to be.
    Gone,
}

impl Now {
    /// What it holds let go of, and told so where it was asked.
    fn let_go(&mut self) {
        if let Some(holding) = self.holding.take() {
            holding.close();
        }
        if self.stage == Stage::Asked {
            self.stage = Stage::LetGo;
        }
    }
}

impl Leaving {
    /// Asked to let go: let go of here, where the thread following it does not wait on what it holds (it reads the
    /// notes); else that thread woken to let go, and waited for (see `LET_GO_WAIT`).
    fn ask(&self) {
        let mut now = self.now.lock().unwrap();
        now.stage = Stage::Asked;
        if !now.waiting {
            now.let_go();
            self.moved.notify_all();
            return;
        }
        unsafe {
            let _ = windows::Win32::System::Threading::SetEvent(self.asked.0);
        }
        drop(self.moved.wait_timeout_while(now, LET_GO_WAIT, |now| now.stage == Stage::Asked).unwrap());
    }

    /// Told it went `to` that.
    fn to(&self, to: Stage) {
        self.now.lock().unwrap().stage = to;
        self.moved.notify_all();
    }
}

/// Windows telling of a followed folder's drive (`context` its `Leaving`): asked to take it out, all held of it is let
/// go of (Windows waits for that, then takes it out, see `Leaving::ask`); the asking failed (another program held it),
/// it is followed again; taken out (asked first or not), it is let go of likewise, and the folder looked for till it
/// comes again.
unsafe extern "system" fn removal(
    _: HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_ACTION,
    _: *const windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_EVENT_DATA,
    _: u32,
) -> u32 {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE, CM_NOTIFY_ACTION_DEVICEREMOVEPENDING};
    let leaving = unsafe { &*(context as *const Leaving) };
    match action {
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => leaving.ask(),
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => leaving.to(Stage::Stays),
        CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => {
            // (Taken out unasked: let go of first.)
            let unasked = matches!(leaving.now.lock().unwrap().stage, Stage::Held | Stage::Stays);
            if unasked {
                leaving.ask();
            }
            leaving.to(Stage::Gone);
        }
        _ => {}
    }
    0
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
        let asked = Event(unsafe { windows::Win32::System::Threading::CreateEventW(None, false, false, None) }.expect("an event"));
        let holding = Holding { changed, above, folder: None };
        let leaving = Box::new(Leaving { asked, now: Mutex::new(Now { holding: Some(holding), waiting: false, stage: Stage::Held }), moved: std::sync::Condvar::new() });
        let heard = hear_removal(&wide, &leaving);
        Some(Watch { path, id, heard, leaving })
    }

    /// Whether it follows the folder at `path` now: that folder, not one renamed away with another made in its place.
    fn follows(&self, path: &std::path::Path) -> bool {
        self.path == path && self.id.is_some() && crate::elevation::folder_id(path) == self.id
    }

    /// Till the folder changes, or Glance asks for it to be read again, or its drive is asked to be taken out; whether
    /// it is still followed (followed again before it is read, see `follow`: one gone, or renamed and another made in
    /// its place, is not). Its drive asked to be taken out (now, or as the notes were read): let go of, and not
    /// followed, once Windows has taken it out or given up (it stays, to be followed again).
    fn wait(&mut self) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
        let handles = {
            let mut now = self.leaving.now.lock().unwrap();
            let Some(holding) = &now.holding else { return self.settled(now) };
            let mut handles = vec![again().0, self.leaving.asked.0, holding.changed];
            handles.extend(holding.above);
            now.waiting = true;
            handles
        };
        let came = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        let mut now = self.leaving.now.lock().unwrap();
        now.waiting = false;
        // (Asked to let go as it waited, whatever woke it.)
        if now.stage == Stage::Asked {
            now.let_go();
            self.leaving.moved.notify_all();
            return self.settled(now);
        }
        // (Asked again: followed as it was. A folder changed: followed on as long as it can be.)
        match came.0.checked_sub(WAIT_OBJECT_0.0).map(|at| at as usize) {
            Some(at) if (2..handles.len()).contains(&at) => unsafe { windows::Win32::Storage::FileSystem::FindNextChangeNotification(handles[at]) }.is_ok(),
            _ => true,
        }
    }

    /// Let go of as its drive was asked to be taken out: not followed, once Windows has taken it out or given up.
    fn settled(&self, now: std::sync::MutexGuard<Now>) -> bool {
        drop(self.leaving.moved.wait_while(now, |now| matches!(now.stage, Stage::Asked | Stage::LetGo)).unwrap());
        false
    }
}

/// The folder at `path` held open for Windows to tell `leaving` through (see `removal`) as its drive is asked to be
/// taken out (kept with what it holds), and how that is heard. None where it cannot be (a folder on the network).
fn hear_removal(path: &windows::core::HSTRING, leaving: &Leaving) -> Option<HCMNOTIFICATION> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_Register_Notification, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_1, CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    let folder = unsafe { CreateFileW(path, FILE_LIST_DIRECTORY.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) }.ok()?;
    // (Held before it is heard through: a telling that comes at once lets go of it too.)
    if let Some(holding) = leaving.now.lock().unwrap().holding.as_mut() {
        holding.folder = Some(folder);
    }
    let filter = CM_NOTIFY_FILTER {
        cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
        u: CM_NOTIFY_FILTER_0 { DeviceHandle: CM_NOTIFY_FILTER_0_1 { hTarget: folder } },
        ..Default::default()
    };
    let mut heard = HCMNOTIFICATION::default();
    let context = leaving as *const Leaving as *const core::ffi::c_void;
    if unsafe { CM_Register_Notification(&filter, Some(context), Some(removal), &mut heard) } != CR_SUCCESS {
        if let Some(folder) = leaving.now.lock().unwrap().holding.as_mut().and_then(|holding| holding.folder.take()) {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(folder) };
        }
        return None;
    }
    Some(heard)
}

impl Drop for Watch {
    fn drop(&mut self) {
        // (Let go of first: a telling under way waits for that, and unregistering waits for a telling under way.)
        self.leaving.now.lock().unwrap().let_go();
        self.leaving.moved.notify_all();
        if let Some(heard) = self.heard {
            unsafe {
                let _ = windows::Win32::Devices::DeviceAndDriverInstallation::CM_Unregister_Notification(heard);
            }
        }
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.leaving.asked.0);
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
        let told_within = |watch: &mut Watch, change: Box<dyn FnOnce() + Send>| {
            let changing = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                change();
            });
            let began = std::time::Instant::now();
            assert!(watch.wait(), "followed still");
            changing.join().unwrap();
            began.elapsed()
        };
        // Not there: the folder above it followed, for it to come.
        let mut above = Watch::start(root.clone(), &notes).expect("followed");
        let made = told_within(&mut above, Box::new({
            let notes = notes.clone();
            move || std::fs::create_dir(&notes).unwrap()
        }));
        // Come: followed itself, a note written into it told of.
        let mut watch = Watch::start(notes.clone(), &notes).expect("followed");
        let written = told_within(&mut watch, Box::new({
            let notes = notes.clone();
            move || std::fs::write(notes.join("a.md"), "a note").unwrap()
        }));
        assert!(watch.follows(&notes));
        // Renamed away and another made in its place: told of, and not taken for it (followed afresh, see `follow`).
        told_within(&mut watch, Box::new({
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
        let mut watch = Watch::start(notes.clone(), &notes).expect("followed");
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
        let began = std::time::Instant::now();
        watch.wait();
        let told = began.elapsed();
        changing.join().unwrap();
        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
        assert!(told >= Duration::from_millis(100) && told < Duration::from_secs(2), "{told:?}");
    }

    /// Windows asking to take the folder's drive out (as `removal` hears it) has all it holds of it let go of before
    /// the asking is answered, by the thread following it, which then waits: the asking failed, it is followed again;
    /// taken out, not followed. Taken out unasked (pulled out), it is let go of likewise.
    #[test]
    fn the_folder_is_let_go_of_as_its_drive_is_taken_out() {
        use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE};
        let notes = std::env::temp_dir().join(format!("glance-notes-taken-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&notes);
        std::fs::create_dir_all(&notes).unwrap();
        let tell = |leaving: usize, action: CM_NOTIFY_ACTION| unsafe { removal(HCMNOTIFICATION::default(), leaving as *const core::ffi::c_void, action, std::ptr::null(), 0) };
        // (Followed on a thread of its own, as `follow` does, till it is followed no more: whether it let go then.
        // Asked to read again meanwhile, by another test, it waits again.)
        let followed = |mut watch: Watch| {
            let (ended, end) = std::sync::mpsc::channel();
            let leaving = &*watch.leaving as *const Leaving as usize;
            std::thread::spawn(move || {
                while watch.wait() {}
                let _ = ended.send(watch.leaving.now.lock().unwrap().holding.is_none());
                drop(watch);
            });
            (leaving, end)
        };
        for (asked, then) in [(true, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED), (true, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE), (false, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE)] {
            let watch = Watch::start(notes.clone(), &notes).expect("followed");
            assert!(watch.heard.is_some(), "its drive's removal heard");
            let (leaving, end) = followed(watch);
            std::thread::sleep(Duration::from_millis(100));
            let began = std::time::Instant::now();
            if asked {
                tell(leaving, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE);
                let stage = unsafe { (*(leaving as *const Leaving)).now.lock().unwrap().stage };
                assert_eq!(stage, Stage::LetGo, "let go of as it was asked, not after waiting it out");
                assert!(began.elapsed() < LET_GO_WAIT);
                // (Not followed again till Windows has said how it went.)
                assert!(end.recv_timeout(Duration::from_millis(200)).is_err());
            }
            tell(leaving, then);
            assert!(end.recv_timeout(Duration::from_secs(2)).expect("followed no more"), "let go of");
            assert!(began.elapsed() < LET_GO_WAIT);
        }
        // Asked while the notes are read (the thread not waiting on what it holds): let go of at once, by the asking
        // itself, and the next wait not followed, once Windows has said how it went.
        let mut watch = Watch::start(notes.clone(), &notes).expect("followed");
        let leaving = &*watch.leaving as *const Leaving as usize;
        let began = std::time::Instant::now();
        tell(leaving, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE);
        assert!(began.elapsed() < Duration::from_secs(1), "not waited out");
        {
            let now = watch.leaving.now.lock().unwrap();
            assert!(now.holding.is_none() && now.stage == Stage::LetGo);
        }
        tell(leaving, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED);
        assert!(!watch.wait());
        drop(watch);
        let _ = std::fs::remove_dir_all(&notes);
    }

    /// A USB drive with the notes' folder on it, followed, can be taken out safely as Windows asks (it was refused, the
    /// folder held open):
    /// `$env:GLANCE_EJECT="G"; cargo test --lib -- --ignored a_usb_drive_with_the_notes_can_be_taken_out --nocapture`
    /// (its root followed as the notes' folder, nothing written to it; it is taken out: put it in again after).
    #[test]
    #[ignore = "takes a drive put in out"]
    fn a_usb_drive_with_the_notes_can_be_taken_out() {
        use crate::search::drive::tests::{eject, removable_device};
        let letter = std::env::var("GLANCE_EJECT").expect("GLANCE_EJECT: the drive's letter");
        let root = PathBuf::from(format!("{letter}:\\"));
        let device = removable_device(&letter).expect("a removable device under the drive");
        let mut watch = Watch::start(root.clone(), &root).expect("followed");
        assert!(watch.heard.is_some(), "its drive's removal heard");
        let (ended, end) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while watch.wait() {}
            let _ = ended.send(());
        });
        std::thread::sleep(Duration::from_millis(300));
        let (veto, why) = eject(device);
        println!("notes followed: veto {veto} ({why})");
        assert_eq!(veto, 0, "taken out");
        assert!(end.recv_timeout(Duration::from_secs(5)).is_ok(), "followed no more");
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
