//! The folders whose sizes the storage component tells (chosen in its settings, or dropped on it): how large what each
//! holds is, as search's index has it (see `search::within`; the drive is not looked through for it), counted again
//! as something in it changes while it is shown, and how that changed lately: what each was at the first counting of a
//! day is kept (in `folders.json` in Glance's own folder, on this PC alone), the days of the last week, from the day it
//! was added. What was counted last is kept too, shown as Glance starts till it is counted again.

use crate::search::{Holds, Watch};
use crate::todo::Day;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The most folders told of.
pub const MOST: usize = 12;

/// How many of the largest things in a folder are told.
const LARGEST: usize = 3;

/// How many days back its change is told over.
const WEEK: i64 = 7;

/// What a folder holds, at any depth: how large its files are together, how many there are, and what is largest of what
/// lies in it directly, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Held {
    pub bytes: u64,
    pub files: u64,
    pub largest: Vec<(String, u64)>,
}

/// A folder as it is told: its path, the name Explorer shows it by, what it holds (none: not counted yet), whether it
/// is not where it was, or on a drive search does not find names on, and how large it was on the days since it was
/// added (each by its number, see `Day::number`, the oldest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub name: String,
    pub held: Option<Held>,
    pub away: bool,
    pub days: Vec<(i64, u64)>,
}

/// How a folder's size changed (bytes, up above 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Kept from today on: nothing to tell yet.
    Begun,
    /// Since a day less than a week back (the one it was added), by its number.
    Since(i64, i64),
    Week(i64),
}

impl Folder {
    /// How its size changed by `today` (a day's number) in the last week, or since it was added.
    pub fn change(&self, today: i64) -> Change {
        let (Some(held), Some(&(first, was))) = (&self.held, self.days.first()) else { return Change::Begun };
        let by = |was: u64| held.bytes as i64 - was as i64;
        match self.days.iter().rev().find(|(day, _)| *day <= today - WEEK) {
            Some(&(_, was)) => Change::Week(by(was)),
            None if first < today => Change::Since(first, by(was)),
            None => Change::Begun,
        }
    }
}

/// The number of today.
pub fn today() -> i64 {
    Day::today().number()
}

/// `bytes` counted on `today` put among `days`, where it is the first counting of the day: the days of the last week
/// left, and the last before them, which what it changed by is told from. Whether it was put there.
fn kept_on(days: &mut Vec<(i64, u64)>, today: i64, bytes: u64) -> bool {
    // (A day still to come: the clock was set back since.)
    days.retain(|(day, _)| *day <= today);
    if days.last().is_some_and(|(day, _)| *day == today) {
        return false;
    }
    days.push((today, bytes));
    let before = days.iter().rposition(|(day, _)| *day <= today - WEEK).unwrap_or(0);
    days.drain(..before);
    true
}

/// What is kept of a folder (see the module's note).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    held: Option<Held>,
    #[serde(default)]
    days: Vec<(i64, u64)>,
}

/// What is asked of the thread counting: the folders to tell of, when they were last looked at, and whether either is
/// news to it.
struct Asked {
    list: Vec<String>,
    looked: Option<Instant>,
    news: bool,
}

static ASKED: (Mutex<Asked>, Condvar) = (Mutex::new(Asked { list: Vec::new(), looked: None, news: false }), Condvar::new());
static SHOWN: Mutex<Option<Arc<Vec<Folder>>>> = Mutex::new(None);
/// Called (on the counting thread) as what is shown changes.
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

#[cfg(any(test, feature = "studio"))]
thread_local! {
    /// What this thread's pictures pretend (see `pretend`).
    static PRETENDED: std::cell::RefCell<Option<Arc<Vec<Folder>>>> = const { std::cell::RefCell::new(None) };
}

/// For pictures: as if these were told of.
#[cfg(any(test, feature = "studio"))]
pub fn pretend(folders: Vec<Folder>) {
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some(Arc::new(folders)));
}

/// How long after they were last looked at they count as looked at.
const LOOKED_FOR: Duration = Duration::from_secs(3);

/// The folders as they are told now, looked at: counted again from now on as what they hold changes.
pub fn shown() -> Arc<Vec<Folder>> {
    #[cfg(any(test, feature = "studio"))]
    if let Some(pretended) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    let shown = SHOWN.lock().unwrap().clone().unwrap_or_default();
    if !shown.is_empty() {
        let mut asked = ASKED.0.lock().unwrap();
        let resting = asked.looked.is_none_or(|at| at.elapsed() > LOOKED_FOR);
        asked.looked = Some(Instant::now());
        if resting {
            asked.news = true;
            ASKED.1.notify_one();
        }
    }
    shown
}

/// The folder `number` stands for (see `number_of`), as it is told now.
pub fn numbered(number: u64) -> Option<Folder> {
    shown().iter().find(|folder| number_of(&folder.path) == number).cloned()
}

static NUMBERS: crate::numbered::Numbered = crate::numbered::Numbered::new();

/// The number a press on the folder at `path` names it by (see `numbered`).
pub fn number_of(path: &str) -> u64 {
    NUMBERS.of(path)
}

/// Starts telling of the folders the settings list (see `follow`); `told` is called as what is told changes.
pub fn start(list: &[String], told: impl Fn() + Send + Sync + 'static) {
    if TOLD.set(Box::new(told)).is_ok() {
        follow(list);
        std::thread::Builder::new().name("glance: folder sizes".into()).spawn(run).expect("a thread started");
    }
}

/// The folders to tell of, as the settings list them (as they are read, and each time they change).
pub fn follow(list: &[String]) {
    let mut asked = ASKED.0.lock().unwrap();
    if asked.list != list {
        asked.list = list.to_vec();
        asked.news = true;
        ASKED.1.notify_one();
    }
}

/// `paths`' folders told of from now on, after those told of (as many as `MOST`), those told of already left where they
/// are.
pub fn add(paths: Vec<PathBuf>) {
    let added: Vec<String> = paths.into_iter().filter(|path| path.is_dir()).map(|path| path.display().to_string()).collect();
    if added.is_empty() {
        return;
    }
    // (Their sizes come from search's index: made now, where it is not yet.)
    crate::search::asked();
    crate::app().change(move |settings| put(&mut settings.storage_folders, added));
}

/// `added` put after `list`'s, as many as `MOST`, none twice.
pub fn put(list: &mut Vec<String>, added: Vec<String>) {
    for path in added {
        if list.len() < MOST && !list.iter().any(|had| had.to_lowercase() == path.to_lowercase()) {
            list.push(path);
        }
    }
}

/// The folder at `path` put in `list` before the one it was after.
pub fn raise(list: &mut [String], path: &str) {
    if let Some(at @ 1..) = list.iter().position(|had| had == path) {
        list.swap(at - 1, at);
    }
}

/// The name the folder at `path` is told by (as it is told now, else its own).
pub fn called(path: &str) -> String {
    let told = SHOWN.lock().unwrap().as_ref().and_then(|shown| shown.iter().find(|folder| folder.path == path).map(|folder| folder.name.clone()));
    told.unwrap_or_else(|| own_name(path))
}

/// The folder at `path`'s own name (a drive's top, its path).
fn own_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned())
}

/// The folder at `path` opened in Explorer, on another thread (the shell may take a while).
pub fn open(path: String) {
    std::thread::spawn(move || {
        let path = windows::core::HSTRING::from(path);
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), &path, windows::core::PCWSTR::null(), windows::core::PCWSTR::null(), windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
        }
    });
}

/// The name Explorer shows the folder at `path` by (its own in the user's language, for those Windows names), else its
/// own.
fn name_of(path: &str) -> String {
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_DISPLAYNAME};
    let mut info = SHFILEINFOW::default();
    let found = unsafe { SHGetFileInfoW(&windows::core::HSTRING::from(path), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_DISPLAYNAME) };
    let units = info.szDisplayName;
    let shown = String::from_utf16_lossy(&units[..units.iter().position(|unit| *unit == 0).unwrap_or(units.len())]);
    if found != 0 && !shown.is_empty() {
        return shown;
    }
    own_name(path)
}

const FILE: &str = "folders.json";

fn read_kept(dir: &Path) -> HashMap<String, Kept> {
    std::fs::read(dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn write_kept(dir: &Path, kept: &HashMap<String, Kept>) {
    let written = std::fs::create_dir_all(dir).and_then(|()| crate::elevation::write_in_place(dir, FILE, &serde_json::to_vec(kept).expect("folders as JSON")));
    if let Err(error) = written {
        crate::journal::note(format!("folder sizes: not kept: {error}"));
    }
}

/// A folder being told of, as the thread counting it has it: how it is told, and, counted since Glance started, what
/// tells whether anything in it changed since, and when it was last tried, with how long it rests after being counted.
struct Counted {
    folder: Folder,
    watch: Option<Watch>,
    tried: Option<(Instant, Duration)>,
}

/// The shortest between two countings of a folder that changes; and how many times as long as counting it took (a
/// folder of two million files, a tenth of a second, at most a twentieth of the time).
const COUNT_EVERY: Duration = Duration::from_secs(2);
const REST: u32 = 20;
/// How long one not counted (its drive being listed still, the folder not there) waits to be tried again: looked at,
/// and else.
const TRY_EVERY: Duration = Duration::from_secs(5);
const TRY_RESTING: Duration = Duration::from_secs(60);
/// How often the day is looked at while none is looked at; and how long what changed waits to be kept, but for a day's
/// first counting.
const DAY_EVERY: Duration = Duration::from_secs(600);
const KEEP_EVERY: Duration = Duration::from_secs(600);

impl Counted {
    fn new(folder: Folder) -> Counted {
        Counted { folder, watch: None, tried: None }
    }

    /// Whether it is to be counted `now`, on `today`: not counted yet, its first counting of the day, or (`looked` at)
    /// something in it changed.
    fn due(&self, now: Instant, today: i64, looked: bool) -> bool {
        let waits = |rest| match &self.watch {
            Some(_) => rest,
            None if looked => TRY_EVERY,
            None => TRY_RESTING,
        };
        if self.tried.is_some_and(|(at, rest)| now < at + waits(rest)) {
            return false;
        }
        match &self.watch {
            None => true,
            Some(watch) => self.folder.days.last().is_none_or(|(day, _)| *day < today) || (looked && crate::search::changed(watch)),
        }
    }

    /// Counted. Whether what is told of it changed, and whether its day's first counting was kept.
    fn count(&mut self, today: i64) -> (bool, bool) {
        let begun = Instant::now();
        let holds = crate::search::within(Path::new(&self.folder.path), LARGEST);
        let took = begun.elapsed();
        let was = self.folder.clone();
        let mut day = false;
        self.tried = Some((begun, (took * REST).max(COUNT_EVERY)));
        match holds {
            Holds::Told { bytes, files, largest, watch } => {
                self.folder.held = Some(Held { bytes, files, largest });
                self.folder.away = false;
                day = kept_on(&mut self.folder.days, today, bytes);
                self.watch = Some(watch);
            }
            Holds::Listing => {
                self.folder.away = false;
                self.watch = None;
            }
            Holds::Nowhere => {
                self.folder.away = true;
                self.watch = None;
            }
        }
        (self.folder != was, day)
    }
}

fn run() {
    crate::settle::background("folder sizes");
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
    let dir = crate::settings::local_dir();
    let mut kept = read_kept(&dir);
    let mut kept_at = Instant::now();
    let mut folders: Vec<Counted> = Vec::new();
    loop {
        let (list, looked) = {
            let mut asked = ASKED.0.lock().unwrap();
            asked.news = false;
            (asked.list.clone(), asked.looked.is_some_and(|at| at.elapsed() <= LOOKED_FOR))
        };
        // Those told of now, in their order, what is known of each kept: told at once, as they were counted last.
        let same = folders.len() == list.len() && folders.iter().zip(&list).all(|(had, path)| had.folder.path == *path);
        let (mut news, mut keep) = (!same, false);
        if !same {
            folders = list
                .iter()
                .map(|path| match folders.iter().position(|had| had.folder.path == *path) {
                    Some(at) => folders.swap_remove(at),
                    None => {
                        let Kept { held, days } = kept.get(path).cloned().unwrap_or_default();
                        Counted::new(Folder { name: name_of(path), path: path.clone(), held, away: false, days })
                    }
                })
                .collect();
            // (One told of no more is not kept: added again, it is told of from then on.)
            let before = kept.len();
            kept.retain(|path, _| list.contains(path));
            keep = kept.len() != before;
            publish(&folders);
        }
        let today = today();
        for counted in &mut folders {
            if counted.due(Instant::now(), today, looked) {
                let (changed, day) = counted.count(today);
                news |= changed;
                keep |= day;
            }
        }
        if news {
            publish(&folders);
            for counted in &folders {
                kept.insert(counted.folder.path.clone(), Kept { held: counted.folder.held.clone(), days: counted.folder.days.clone() });
            }
            if let Some(told) = TOLD.get() {
                told();
            }
        }
        if keep || (news && kept_at.elapsed() >= KEEP_EVERY) {
            write_kept(&dir, &kept);
            kept_at = Instant::now();
        }
        let wait = if looked {
            Duration::from_secs(1)
        } else if folders.iter().any(|counted| counted.watch.is_none()) {
            TRY_EVERY
        } else {
            DAY_EVERY
        };
        let asked = ASKED.0.lock().unwrap();
        drop(ASKED.1.wait_timeout_while(asked, wait, |asked| !asked.news).unwrap());
    }
}

fn publish(folders: &[Counted]) {
    *SHOWN.lock().unwrap() = Some(Arc::new(folders.iter().map(|counted| counted.folder.clone()).collect()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(bytes: u64, days: &[(i64, u64)]) -> Folder {
        Folder { path: r"C:\a".into(), name: "a".into(), held: Some(Held { bytes, files: 1, largest: Vec::new() }), away: false, days: days.to_vec() }
    }

    /// A folder's change is told over the last week, from what it was at the first counting of the day a week back (or
    /// the last day before it Glance ran); one added since, from the day it was; added today, nothing yet.
    #[test]
    fn a_folder_s_change_is_told_over_a_week() {
        assert_eq!(folder(50, &[(100, 50)]).change(100), Change::Begun);
        assert_eq!(folder(80, &[(98, 50), (100, 70)]).change(100), Change::Since(98, 30));
        assert_eq!(folder(80, &[(93, 100), (98, 50), (100, 70)]).change(100), Change::Week(-20));
        assert_eq!(folder(80, &[(80, 90), (98, 50)]).change(100), Change::Week(-10));
        assert_eq!(folder(80, &[]).change(100), Change::Begun);
    }

    /// The first counting of each day is kept, those after it that day are not, and those more than a week back are let
    /// go of, but the last of them.
    #[test]
    fn the_first_counting_of_a_day_is_kept() {
        let mut days = Vec::new();
        assert!(kept_on(&mut days, 100, 5));
        assert!(!kept_on(&mut days, 100, 6));
        assert_eq!(days, [(100, 5)]);
        for (day, bytes) in [(101, 7), (103, 8), (107, 9)] {
            assert!(kept_on(&mut days, day, bytes));
        }
        assert_eq!(days, [(100, 5), (101, 7), (103, 8), (107, 9)]);
        assert!(kept_on(&mut days, 108, 10));
        assert_eq!(days, [(101, 7), (103, 8), (107, 9), (108, 10)]);
        assert!(kept_on(&mut days, 130, 11));
        assert_eq!(days, [(108, 10), (130, 11)]);
        // The clock set back: the days to come are none.
        assert!(kept_on(&mut days, 110, 12));
        assert_eq!(days, [(108, 10), (110, 12)]);
    }

    /// One not counted is tried again soon after it is looked at, however it was tried last; one counted, once it has
    /// rested.
    #[test]
    fn one_not_counted_is_tried_again_soon_once_looked_at() {
        let begun = Instant::now();
        let mut counted = Counted::new(folder(0, &[]));
        assert!(counted.due(begun, 100, false));
        counted.tried = Some((begun, Duration::ZERO));
        let after = |by: Duration| begun + by;
        assert!(!counted.due(after(TRY_EVERY - Duration::from_millis(1)), 100, true));
        assert!(!counted.due(after(TRY_EVERY), 100, false));
        assert!(counted.due(after(TRY_EVERY), 100, true));
        assert!(counted.due(after(TRY_RESTING), 100, false));
    }

    /// As many as there is room for, none twice (a path's case being no other folder).
    #[test]
    fn a_folder_is_put_once() {
        let mut list = vec![r"C:\A".to_string()];
        put(&mut list, vec![r"c:\a".into(), r"C:\B".into(), r"C:\B".into()]);
        assert_eq!(list, [r"C:\A", r"C:\B"]);
        put(&mut list, (0..20).map(|n| format!(r"C:\{n}")).collect());
        assert_eq!(list.len(), MOST);
    }

    /// Raised, a folder goes before the one it was after; the first, and one not among them, stay.
    #[test]
    fn a_folder_is_raised() {
        let mut list = ["a", "b", "c"].map(String::from);
        raise(&mut list, "c");
        assert_eq!(list, ["a", "c", "b"]);
        raise(&mut list, "a");
        raise(&mut list, "d");
        assert_eq!(list, ["a", "c", "b"]);
    }

    /// What is kept is read again as it was; nothing kept, or not as written, is none.
    #[test]
    fn what_is_kept_is_read_again() {
        let dir = std::env::temp_dir().join(format!("glance-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_kept(&dir).is_empty());
        let kept = HashMap::from([(r"D:\项目".to_string(), Kept { held: Some(Held { bytes: 5, files: 2, largest: vec![("target".into(), 4)] }), days: vec![(100, 5)] })]);
        write_kept(&dir, &kept);
        assert_eq!(read_kept(&dir), kept);
        std::fs::write(dir.join(FILE), b"{").unwrap();
        assert!(read_kept(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
