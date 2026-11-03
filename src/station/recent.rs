//! Recent files: the files lately come to the user's folders, wherever they landed (downloaded, a screenshot, a
//! document saved), newest first, shown as a page of the station.
//!
//! Read as the user, never through the elevated sensors: from the change journal of each of the PC's own fixed drives, read the way NTFS
//! lets anyone read it (`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`, through the drive's root folder). Its records name no
//! files there, so each file come is opened by its number (`OpenFileById`) and its path asked of it: a file the user
//! may not open is not opened, and so never listed. The journal keeps what changed while Glance was not running: where
//! it was last read up to is kept, and read on from there, a batch at a time (the list shown and kept between them).
//! A file's number is its drive's own: a file is known by its drive and its number together.
//!
//! Only what a person keeps: the folders of Windows, programs, programs' own data and tools' (`AppData`, `.git`,
//! `node_modules`, …) are left out, and so are a repository's working tree (a checkout writes hundreds of files at
//! once, its tools' to follow) and a program's folder wherever it was put (one holding a program or a library: what is
//! written there, its logs and pictures, are the program's), as are hidden, system and temporary files and those being written (a download in progress);
//! outside Downloads and the desktop, only documents, pictures, videos, music and archives. Glance's own notes are
//! left out too. A file moved or
//! renamed is not one come (its creation is old); one renamed as a download finishes is.
//!
//! A file listed is followed: renamed, it is listed under its new name; moved with its folder (which the journal tells
//! of the folder alone), found again by its number; deleted, taken off. One that cannot be looked for now (its drive
//! away, or locked) stays listed.
//!
//! What the user asked of the list (see `Scope`): folders not listed, drives left out (each by its volume, whatever
//! letter it has), and, where chosen, only the files in some folders. A file outside it is not kept as it comes (a
//! thousand photos copied into a folder left out would else fill the list), and one kept before is not shown while it
//! is outside (listed again, it comes back).
//!
//! The page shows the files come into one folder together, many at once (copied from a phone, an archive extracted),
//! as one row (see `listed`): a rule of how the list is shown, the list kept file by file.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What a file come is, by its name: what the page's choices pick out (see `Filter`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Screenshot,
    Document,
    Picture,
    Video,
    Audio,
    Archive,
    Program,
    Other,
}

/// Which file: its drive's letter, and its number on that drive.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Key {
    pub drive: char,
    pub file: u128,
}

/// A file come lately: which it is, where it is, when it was made (ms since 1970), how large it is, what it is, whether
/// it came into Downloads, and the site it was downloaded from where its download says so.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Came {
    pub key: Key,
    pub path: PathBuf,
    pub came_ms: u64,
    pub bytes: u64,
    pub kind: Kind,
    pub downloaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

/// Which of them a page shows: all; those downloaded; screenshots; documents; pictures and videos.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Downloads,
    Screenshots,
    Documents,
    Pictures,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Downloads, Filter::Screenshots, Filter::Documents, Filter::Pictures];

    /// Whether `came` is one it shows.
    pub fn takes(self, came: &Came) -> bool {
        match self {
            Filter::All => true,
            Filter::Downloads => came.downloaded || came.site.is_some(),
            Filter::Screenshots => came.kind == Kind::Screenshot,
            Filter::Documents => came.kind == Kind::Document,
            Filter::Pictures => matches!(came.kind, Kind::Picture | Kind::Screenshot | Kind::Video),
        }
    }
}

/// How long a file stays listed after it was made, and the most listed.
const KEEP_MS: u64 = 7 * 24 * 3_600_000;
const MOST: usize = 300;
/// How often the journals are read (with nothing left to read on).
const EVERY: Duration = Duration::from_secs(2);
/// How often where the journals were read up to is kept, with nothing else changed.
const SAVE_EVERY: Duration = Duration::from_secs(60);
/// How often each file listed is looked for where it was (and at once when a folder was renamed).
const RECHECK_EVERY: Duration = Duration::from_secs(60);
/// How often which folders are tools' is forgotten (one made a repository, or a program's, is seen).
const FORGET_EVERY: Duration = Duration::from_secs(600);
/// The most of a journal read back the first time (its records' bytes): enough for the days kept on a busy drive.
const FIRST_READ_BACK: i64 = 512 << 20;
/// The most of a journal's records read from a drive at a time (the list shown and kept between such batches).
const BATCH: usize = 4 << 20;
/// A journal's page: its records never cross one, so each starts with a record (a read must start at one).
const JOURNAL_PAGE: i64 = 4096;

/// The folders that tell what a file is by where it is: Downloads, the desktop, screenshots'.
#[derive(Clone, Debug, Default)]
pub struct Places {
    pub downloads: Option<PathBuf>,
    pub desktop: Option<PathBuf>,
    pub screenshots: Option<PathBuf>,
}

/// Folders left out wherever they are: tools' and programs' own.
const LEFT_OUT: [&str; 20] = ["appdata", "node_modules", "__pycache__", "venv", "site-packages", "$recycle.bin", "system volume information", "windowsapps", "temp", "tmp", "cache", "caches", "obj", "target", "build", "dist", "thumb", "thumbs", "thumbnail", "thumbnails"];
/// Folders left out at a drive's top: Windows', programs'.
const LEFT_OUT_AT_TOP: [&str; 7] = ["windows", "program files", "program files (x86)", "programdata", "recovery", "$windows.~bt", "msocache"];

/// Whether `path` is in a folder left out (see the module's note), or one of `hidden` (folders the user asked not to
/// be listed). Its name is not looked at: only the folders it is in.
pub fn left_out(path: &Path, hidden: &[PathBuf]) -> bool {
    if hidden.iter().any(|folder| under(path, folder)) {
        return true;
    }
    let folders: Vec<String> = path.parent().into_iter().flat_map(|parent| parent.components()).filter_map(|part| match part {
        std::path::Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
        _ => None,
    }).collect();
    folders.first().is_some_and(|top| LEFT_OUT_AT_TOP.contains(&top.as_str())) || folders.iter().any(|folder| folder.starts_with('.') || LEFT_OUT.contains(&folder.as_str()))
}

/// Whether `path` is `folder` or in it, its names compared without case (as NTFS tells names apart): `d:\photos`
/// holds `D:\Photos\a.jpg`.
fn under(path: &Path, folder: &Path) -> bool {
    let name = |part: std::path::Component| part.as_os_str().to_string_lossy().to_lowercase();
    let mut parts = path.components();
    folder.components().all(|part| parts.next().is_some_and(|mine| name(mine) == name(part)))
}

/// What the list shows of the files come, as the user asked (see `Settings::recent_hidden` and those after it):
/// none in a folder not listed, none on a drive left out (by its volume: see `Volume`), and where `only` is some,
/// only those in its folders (or below them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub hidden: Vec<PathBuf>,
    pub drives: Vec<String>,
    pub only: Option<Vec<PathBuf>>,
}

impl Scope {
    pub fn of(settings: &crate::settings::Settings) -> Scope {
        Scope {
            hidden: settings.recent_hidden.clone(),
            drives: settings.recent_hidden_drives.iter().map(|drive| drive.volume.clone()).collect(),
            only: settings.recent_only.then(|| settings.recent_folders.clone()),
        }
    }

    /// Whether the file at `path`, on the volume `volume` (none: not known), is one shown.
    pub fn takes(&self, path: &Path, volume: Option<&str>) -> bool {
        !self.hidden.iter().any(|folder| under(path, folder))
            && !volume.is_some_and(|volume| self.drives.iter().any(|out| out == volume))
            && self.only.as_ref().is_none_or(|folders| folders.iter().any(|folder| under(path, folder)))
    }

    /// Whether it shows nothing whatever comes: only some folders' files, and no folder chosen.
    pub fn nowhere(&self) -> bool {
        self.only.as_ref().is_some_and(Vec::is_empty)
    }
}

/// A drive the list reads (see `os::drives`), as the settings offer it: its letter now, its volume as Windows names
/// it whatever letter it is given (`\\?\Volume{…}\`, what a drive is known by: see `settings::HiddenDrive`), and its
/// label.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Volume {
    pub letter: char,
    pub volume: String,
    pub label: String,
}

/// The drives the list reads now (see `os::drives`), by their letters.
#[cfg(windows)]
pub fn volumes() -> Vec<Volume> {
    os::volumes()
}

/// Files come into one folder together, one after another, as a program writes them (copied, extracted, synced),
/// shown as one row (see `listed`): each began within `BURST_GAP_MS` of the one before, past the time that one took to
/// write at `BURST_RATE` (bytes a second: a file is made as it begins to be written, so the next begins as it is
/// done), and at least `BURST_LEAST` of them. A person saving, taking a screenshot or downloading is seldom five
/// seconds apart five times running. Ten MB a second is a slow source (a phone on USB 2, a card), a 2 GB video copied
/// from it chained for 205 s. Five rows are most of a page (some six show), fewer shown each by its name.
const BURST_GAP_MS: u64 = 5_000;
const BURST_RATE: u64 = 10_000_000;
const BURST_LEAST: usize = 5;

/// `files` (newest first) as the page lists those `filter` takes: each alone, or those come into one folder together
/// (see `BURST_GAP_MS`) as one, the newest first among them, where `filter` takes `BURST_LEAST` or more of them. A
/// burst is found among all the files (one of them the filter leaves out does not break it), and stands where its
/// newest file would. Its folder is the folder itself (an archive extracted into folders comes as a burst in each),
/// its name told without case.
pub fn listed(files: &[Came], filter: Filter) -> Vec<Vec<&Came>> {
    let mut folders: HashMap<String, Vec<usize>> = HashMap::new();
    for (at, came) in files.iter().enumerate() {
        folders.entry(came.path.parent().map(|folder| folder.to_string_lossy().to_lowercase()).unwrap_or_default()).or_default().push(at);
    }
    // Each file's burst, where it is in one (by its place among the bursts).
    let mut burst_of: Vec<Option<usize>> = vec![None; files.len()];
    let mut bursts = 0;
    let mut close = |run: &mut Vec<usize>| {
        let taken: Vec<usize> = run.drain(..).filter(|at| filter.takes(&files[*at])).collect();
        if taken.len() >= BURST_LEAST {
            taken.into_iter().for_each(|at| burst_of[at] = Some(bursts));
            bursts += 1;
        }
    };
    for (_, mut ats) in folders {
        ats.sort_by_key(|at| (files[*at].came_ms, std::cmp::Reverse(*at)));
        let mut run: Vec<usize> = Vec::new();
        for at in ats {
            if let Some(&last) = run.last() {
                let within = BURST_GAP_MS + files[last].bytes.saturating_mul(1000) / BURST_RATE;
                if files[at].came_ms.saturating_sub(files[last].came_ms) > within {
                    close(&mut run);
                }
            }
            run.push(at);
        }
        close(&mut run);
    }
    let mut rows: Vec<Vec<&Came>> = Vec::new();
    // Where each burst's row is, once its newest file put it there.
    let mut row_of: HashMap<usize, usize> = HashMap::new();
    for (at, came) in files.iter().enumerate().filter(|(_, came)| filter.takes(came)) {
        match burst_of[at] {
            Some(burst) => match row_of.get(&burst) {
                Some(&row) => rows[row].push(came),
                None => {
                    row_of.insert(burst, rows.len());
                    rows.push(vec![came]);
                }
            },
            None => rows.push(vec![came]),
        }
    }
    rows
}

/// A row of the page: a file come lately, or the files come together (see `listed`), as the page's filter shows them,
/// by one of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Entry {
    File(Key),
    Burst(Filter, Key),
}

/// Where the files of `entry` are, as the list is now: the newest first.
pub fn paths_of(entry: Entry) -> Option<Vec<PathBuf>> {
    let shown = shown();
    match entry {
        Entry::File(key) => shown.iter().find(|came| came.key == key).map(|came| vec![came.path.clone()]),
        Entry::Burst(filter, key) => listed(&shown, filter).into_iter().find(|row| row.iter().any(|came| came.key == key)).map(|row| row.iter().map(|came| came.path.clone()).collect()),
    }
}

/// What the file at `path` is (see `Kind`), and whether it came into Downloads; none, for a file not listed: one being
/// written or left by a program (a download in progress, Office's lock), or, outside Downloads and the desktop, one that
/// is none of the kinds a person keeps.
pub fn kind_of(path: &Path, places: &Places) -> Option<(Kind, bool)> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.starts_with("~$") || name.starts_with('.') {
        return None;
    }
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ["tmp", "temp", "crdownload", "part", "partial", "download", "opdownload", "!ut", "lock", "lnk", "ini", "log"].contains(&extension.as_str()) {
        return None;
    }
    let inside = |folder: &Option<PathBuf>| folder.as_ref().is_some_and(|folder| path.starts_with(folder));
    let downloaded = inside(&places.downloads);
    let anything = downloaded || inside(&places.desktop);
    let kind = match kind_of_extension(&extension) {
        Kind::Picture => {
            let shot = inside(&places.screenshots) || ["屏幕截图", "screenshot", "截图", "snipaste"].iter().any(|start| name.starts_with(start));
            if shot { Kind::Screenshot } else { Kind::Picture }
        }
        Kind::Program | Kind::Other if !anything => return None,
        kind => kind,
    };
    Some((kind, downloaded))
}

/// The extensions (lower case, no dot) of the kinds of file a person keeps.
pub const DOCUMENTS: &[&str] = &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "epub", "pages", "key", "numbers", "wps", "et", "dps"];
pub const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "svg", "psd", "ai", "avif", "dng", "cr2", "cr3", "nef", "arw"];
pub const VIDEOS: &[&str] = &["mp4", "mkv", "mov", "avi", "webm", "wmv", "flv", "m4v"];
pub const AUDIO: &[&str] = &["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"];
pub const ARCHIVES: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso"];
pub const PROGRAMS: &[&str] = &["exe", "msi", "msix", "msixbundle", "appx"];

/// What a file of this extension (lower case, no dot) is, by its extension alone (never a screenshot: that is told by
/// its name and its folder).
pub fn kind_of_extension(extension: &str) -> Kind {
    [(DOCUMENTS, Kind::Document), (PICTURES, Kind::Picture), (VIDEOS, Kind::Video), (AUDIO, Kind::Audio), (ARCHIVES, Kind::Archive), (PROGRAMS, Kind::Program)]
        .into_iter()
        .find(|(extensions, _)| extensions.contains(&extension))
        .map_or(Kind::Other, |(_, kind)| kind)
}

/// Whether a file of this extension (lower case, no dot) is a picture.
pub fn is_picture(extension: &str) -> bool {
    PICTURES.contains(&extension)
}

/// The site a download came from, as its `Zone.Identifier` says (`text`): the page it was taken from (`ReferrerUrl`),
/// else where the file itself was (`HostUrl`); none where it names no web address (it may say only its zone).
pub fn site_of(text: &str) -> Option<String> {
    let value = |key: &str| text.lines().find_map(|line| line.trim().strip_prefix(key).map(str::trim).map(str::to_string));
    let host = |url: String| -> Option<String> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        let host = rest.split(['/', '?', '#', ':']).next()?.trim_start_matches("www.");
        (!host.is_empty()).then(|| host.to_string())
    };
    value("ReferrerUrl=").and_then(host).or_else(|| value("HostUrl=").and_then(host))
}

/// One change a journal's record tells: the file's number and its folder's, why (its reasons, as `USN_REASON_*`), its
/// attributes, and when (ms since 1970).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Change {
    pub file: u128,
    pub folder: u128,
    pub reason: u32,
    pub attributes: u32,
    pub at_ms: u64,
}

/// The changes in a read of a journal (`read`: what `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` gave), and where to read on
/// from. Records of versions 2 (NTFS) and 3 (ReFS, with longer numbers) are read; others are passed over.
pub fn changes_in(read: &[u8]) -> Option<(i64, Vec<Change>)> {
    let next = i64::from_le_bytes(read.get(..8)?.try_into().ok()?);
    let u16_at = |at: usize| read.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| read.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64_at = |at: usize| read.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    let u128_at = |at: usize| Some(u128::from(u64_at(at)?) | (u128::from(u64_at(at + 8)?) << 64));
    let mut changes = Vec::new();
    let mut at = 8;
    while at + 8 <= read.len() {
        let Some(length) = u32_at(at).filter(|length| *length >= 8) else { break };
        let change = match u16_at(at + 4) {
            Some(2) => (|| Some(Change { file: u128::from(u64_at(at + 8)?), folder: u128::from(u64_at(at + 16)?), at_ms: ms_of(u64_at(at + 32)?), reason: u32_at(at + 40)?, attributes: u32_at(at + 52)? }))(),
            Some(3) => (|| Some(Change { file: u128_at(at + 8)?, folder: u128_at(at + 24)?, at_ms: ms_of(u64_at(at + 48)?), reason: u32_at(at + 56)?, attributes: u32_at(at + 68)? }))(),
            _ => None,
        };
        changes.extend(change);
        at += length as usize;
    }
    Some((next, changes))
}

/// Each file's changes in `changes` as one: its reasons together (a file made, then written again, was made), as it
/// was last (its folder, its attributes, when), in the order each was first changed.
fn together(changes: Vec<Change>) -> Vec<Change> {
    let mut at: HashMap<u128, usize> = HashMap::new();
    let mut together: Vec<Change> = Vec::new();
    for change in changes {
        match at.get(&change.file) {
            Some(&was) => together[was] = Change { reason: together[was].reason | change.reason, ..change },
            None => {
                at.insert(change.file, together.len());
                together.push(change);
            }
        }
    }
    together
}

/// A file time (100 ns since 1601) in ms since 1970.
fn ms_of(file_time: u64) -> u64 {
    (file_time / 10_000).saturating_sub(11_644_473_600_000)
}

/// What the change journals' reasons, attributes and the files' state say of a change.
mod reasons {
    pub const CREATED: u32 = 0x0000_0100;
    pub const DELETED: u32 = 0x0000_0200;
    pub const RENAMED_TO: u32 = 0x0000_2000;
    pub const CLOSED: u32 = 0x8000_0000;
    pub const HIDDEN: u32 = 0x2;
    pub const SYSTEM: u32 = 0x4;
    pub const FOLDER: u32 = 0x10;
    pub const TEMPORARY: u32 = 0x100;
    /// What a file listed is not: hidden, the system's, a folder, passing.
    pub const NOT_LISTED: u32 = HIDDEN | SYSTEM | FOLDER | TEMPORARY;
}

/// A file looked for by its number: where it is (its path, when it was made, how large it is, its attributes), gone, not the user's
/// to open (never listed), or not to be told now (its drive away, it locked: asked again later).
#[derive(Clone, PartialEq, Debug)]
pub enum Looked {
    At(PathBuf, u64, u64, u32),
    Gone,
    Theirs,
    Unknown,
}

/// How many times a file come that could not be opened is tried again (a turn apart: see `EVERY`), and the most such
/// files waiting at once.
const LATER_TRIES: u8 = 30;
const LATER_MOST: usize = 1000;

/// The list, as kept between runs: where each drive's journal was read up to (by its letter, the journal's number,
/// and the place in it), the files come, and the files come that could not be opened as they were read, to be tried
/// again (their journal's records read past: kept with where it was read up to, so a restart does not lose them).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Kept {
    read: Vec<(char, u64, i64)>,
    files: Vec<Came>,
    #[serde(default)]
    later: Vec<(char, Change, u8)>,
    /// The volume last seen at each drive's letter (see `Volume`): a file listed from a drive now away is told by the
    /// drive it was on (one left out stays out).
    #[serde(default)]
    volumes: Vec<(char, String)>,
}

impl Kept {
    /// `change` on drive `drive` seen: a file taken off the list where it is gone; one come, or come under another
    /// name, opened by its number (`open`) and listed if it is one listed (made within the days kept before `now`, as
    /// `kind_of` says, and not `out`); one listed, written again, followed (how large it is now); whether the list
    /// changed. (A change older than the days kept, of a file not listed, is of a file made before them: never opened.)
    /// One that could not be opened now is kept to be tried again (see `retry`).
    fn take(&mut self, drive: char, change: Change, now: u64, places: &Places, out: &dyn Fn(&Path) -> bool, open: impl Fn(u128) -> Looked, site: impl Fn(&Path) -> Option<String>) -> bool {
        use reasons::*;
        let key = Key { drive, file: change.file };
        if change.reason & CLOSED == 0 {
            return false;
        }
        if change.reason & DELETED != 0 {
            let before = self.files.len();
            self.files.retain(|came| came.key != key);
            self.later.retain(|(letter, later, _)| (*letter, later.file) != (drive, change.file));
            return self.files.len() != before;
        }
        let had = self.files.iter().position(|came| came.key == key);
        let come = change.reason & (CREATED | RENAMED_TO) != 0 && now.saturating_sub(change.at_ms) <= KEEP_MS;
        // (Made hidden, the system's or passing since it was listed: no longer listed.)
        if change.attributes & reasons::NOT_LISTED != 0 {
            self.later.retain(|(letter, later, _)| (*letter, later.file) != (drive, change.file));
            return had.map(|at| self.files.remove(at)).is_some();
        }
        if !(come || had.is_some()) {
            return false;
        }
        let (path, made_ms, bytes) = match open(change.file) {
            Looked::At(_, _, _, attributes) if attributes & reasons::NOT_LISTED != 0 => return had.map(|at| self.files.remove(at)).is_some(),
            Looked::At(path, made_ms, bytes, _) => (path, made_ms, bytes),
            Looked::Unknown => {
                if self.later.len() < LATER_MOST && !self.later.iter().any(|(letter, later, _)| (*letter, later.file) == (drive, change.file)) {
                    self.later.push((drive, change, 0));
                }
                return false;
            }
            Looked::Gone | Looked::Theirs => return false,
        };
        let listed = (now.saturating_sub(made_ms) <= KEEP_MS && !out(&path)).then(|| kind_of(&path, places)).flatten();
        match (listed, had) {
            (Some((kind, downloaded)), Some(at)) => {
                let came = &mut self.files[at];
                let site = if downloaded { site(&path) } else { None };
                let changed = came.path != path || came.kind != kind || came.bytes != bytes;
                *came = Came { path, kind, downloaded, bytes, site: site.or(came.site.take()), ..came.clone() };
                changed
            }
            (Some((kind, downloaded)), None) => {
                let site = site(&path);
                self.files.push(Came { key, path, came_ms: made_ms, bytes, kind, downloaded, site });
                self.files.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
                self.files.truncate(MOST);
                true
            }
            // (Renamed into a folder left out, or to a name not listed: no longer listed.)
            (None, Some(at)) => {
                self.files.remove(at);
                true
            }
            (None, None) => false,
        }
    }

    /// The drives as they were looked for: the volume at each one's letter (see `volumes`).
    fn saw<'a>(&mut self, drives: impl IntoIterator<Item = (char, &'a str)>) {
        for (letter, volume) in drives {
            self.volumes.retain(|(at, _)| *at != letter);
            self.volumes.push((letter, volume.to_string()));
        }
    }

    /// The files kept that `scope` shows, each on the drive its letter last was.
    fn shown(&self, scope: &Scope) -> Vec<Came> {
        self.files.iter().filter(|came| !outside(scope, &self.volumes, came.key.drive, &came.path)).cloned().collect()
    }

    /// What was listed from the drives `outside` (by their letters and volumes), what waits to be tried on them, and
    /// where their journals were read up to, let go of: each only where its letter last had that volume (a drive of
    /// the PC's own once at that letter keeps its own). Whether anything was.
    fn let_go(&mut self, outside: &[(char, String)]) -> bool {
        let letters: Vec<char> = outside.iter().filter(|drive| self.volumes.contains(drive)).map(|(letter, _)| *letter).collect();
        if letters.is_empty() {
            return false;
        }
        self.files.retain(|came| !letters.contains(&came.key.drive));
        self.later.retain(|(letter, ..)| !letters.contains(letter));
        self.read.retain(|(letter, ..)| !letters.contains(letter));
        self.volumes.retain(|(letter, _)| !letters.contains(letter));
        true
    }

    /// Those made longer ago than the days kept before `now` taken off; whether any were.
    fn age(&mut self, now: u64) -> bool {
        let before = self.files.len();
        self.files.retain(|came| now.saturating_sub(came.came_ms) <= KEEP_MS);
        self.files.len() != before
    }

    /// The files come that could not be opened as they were read, tried again (`open`: on their drives, `out`: left
    /// out on that drive), each up to `LATER_TRIES` times, and whether the list changed.
    fn retry(&mut self, now: u64, places: &Places, out: &dyn Fn(char, &Path) -> bool, open: impl Fn(char, u128) -> Looked, site: impl Fn(&Path) -> Option<String>) -> bool {
        let mut changed = false;
        for (drive, change, tries) in std::mem::take(&mut self.later) {
            let waiting = self.later.len();
            changed |= self.take(drive, change, now, places, &|path| out(drive, path), |file| open(drive, file), &site);
            // (Still not to be told: tried once more, or let go of.)
            if self.later.len() > waiting {
                if tries + 1 >= LATER_TRIES {
                    self.later.pop();
                } else if let Some(last) = self.later.last_mut() {
                    last.2 = tries + 1;
                }
            }
        }
        changed
    }

    /// Each file listed looked for by its number (`look`): followed where it is now (moved with its folder: the journal
    /// tells of the folder alone), and what it is told again there (`kind_of`: moved out of Downloads, or into the
    /// screenshots' folder); taken off where it is gone, or where it is now `out` or no kind listed there; one that
    /// cannot be looked for now (its drive away, it locked) kept as it is. Whether the list changed. (By its number,
    /// not its path: another file put where it was is not it.)
    /// Only those `which` asks for are looked for (the rest kept as they are).
    fn recheck(&mut self, places: &Places, look: impl Fn(Key) -> Looked, out: &dyn Fn(&Path) -> bool, which: impl Fn(&Came) -> bool) -> bool {
        let mut changed = false;
        self.files.retain_mut(|came| {
            if !which(came) {
                return true;
            }
            let keep = match look(came.key) {
                // (Made hidden, the system's or passing meanwhile: no longer listed.)
                Looked::At(_, _, _, attributes) if attributes & reasons::NOT_LISTED != 0 => false,
                Looked::At(path, _, bytes, _) => match (!out(&path)).then(|| kind_of(&path, places)).flatten() {
                    Some((kind, downloaded)) => {
                        if (&came.path, came.kind, came.downloaded, came.bytes) != (&path, kind, downloaded, bytes) {
                            (came.path, came.kind, came.downloaded, came.bytes) = (path, kind, downloaded, bytes);
                            changed = true;
                        }
                        true
                    }
                    None => false,
                },
                Looked::Gone => false,
                Looked::Theirs | Looked::Unknown => true,
            };
            changed |= !keep;
            keep
        });
        changed
    }
}

/// Whether the file at `path`, on drive `letter`, is one `scope` does not show, the drive known by the volume
/// `volumes` last saw at its letter (see `Kept::volumes`, none seen: not left out by its drive).
fn outside(scope: &Scope, volumes: &[(char, String)], letter: char, path: &Path) -> bool {
    !scope.takes(path, volumes.iter().find(|(at, _)| *at == letter).map(|(_, volume)| volume.as_str()))
}

/// Which folders are tools': in a repository's working tree (it, or a folder above, holds `.git`), or in a program's
/// folder (it, or a folder above, holds a program or a library), as each was looked at (forgotten now and then: one
/// made or removed is seen). The folders above are asked first: one held holds all below it, which are not looked at.
/// Downloads, the desktop, the user's own folder and a drive's top are never a program's, whatever they hold, and are
/// not looked through for one. `looks` says what a folder holds itself: whether a repository, or (asked to look for
/// one) a program.
struct Tools<F: Fn(&Path, bool) -> bool> {
    known: std::cell::RefCell<HashMap<PathBuf, bool>>,
    never: Vec<PathBuf>,
    looks: F,
}

/// Whether the folder at `folder` is a tool's itself, as `Tools` asks: a repository (`.git`), or, where `programs`, a
/// folder holding a program or a library.
fn looked_at(folder: &Path, programs: bool) -> bool {
    folder.join(".git").exists()
        || programs
            && std::fs::read_dir(folder).is_ok_and(|entries| entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                (name.ends_with(".exe") || name.ends_with(".dll")) && entry.file_type().is_ok_and(|kind| kind.is_file())
            }))
}

/// Whether a folder renamed or moved, `now` where it is (see `os::Drive::look`), went where all in it is left out (by
/// `out`, asked of a file in it) and made no other folder a tool's: what was asked of every other folder stands. A
/// repository's `.git` makes the folder it is in one (see `looked_at`). One not to be looked at now: not known.
fn moved_where_left_out(now: &Looked, out: impl Fn(&Path) -> bool) -> bool {
    matches!(now, Looked::At(folder, ..) if !folder.file_name().is_some_and(|name| name.eq_ignore_ascii_case(".git")) && out(&folder.join("_")))
}

impl<F: Fn(&Path, bool) -> bool> Tools<F> {
    fn new(places: &Places, looks: F) -> Self {
        let profile = places.desktop.as_ref().and_then(|desktop| desktop.parent()).map(Path::to_path_buf);
        Tools { known: Default::default(), never: [places.downloads.clone(), places.desktop.clone(), profile].into_iter().flatten().collect(), looks }
    }

    /// Whether the file at `path` is in a tool's folder.
    fn hold(&self, path: &Path) -> bool {
        path.parent().is_some_and(|folder| self.holds(folder))
    }

    /// Whether `folder` is a tool's, or in one.
    fn holds(&self, folder: &Path) -> bool {
        if let Some(known) = self.known.borrow().get(folder) {
            return *known;
        }
        let held = folder.parent().is_some_and(|above| self.holds(above)) || (self.looks)(folder, folder.parent().is_some() && !self.never.iter().any(|never| never == folder));
        let mut known = self.known.borrow_mut();
        if known.len() > 20_000 {
            known.clear();
        }
        known.insert(folder.to_path_buf(), held);
        held
    }

    fn forget(&self) {
        self.known.borrow_mut().clear();
    }
}

static SHOWN: Mutex<Option<Arc<Vec<Came>>>> = Mutex::new(None);
/// Whether the list is on (see `StationSettings::recent`).
static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// What the settings ask of the list, as they last changed (see `follow_settings`): what it shows, while it is on,
/// none while it is off, and its thread sleeps till it is on.
static WANTED: (Mutex<Option<Scope>>, std::sync::Condvar) = (Mutex::new(None), std::sync::Condvar::new());
/// Whether what the settings ask shows nothing whatever comes (see `Scope::nowhere`).
static NOWHERE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the list is on, and what it shows, taken from the settings as they change (its thread takes them from
/// here, and does not read the settings).
pub fn follow_settings(settings: &crate::settings::Settings) {
    let scope = settings.station.recent.then(|| Scope::of(settings));
    NOWHERE.store(scope.as_ref().is_some_and(Scope::nowhere), std::sync::atomic::Ordering::Relaxed);
    *WANTED.0.lock().unwrap() = scope;
    WANTED.1.notify_all();
}

/// Whether the files come lately are listed (switched on in the settings, and followed).
pub fn on() -> bool {
    ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the list shows nothing whatever comes: only the files in some folders, and none chosen.
pub fn nowhere() -> bool {
    NOWHERE.load(std::sync::atomic::Ordering::Relaxed)
}
static TOLD: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

#[cfg(any(test, feature = "studio"))]
thread_local! {
    /// What this thread's pictures pretend (see `pretend`): each test's its own, as tests run side by side.
    static PRETENDED: std::cell::RefCell<Option<Arc<Vec<Came>>>> = const { std::cell::RefCell::new(None) };
}

/// The files come lately, newest first (none while the list is off, or not yet read).
pub fn shown() -> Arc<Vec<Came>> {
    #[cfg(any(test, feature = "studio"))]
    if let Some(pretended) = PRETENDED.with(|pretended| pretended.borrow().clone()) {
        return pretended;
    }
    SHOWN.lock().unwrap().clone().unwrap_or_default()
}

/// For pictures: as if these had come (its thread not started).
#[cfg(any(test, feature = "studio"))]
pub fn pretend(files: Vec<Came>) {
    ON.store(true, std::sync::atomic::Ordering::Relaxed);
    PRETENDED.with(|pretended| *pretended.borrow_mut() = Some(Arc::new(files)));
}

/// Starts following the drives on a thread of its own; `told` is called whenever the list changed.
pub fn start(told: impl Fn() + Send + Sync + 'static) {
    if TOLD.set(Box::new(told)).is_err() {
        return;
    }
    std::thread::Builder::new().name("glance: recent".into()).spawn(follow).expect("a thread started");
}

/// What it has found lately, kept in Glance's local folder (see `settings::local_dir`).
pub const FILE: &str = "recent.json";

#[cfg(windows)]
fn follow() {
    let folder = crate::settings::local_dir();
    let mut kept: Kept = std::fs::read(folder.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let places = windows_places();
    let mut drives: Vec<os::Drive> = Vec::new();
    // (Never yet: an `Instant` counts from the PC's start, and an hour before it is none to be had in its first hour.)
    let mut looked_for_drives: Option<Instant> = None;
    let mut rechecked: Option<Instant> = None;
    let mut forgotten = Instant::now();
    let mut saved = Instant::now();
    // What is not yet kept: the list changed, where the journals were read up to moved.
    let (mut files_owed, mut read_owed) = (false, false);
    let tools = Tools::new(&places, looked_at);
    // The folders (by drive and number) whose files are left out, as one of their files was found to be: their files
    // not opened again (forgotten as a folder is renamed, or with the tools').
    let skipped: std::cell::RefCell<HashSet<(char, u128)>> = Default::default();
    // What was last shown of the list (none: nothing yet).
    let mut shown_scope: Option<Scope> = None;
    loop {
        let wanted = WANTED.0.lock().unwrap().clone();
        let was_on = ON.swap(wanted.is_some(), std::sync::atomic::Ordering::Relaxed);
        let Some(scope) = wanted else {
            if was_on {
                publish(Vec::new());
                shown_scope = None;
            }
            // (Off, no drive is held: each is opened again as it is on.)
            drives.clear();
            looked_for_drives = None;
            drop(WANTED.1.wait_while(WANTED.0.lock().unwrap(), |wanted| wanted.is_none()).unwrap());
            continue;
        };
        // The drives looked for again now and then (one put in, or formatted), and the volume at each letter kept;
        // what was listed from a drive plugged in from outside let go of (listed before such drives were left alone).
        let mut let_go = false;
        if looked_for_drives.is_none_or(|looked| looked.elapsed() > Duration::from_secs(60)) {
            let outside;
            (drives, outside) = os::drives();
            looked_for_drives = Some(Instant::now());
            kept.saw(drives.iter().map(|drive| (drive.letter, drive.volume.as_str())));
            let_go = kept.let_go(&outside);
        }
        // (The folders skipped are known by their numbers: one out of what was shown may be in what is shown now.)
        if forgotten.elapsed() > FORGET_EVERY || shown_scope.as_ref().is_some_and(|shown| *shown != scope) {
            tools.forget();
            skipped.borrow_mut().clear();
            forgotten = Instant::now();
        }
        // (Glance's own notes too: written in Glance, they are no file come to the user's folders.)
        let own = [crate::notes::folder().path];
        let out = |path: &Path| left_out(path, &own) || tools.hold(path);
        // What is not shown is not kept as it comes either (see the module's note), and what was kept is shown as it
        // is asked now. A file is told by the volume its drive's letter last had.
        let volumes = kept.volumes.clone();
        let out_on = |letter: char, path: &Path| out(path) || outside(&scope, &volumes, letter, path);
        let now = super::now_ms();
        let mut changed = kept.age(now) | let_go;
        let mut folder_renamed = false;
        let mut files_moved = false;
        let mut more = false;
        for drive in &mut drives {
            // (Where its journal starts and ends now: a place read up to that its records have since run past is read
            // on from where they start; a journal made anew, read as the first time.)
            if !drive.query() {
                continue;
            }
            // (A drive left out: its journal read on, its files not opened, but those kept followed as they go.)
            let drive_out = scope.drives.contains(&drive.volume);
            let read_to = kept.read.iter().find(|(letter, journal, _)| *letter == drive.letter && *journal == drive.journal).map(|(_, _, at)| *at);
            let from = read_to.map_or_else(|| drive.start(FIRST_READ_BACK), |at| at.max(drive.first));
            let Some((to, changes, done)) = drive.read(from, BATCH) else { continue };
            more |= !done;
            let letter = drive.letter;
            for change in together(changes) {
                if change.attributes & reasons::FOLDER != 0 {
                    // (A folder renamed or moved: the files listed from it followed. Where it is now left out, what
                    // was asked of every other folder stands, and the files listed that are no longer where they were
                    // are looked for (one may have been in it); elsewhere, which folders are left out is asked anew,
                    // and every file listed looked for. A user's temporary folders are renamed every second or so.)
                    if change.reason & reasons::RENAMED_TO != 0 && change.reason & reasons::CLOSED != 0 {
                        if moved_where_left_out(&drive.look(change.file), |path| out_on(letter, path)) {
                            files_moved = true;
                        } else {
                            folder_renamed = true;
                            skipped.borrow_mut().retain(|(drive, _)| *drive != letter);
                            tools.forget();
                        }
                    }
                    continue;
                }
                if change.reason & reasons::DELETED == 0 && skipped.borrow().contains(&(letter, change.folder)) {
                    continue;
                }
                if drive_out && !kept.files.iter().any(|came| came.key == Key { drive: letter, file: change.file }) {
                    continue;
                }
                // (A file found left out: its folder (where the record says it was) skipped from now on, all left out
                // being so by its folders, once that folder is known to be left out itself: the file may have moved
                // since out of the folder the record names, from one listed into one left out.)
                let out_here = |path: &Path| {
                    let left = out_on(letter, path);
                    if left && !skipped.borrow().contains(&(letter, change.folder)) {
                        let folder_left = matches!(drive.look(change.folder), Looked::At(folder, ..) if out_on(letter, &folder.join("_")));
                        if folder_left {
                            let mut skipped = skipped.borrow_mut();
                            if skipped.len() > 50_000 {
                                skipped.clear();
                            }
                            skipped.insert((letter, change.folder));
                        }
                    }
                    left
                };
                changed |= kept.take(letter, change, now, &places, &out_here, |file| drive.look(file), |path| std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).ok().and_then(|text| site_of(&text)));
            }
            if read_to != Some(to) {
                kept.read.retain(|(letter, ..)| *letter != drive.letter);
                kept.read.push((drive.letter, drive.journal, to));
                read_owed = true;
            }
        }
        // The files come that could not be opened, tried again; each file listed looked for now and then, and as a
        // folder was renamed.
        let open = |drive: char, file: u128| drives.iter().find(|one| one.letter == drive).map_or(Looked::Unknown, |one| one.look(file));
        if !kept.later.is_empty() {
            changed |= kept.retry(now, &places, &out_on, open, |path| std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).ok().and_then(|text| site_of(&text)));
            // (What waits kept with where the journals were read up to, now and then.)
            read_owed = true;
        }
        if folder_renamed || rechecked.is_none_or(|rechecked| rechecked.elapsed() > RECHECK_EVERY) {
            changed |= kept.recheck(&places, |key| open(key.drive, key.file), &out, |_| true);
            rechecked = Some(Instant::now());
        } else if files_moved {
            changed |= kept.recheck(&places, |key| open(key.drive, key.file), &out, |came| !came.path.exists());
        }
        if changed || shown_scope.as_ref() != Some(&scope) {
            publish(kept.shown(&scope));
            shown_scope = Some(scope);
        }
        // Kept as it changes, and where each journal was read up to now and then (a restart reads on from there); one
        // not kept, kept at the next turn.
        files_owed |= changed;
        if files_owed || read_owed && saved.elapsed() > SAVE_EVERY {
            let written = serde_json::to_vec(&kept).map_err(std::io::Error::other).and_then(|bytes| {
                crate::elevation::ensure_folder(&folder)?;
                crate::elevation::write_in_place(&folder, FILE, &bytes)
            });
            if written.is_ok() {
                (files_owed, read_owed) = (false, false);
                saved = Instant::now();
            }
        }
        // (Read on at once while a journal has more: the list shown and kept between its batches.)
        if !more {
            std::thread::sleep(EVERY);
        }
    }
}

fn publish(files: Vec<Came>) {
    *SHOWN.lock().unwrap() = Some(Arc::new(files));
    if let Some(told) = TOLD.get() {
        told();
    }
}

/// This user's Downloads, desktop and screenshots' folders.
#[cfg(windows)]
fn windows_places() -> Places {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Downloads, FOLDERID_Screenshots, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let known = |id: &windows::core::GUID| unsafe {
        let found = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = found.to_string().ok().map(PathBuf::from);
        windows::Win32::System::Com::CoTaskMemFree(Some(found.0 as *const _));
        path
    };
    Places { downloads: known(&FOLDERID_Downloads), desktop: known(&FOLDERID_Desktop), screenshots: known(&FOLDERID_Screenshots) }
}

#[cfg(windows)]
mod os {
    use super::{Looked, JOURNAL_PAGE};
    use std::path::PathBuf;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, GENERIC_READ, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetFileInformationByHandle, GetFinalPathNameByHandleW, GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, OpenFileById,
        BY_HANDLE_FILE_INFORMATION, ExtendedFileIdType, FileIdType, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_128, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0,
        FILE_NAME_NORMALIZED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0};
    use windows::Win32::System::IO::DeviceIoControl;

    /// A fixed drive whose journal is followed: its letter, its volume (see `super::Volume`), its root folder held
    /// open, the journal's number, where its records start and where the next will be (as last asked).
    pub struct Drive {
        pub letter: char,
        pub volume: String,
        root: HANDLE,
        pub journal: u64,
        pub first: i64,
        pub next: i64,
        /// Its numbers twice as long (ReFS's).
        long: bool,
    }

    impl Drop for Drive {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.root) };
        }
    }

    /// The PC's own fixed drives with a change journal the user may read (NTFS's and ReFS's), and the drives plugged in
    /// from outside, by their letters and volumes (each told once: one that cannot be told is neither).
    pub fn drives() -> (Vec<Drive>, Vec<(char, String)>) {
        let (mut own, mut outside) = (Vec::new(), Vec::new());
        for letter in letters() {
            match crate::drives::outside(letter) {
                Some(false) => own.extend(Drive::at(letter)),
                Some(true) => outside.extend(volume(&HSTRING::from(format!("{letter}:\\"))).map(|volume| (letter, volume))),
                None => {}
            }
        }
        (own, outside)
    }

    /// The volume at `root` ("D:\"), as Windows names it.
    fn volume(root: &HSTRING) -> Option<String> {
        let mut volume = [0u16; 64];
        unsafe { GetVolumeNameForVolumeMountPointW(root, &mut volume) }.ok()?;
        Some(String::from_utf16_lossy(&volume).trim_end_matches('\0').to_string())
    }

    /// The drives' letters Windows has now.
    fn letters() -> impl Iterator<Item = char> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8).filter(move |bit| mask & (1 << bit) != 0).map(|bit| (b'A' + bit) as char)
    }

    /// The drive at `letter`, one of the PC's own (see `drives`), where its journal is followed: a fixed drive, NTFS or
    /// ReFS. Removable ones are not (most have no journal, and one held open could not be taken out safely), nor is a
    /// drive plugged in from outside that Windows calls fixed (a USB hard drive: held open, it could not be taken out
    /// safely either, and each file copied to it was opened again as it came, the drive read while it was being written).
    /// Its volume, its label, and whether it is ReFS.
    fn followed(letter: char) -> Option<(String, String, bool)> {
        let root = HSTRING::from(format!("{letter}:\\"));
        unsafe {
            // (DRIVE_FIXED.)
            if GetDriveTypeW(&root) != 3 {
                return None;
            }
            let (mut label, mut system) = ([0u16; 261], [0u16; 32]);
            GetVolumeInformationW(&root, Some(&mut label), None, None, None, Some(&mut system)).ok()?;
            let text = |wide: &[u16]| String::from_utf16_lossy(wide).trim_end_matches('\0').to_string();
            let system = text(&system);
            if system != "NTFS" && system != "ReFS" {
                return None;
            }
            Some((volume(&root)?, text(&label), system == "ReFS"))
        }
    }

    /// The drives whose journals are followed, as the settings offer them (none opened).
    pub fn volumes() -> Vec<super::Volume> {
        letters()
            .filter(|&letter| crate::drives::outside(letter) == Some(false))
            .filter_map(|letter| followed(letter).map(|(volume, label, _)| super::Volume { letter, volume, label }))
            .collect()
    }

    impl Drive {
        fn at(letter: char) -> Option<Drive> {
            let (volume, _, long) = followed(letter)?;
            let root = HSTRING::from(format!("{letter}:\\"));
            unsafe {
                let handle = CreateFileW(&root, GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None).ok()?;
                let mut drive = Drive { letter, volume, root: handle, journal: 0, first: 0, next: 0, long };
                drive.query().then_some(drive)
            }
        }

        /// Its journal asked again: its number, where its records start and where the next will be; whether it has
        /// one now.
        pub fn query(&mut self) -> bool {
            let mut data = USN_JOURNAL_DATA_V0::default();
            let mut got = 0u32;
            let asked = unsafe { DeviceIoControl(self.root, FSCTL_QUERY_USN_JOURNAL, None, 0, Some((&mut data as *mut USN_JOURNAL_DATA_V0).cast()), std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32, Some(&mut got), None) };
            if asked.is_err() {
                return false;
            }
            (self.journal, self.first, self.next) = (data.UsnJournalID, data.FirstUsn, data.NextUsn);
            true
        }

        /// Where its journal is first read from: `back` bytes of records before its end (the page that place is in,
        /// where a record starts), or where its records start.
        pub fn start(&self, back: i64) -> i64 {
            ((self.next - back).div_euclid(JOURNAL_PAGE) * JOURNAL_PAGE).max(self.first)
        }

        /// What changed from `from` on (a record's place), up to where its journal ended as last asked, or `most`
        /// bytes of records: where to read on from, the changes (each file's, as it was closed), and whether that is
        /// all there was; none where it could not be read.
        pub fn read(&self, from: i64, most: usize) -> Option<(i64, Vec<super::Change>, bool)> {
            let mut at = from;
            let mut changes = Vec::new();
            let mut buffer = vec![0u8; 1 << 16];
            let mut read_bytes = 0usize;
            loop {
                if at >= self.next {
                    return Some((at, changes, true));
                }
                let ask = READ_USN_JOURNAL_DATA_V0 {
                    StartUsn: at,
                    ReasonMask: super::reasons::CREATED | super::reasons::DELETED | super::reasons::RENAMED_TO | super::reasons::CLOSED,
                    ReturnOnlyOnClose: 1,
                    Timeout: 0,
                    BytesToWaitFor: 0,
                    UsnJournalID: self.journal,
                };
                let mut got = 0u32;
                let read = unsafe {
                    DeviceIoControl(self.root, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, Some((&ask as *const READ_USN_JOURNAL_DATA_V0).cast()), std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32, Some(buffer.as_mut_ptr().cast()), buffer.len() as u32, Some(&mut got), None)
                };
                // (Its records from `at` gone meanwhile, or the journal made anew: asked again at the next turn.)
                if read.is_err() {
                    return None;
                }
                let (next, more) = super::changes_in(&buffer[..got as usize])?;
                changes.extend(more);
                read_bytes += got as usize;
                if next == at || got <= 8 {
                    return Some((next, changes, true));
                }
                at = next;
                if read_bytes >= most {
                    return Some((at, changes, at >= self.next));
                }
            }
        }

        /// The file numbered `file` on it, opened as the user: where it is (its path, when it was made, in ms since
        /// 1970, and how large it is), gone, not theirs to open, or not to be told now (locked, its drive not ready).
        pub fn look(&self, file: u128) -> Looked {
            let described = FILE_ID_DESCRIPTOR {
                dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
                Type: if self.long { ExtendedFileIdType } else { FileIdType },
                Anonymous: if self.long { FILE_ID_DESCRIPTOR_0 { ExtendedFileId: FILE_ID_128 { Identifier: file.to_le_bytes() } } } else { FILE_ID_DESCRIPTOR_0 { FileId: file as i64 } },
            };
            unsafe {
                let handle = match OpenFileById(self.root, &described, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, FILE_FLAG_BACKUP_SEMANTICS) {
                    Ok(handle) => handle,
                    // (A number no file has: the file deleted, its number's sequence moved on.)
                    Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() || error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Looked::Gone,
                    Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => return Looked::Theirs,
                    Err(_) => return Looked::Unknown,
                };
                let mut name = [0u16; 1024];
                let length = GetFinalPathNameByHandleW(handle, &mut name, FILE_NAME_NORMALIZED) as usize;
                let mut information = BY_HANDLE_FILE_INFORMATION::default();
                let told = GetFileInformationByHandle(handle, &mut information).is_ok();
                let _ = CloseHandle(handle);
                if length == 0 || length >= name.len() || !told {
                    return Looked::Unknown;
                }
                let path = String::from_utf16_lossy(&name[..length]);
                // (Written as Windows writes a path for a person: "\\?\D:\…" as "D:\…".)
                let path = PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(&path));
                let made = (u64::from(information.ftCreationTime.dwHighDateTime) << 32) | u64::from(information.ftCreationTime.dwLowDateTime);
                let bytes = (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow);
                Looked::At(path, super::ms_of(made), bytes, information.dwFileAttributes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places { downloads: Some(PathBuf::from(r"C:\Users\a\Downloads")), desktop: Some(PathBuf::from(r"C:\Users\a\Desktop")), screenshots: Some(PathBuf::from(r"C:\Users\a\Pictures\Screenshots")) }
    }

    /// Windows', programs' and tools' folders left out, and those asked; a person's own kept.
    #[test]
    fn leaves_out_what_is_not_a_persons() {
        let hidden = [PathBuf::from(r"D:\Private")];
        for path in [r"C:\Windows\Temp\a.pdf", r"C:\Program Files\x\b.png", r"C:\Users\a\AppData\Local\c.png", r"D:\code\.git\objects\d.zip", r"D:\code\node_modules\e.md", r"D:\Private\f.pdf", r"D:\proj\target\release\g.zip"] {
            assert!(left_out(Path::new(path), &hidden), "{path}");
        }
        for path in [r"C:\Users\a\Downloads\setup.exe", r"D:\Documents\report.pdf", r"D:\Windows Stuff\notes.md", r"C:\Users\a\Pictures\Screenshots\s.png"] {
            assert!(!left_out(Path::new(path), &hidden), "{path}");
        }
    }

    /// What a file is by its name and where it came: a screenshot by its folder or name, anything in Downloads (and
    /// downloaded), a program only there or on the desktop, a download in progress or a lock not at all.
    #[test]
    fn tells_what_a_file_is() {
        let places = places();
        let of = |path: &str| kind_of(Path::new(path), &places);
        assert_eq!(of(r"C:\Users\a\Pictures\Screenshots\x.png"), Some((Kind::Screenshot, false)));
        assert_eq!(of(r"D:\shots\屏幕截图 2026-10-18 154210.png"), Some((Kind::Screenshot, false)));
        assert_eq!(of(r"D:\photos\sea.JPG"), Some((Kind::Picture, false)));
        assert_eq!(of(r"C:\Users\a\Downloads\glance-setup.exe"), Some((Kind::Program, true)));
        assert_eq!(of(r"C:\Users\a\Downloads\data.bin"), Some((Kind::Other, true)));
        assert_eq!(of(r"C:\Users\a\Desktop\tool.exe"), Some((Kind::Program, false)));
        assert_eq!(of(r"D:\build\tool.exe"), None);
        assert_eq!(of(r"D:\code\main.rs"), None);
        assert_eq!(of(r"C:\Users\a\Downloads\big.zip.crdownload"), None);
        assert_eq!(of(r"D:\Documents\~$report.docx"), None);
        assert_eq!(of(r"D:\Documents\report.docx"), Some((Kind::Document, false)));
    }

    /// A download's site: the page it was taken from, else where it was; none where only its zone is said.
    #[test]
    fn reads_where_a_download_came_from() {
        assert_eq!(site_of("[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://github.com/lulu-loopp/glance/releases\r\nHostUrl=https://objects.githubusercontent.com/x\r\n").as_deref(), Some("github.com"));
        assert_eq!(site_of("[ZoneTransfer]\nZoneId=3\nHostUrl=https://www.example.com:8443/a.zip\n").as_deref(), Some("example.com"));
        assert_eq!(site_of("[ZoneTransfer]\nZoneId=3\n"), None);
        assert_eq!(site_of("[ZoneTransfer]\nZoneId=3\nHostUrl=about:internet\n"), None);
    }

    /// A journal's read: where to read on from, then records of version 2 and 3 (another passed over), with their
    /// folders.
    #[test]
    fn reads_a_journals_records() {
        let record = |major: u16, file: u128, folder: u128, reason: u32, attributes: u32, stamp: u64| {
            let mut bytes = vec![0u8; if major == 3 { 80 } else { 64 }];
            let length = bytes.len() as u32;
            bytes[..4].copy_from_slice(&length.to_le_bytes());
            bytes[4..6].copy_from_slice(&major.to_le_bytes());
            match major {
                3 => {
                    bytes[8..24].copy_from_slice(&file.to_le_bytes());
                    bytes[24..40].copy_from_slice(&folder.to_le_bytes());
                    bytes[48..56].copy_from_slice(&stamp.to_le_bytes());
                    bytes[56..60].copy_from_slice(&reason.to_le_bytes());
                    bytes[68..72].copy_from_slice(&attributes.to_le_bytes());
                }
                _ => {
                    bytes[8..16].copy_from_slice(&(file as u64).to_le_bytes());
                    bytes[16..24].copy_from_slice(&(folder as u64).to_le_bytes());
                    bytes[32..40].copy_from_slice(&stamp.to_le_bytes());
                    bytes[40..44].copy_from_slice(&reason.to_le_bytes());
                    bytes[52..56].copy_from_slice(&attributes.to_le_bytes());
                }
            }
            bytes
        };
        // 2026-10-18 00:00 UTC as a file time.
        let stamp = (1_760_745_600_000u64 + 11_644_473_600_000) * 10_000;
        let mut read = 777i64.to_le_bytes().to_vec();
        read.extend(record(2, 42, 5, 0x8000_0100, 0x20, stamp));
        read.extend(record(4, 1, 1, 0, 0, 0));
        read.extend(record(3, 1u128 << 70, 1u128 << 66, 0x8000_0200, 0, stamp));
        let (next, changes) = changes_in(&read).unwrap();
        assert_eq!(next, 777);
        assert_eq!(changes, vec![
            Change { file: 42, folder: 5, reason: 0x8000_0100, attributes: 0x20, at_ms: 1_760_745_600_000 },
            Change { file: 1u128 << 70, folder: 1u128 << 66, reason: 0x8000_0200, attributes: 0, at_ms: 1_760_745_600_000 },
        ]);
    }

    /// The list as changes come: a file made lately listed (with its site), one made long ago (moved, renamed) not, one
    /// deleted taken off, one renamed followed to its new name, one renamed into a folder left out taken off.
    #[test]
    fn follows_files_as_they_come_and_go() {
        let now = 1_760_800_000_000;
        let places = places();
        let change = |file: u128, reason: u32| Change { file, folder: 9, reason, attributes: 0x20, at_ms: now };
        let mut kept = Kept::default();
        let open = |path: &'static str, made: u64| move |_: u128| Looked::At(PathBuf::from(path), made, 10, 0);
        let site = |_: &Path| Some("github.com".to_string());
        let out = |path: &Path| left_out(path, &[]);
        assert!(kept.take('C', change(1, 0x8000_0100), now, &places, &out, open(r"C:\Users\a\Downloads\a.zip", now - 1000), site));
        assert_eq!((kept.files[0].kind, kept.files[0].downloaded, kept.files[0].site.as_deref()), (Kind::Archive, true, Some("github.com")));
        // Moved here, made long ago: not one come.
        assert!(!kept.take('D', change(2, 0x8000_2000), now, &places, &out, open(r"D:\Documents\old.pdf", now - KEEP_MS - 1), |_| None));
        // Not yet closed: nothing yet.
        assert!(!kept.take('D', change(3, 0x100), now, &places, &out, open(r"D:\Documents\new.pdf", now), |_| None));
        assert!(kept.take('D', change(3, 0x8000_0100), now, &places, &out, open(r"D:\Documents\new.pdf", now - 10), |_| None));
        assert_eq!(kept.files.iter().map(|came| came.key.file).collect::<Vec<_>>(), [3, 1]);
        // Renamed: followed.
        assert!(kept.take('D', change(3, 0x8000_2000), now, &places, &out, open(r"D:\Documents\renamed.pdf", now - 10), |_| None));
        assert_eq!(kept.files[0].path, PathBuf::from(r"D:\Documents\renamed.pdf"));
        // Renamed into a folder left out: taken off.
        assert!(kept.take('D', change(3, 0x8000_2000), now, &places, &out, open(r"D:\Documents\.trash\renamed.pdf", now - 10), |_| None));
        // Deleted: taken off.
        assert!(kept.take('C', change(1, 0x8000_0200), now, &places, &out, |_| Looked::Gone, |_| None));
        assert!(kept.files.is_empty());
        // A folder, a hidden file: never.
        assert!(!kept.take('D', Change { attributes: 0x10, ..change(4, 0x8000_0100) }, now, &places, &out, open(r"D:\Documents\x.pdf", now), |_| None));
        assert!(!kept.take('D', Change { attributes: 0x2, ..change(5, 0x8000_0100) }, now, &places, &out, open(r"D:\Documents\y.pdf", now), |_| None));
        // A change older than the days kept: its file never opened.
        assert!(!kept.take('D', Change { at_ms: now - KEEP_MS - 1, ..change(6, 0x8000_0100) }, now, &places, &out, |_| panic!("opened"), |_| None));
    }

    /// The same number on two drives: two files, one deleted leaving the other listed.
    #[test]
    fn knows_a_file_by_its_drive_and_number() {
        let now = 1_760_800_000_000;
        let places = places();
        let change = |reason: u32| Change { file: 7, folder: 9, reason, attributes: 0x20, at_ms: now };
        let out = |path: &Path| left_out(path, &[]);
        let mut kept = Kept::default();
        assert!(kept.take('C', change(0x8000_0100), now, &places, &out, |_| Looked::At(PathBuf::from(r"C:\Users\a\Downloads\c.zip"), now - 20, 1, 0), |_| None));
        assert!(kept.take('D', change(0x8000_0100), now, &places, &out, |_| Looked::At(PathBuf::from(r"D:\Documents\d.pdf"), now - 10, 1, 0), |_| None));
        assert_eq!(kept.files.len(), 2);
        assert!(kept.take('D', change(0x8000_0200), now, &places, &out, |_| Looked::Gone, |_| None));
        assert_eq!(kept.files.iter().map(|came| came.path.clone()).collect::<Vec<_>>(), [PathBuf::from(r"C:\Users\a\Downloads\c.zip")]);
    }

    /// A file made and written again within one read: made, as one change (its reasons together).
    #[test]
    fn takes_a_files_changes_together() {
        let change = |file: u128, reason: u32, at_ms: u64| Change { file, folder: 9, reason, attributes: 0x20, at_ms };
        let together = together(vec![change(1, 0x8000_0102, 1), change(2, 0x8000_0100, 2), change(1, 0x8000_0002, 3)]);
        assert_eq!(together, vec![change(1, 0x8000_0102, 3), change(2, 0x8000_0100, 2)]);
    }

    /// Each file listed looked for by its number: one where it was kept; one moved with its folder followed (another
    /// file put where it was not taken for it); one gone taken off; one whose drive is away, or locked, kept; one now
    /// in a tool's folder taken off; one moved out of Downloads told again where it is (a program left there: no kind
    /// listed outside Downloads and the desktop, taken off; a picture: no longer downloaded); one written again, as
    /// large as it is now.
    #[test]
    fn looks_for_each_file_again() {
        let came = |drive: char, file: u128, path: &str, kind: Kind, downloaded: bool| Came { key: Key { drive, file }, path: PathBuf::from(path), came_ms: 1, bytes: 1, kind, downloaded, site: None };
        let mut kept = Kept {
            files: vec![
                came('D', 1, r"D:\a\one.pdf", Kind::Document, false),
                came('D', 2, r"D:\b\two.pdf", Kind::Document, false),
                came('D', 3, r"D:\c\three.pdf", Kind::Document, false),
                came('E', 4, r"E:\four.pdf", Kind::Document, false),
                came('D', 5, r"D:\d\five.pdf", Kind::Document, false),
                came('D', 6, r"D:\repo\six.pdf", Kind::Document, false),
                came('C', 7, r"C:\Users\a\Downloads\tool.exe", Kind::Program, true),
                came('C', 8, r"C:\Users\a\Downloads\sea.png", Kind::Picture, true),
            ],
            ..Kept::default()
        };
        let look = |key: Key| match (key.drive, key.file) {
            ('D', 1) => Looked::At(PathBuf::from(r"D:\a\one.pdf"), 1, 900, 0),
            ('D', 2) => Looked::At(PathBuf::from(r"D:\b renamed\two.pdf"), 1, 1, 0),
            ('D', 3) => Looked::Gone,
            ('D', 5) => Looked::Unknown,
            ('D', 6) => Looked::At(PathBuf::from(r"D:\repo\six.pdf"), 1, 1, 0),
            ('C', 7) => Looked::At(PathBuf::from(r"D:\Tools\tool.exe"), 1, 1, 0),
            ('C', 8) => Looked::At(PathBuf::from(r"D:\Photos\sea.png"), 1, 1, 0),
            _ => Looked::Unknown,
        };
        assert!(kept.recheck(&places(), look, &|path: &Path| path.starts_with(r"D:\repo"), |_| true));
        assert_eq!(kept.files.iter().map(|came| came.path.to_str().unwrap()).collect::<Vec<_>>(), [r"D:\a\one.pdf", r"D:\b renamed\two.pdf", r"E:\four.pdf", r"D:\d\five.pdf", r"D:\Photos\sea.png"]);
        assert_eq!(kept.files[0].bytes, 900);
        assert!(!kept.files[4].downloaded);
        assert!(!kept.recheck(&places(), |_| Looked::Unknown, &|_| false, |_| true));
    }

    /// A folder renamed where all in it is left out (in the user's temporary files, or into them) changes nothing
    /// asked of other folders; one renamed where its files may be listed does, and so does a repository's `.git` put
    /// anywhere (the folder it is in is now a tool's), and one not to be looked at now. As it does not, only the files
    /// listed that are not where they were are looked for again.
    #[test]
    fn a_folder_renamed_where_all_is_left_out_changes_no_other() {
        let at = |path: &str| Looked::At(PathBuf::from(path), 1, 0, reasons::FOLDER);
        let out = |path: &Path| left_out(path, &[]);
        assert!(moved_where_left_out(&at(r"C:\Users\a\AppData\Local\Temp\run-41"), out));
        assert!(moved_where_left_out(&at(r"D:\work\target\debug"), out));
        assert!(!moved_where_left_out(&at(r"D:\Documents\Reports 2026"), out));
        assert!(!moved_where_left_out(&at(r"D:\Documents\site\.git"), out));
        assert!(!moved_where_left_out(&at(r"D:\Documents\site\.GIT"), out));
        assert!(!moved_where_left_out(&Looked::Gone, out));
        assert!(!moved_where_left_out(&Looked::Unknown, out));

        let came = |file: u128, path: &str| Came { key: Key { drive: 'D', file }, path: PathBuf::from(path), came_ms: 1, bytes: 1, kind: Kind::Document, downloaded: false, site: None };
        let mut kept = Kept { files: vec![came(1, r"D:\stays\one.pdf"), came(2, r"D:\went\two.pdf")], ..Kept::default() };
        let looked = std::cell::RefCell::new(Vec::new());
        let look = |key: Key| {
            looked.borrow_mut().push(key.file);
            Looked::At(PathBuf::from(r"D:\went on\two.pdf"), 1, 1, 0)
        };
        assert!(kept.recheck(&places(), look, &|_| false, |came| came.path.starts_with(r"D:\went")));
        assert_eq!(*looked.borrow(), [2]);
        assert_eq!(kept.files.iter().map(|came| came.path.to_str().unwrap()).collect::<Vec<_>>(), [r"D:\stays\one.pdf", r"D:\went on\two.pdf"]);
    }

    /// A file come that could not be opened as it was read (locked, its drive not ready): tried again till it can be,
    /// and listed; one never to be opened let go of after its tries; one not the user's to open never tried again.
    #[test]
    fn tries_again_a_file_it_could_not_open() {
        let now = 1_760_800_000_000;
        let places = places();
        let change = |file: u128| Change { file, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now };
        let out = |path: &Path| left_out(path, &[]);
        let mut kept = Kept::default();
        assert!(!kept.take('D', change(1), now, &places, &out, |_| Looked::Unknown, |_| None));
        assert!(!kept.take('D', change(2), now, &places, &out, |_| Looked::Unknown, |_| None));
        assert!(!kept.take('D', change(3), now, &places, &out, |_| Looked::Theirs, |_| None));
        assert_eq!(kept.later.len(), 2);
        // Kept with where the journal was read up to.
        let kept_again: Kept = serde_json::from_slice(&serde_json::to_vec(&kept).unwrap()).unwrap();
        assert_eq!(kept_again.later.len(), 2);
        let open = |_: char, file: u128| if file == 1 { Looked::At(PathBuf::from(r"D:\Documents\one.pdf"), now - 10, 5, 0) } else { Looked::Unknown };
        assert!(kept.retry(now, &places, &|_, path| out(path), open, |_| None));
        assert_eq!(kept.files.iter().map(|came| came.key.file).collect::<Vec<_>>(), [1]);
        assert_eq!(kept.later.iter().map(|(_, change, tries)| (change.file, *tries)).collect::<Vec<_>>(), [(2, 1)]);
        for _ in 1..LATER_TRIES {
            kept.retry(now, &places, &|_, path| out(path), open, |_| None);
        }
        assert!(kept.later.is_empty());
    }

    /// A file listed, written again (no longer made or renamed): as large as it is now.
    #[test]
    fn follows_a_listed_file_as_it_is_written() {
        let now = 1_760_800_000_000;
        let places = places();
        let out = |path: &Path| left_out(path, &[]);
        let mut kept = Kept::default();
        let open = |bytes: u64| move |_: u128| Looked::At(PathBuf::from(r"D:\Documents\a.txt"), now - 100, bytes, 0);
        assert!(kept.take('D', Change { file: 1, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now }, now, &places, &out, open(5), |_| None));
        // Written again (data extended, closed).
        assert!(kept.take('D', Change { file: 1, folder: 9, reason: 0x8000_0002, attributes: 0x20, at_ms: now }, now, &places, &out, open(500), |_| None));
        assert_eq!(kept.files[0].bytes, 500);
        // Another file, not listed, written: not opened.
        assert!(!kept.take('D', Change { file: 2, folder: 9, reason: 0x8000_0002, attributes: 0x20, at_ms: now }, now, &places, &out, |_| panic!("opened"), |_| None));
        // Made hidden (its attributes changed, closed): no longer listed.
        assert!(kept.take('D', Change { file: 1, folder: 9, reason: 0x8000_8000, attributes: 0x22, at_ms: now }, now, &places, &out, open(500), |_| None));
        assert!(kept.files.is_empty());
        // Made hidden while the journal was not read: taken off as it is looked for again.
        assert!(kept.take('D', Change { file: 1, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now }, now, &places, &out, open(5), |_| None));
        assert!(kept.recheck(&places, |_| Looked::At(PathBuf::from(r"D:\Documents\a.txt"), now - 100, 5, 0x22), &out, |_| true));
        assert!(kept.files.is_empty());
    }

    /// A file in a repository's working tree or a program's folder, however deep: a tool's; beside them, or in
    /// Downloads holding a program: not. The folders above asked first (one held: none below it looked at), and
    /// Downloads never looked through for a program.
    #[test]
    fn knows_tools_files() {
        let looked: std::cell::RefCell<Vec<(PathBuf, bool)>> = Default::default();
        let looks = |folder: &Path, programs: bool| {
            looked.borrow_mut().push((folder.to_path_buf(), programs));
            folder == Path::new("D:/repo") || programs && (folder == Path::new("D:/app") || folder == Path::new("C:/Users/a/Downloads"))
        };
        let tools = Tools::new(&Places { downloads: Some(PathBuf::from("C:/Users/a/Downloads")), desktop: Some(PathBuf::from("C:/Users/a/Desktop")), screenshots: None }, looks);
        assert!(tools.hold(Path::new("D:/repo/docs/deep/a.md")));
        assert!(!looked.borrow().iter().any(|(folder, _)| folder.starts_with("D:/repo/docs")));
        assert!(tools.hold(Path::new("D:/repo/b.md")));
        assert!(tools.hold(Path::new("D:/app/skins/icons/c.png")));
        assert!(!tools.hold(Path::new("D:/loose/d.md")));
        assert!(!tools.hold(Path::new("C:/Users/a/Downloads/e.pdf")));
        assert!(looked.borrow().iter().any(|(folder, programs)| folder == Path::new("C:/Users/a/Downloads") && !programs));
    }

    /// What the user asked of the list: none in a folder not listed (its name told without case), none on a drive
    /// left out (by its volume, whatever its letter), and with some folders chosen, only theirs and their folders'
    /// (one not listed inside them still not, and one named as they begin not theirs). With none chosen, nothing.
    #[test]
    fn shows_what_the_user_asked() {
        let left = r"\\?\Volume{e}\";
        let scope = Scope { hidden: vec![PathBuf::from(r"D:\Photos\Private")], drives: vec![left.to_string()], only: Some(vec![PathBuf::from(r"d:\photos"), PathBuf::from(r"C:\Users\a\Downloads")]) };
        let takes = |scope: &Scope, path: &str, volume: Option<&str>| scope.takes(Path::new(path), volume);
        assert!(takes(&scope, r"D:\Photos\a.jpg", Some("d")));
        assert!(takes(&scope, r"D:\Photos\2026\b.jpg", Some("d")));
        assert!(takes(&scope, r"C:\Users\a\Downloads\f.zip", None));
        assert!(!takes(&scope, r"D:\Photos\Private\c.jpg", Some("d")));
        assert!(!takes(&scope, r"D:\photos\PRIVATE\deeper\c.jpg", Some("d")));
        assert!(!takes(&scope, r"D:\Photoshop\d.psd", Some("d")));
        assert!(!takes(&scope, r"D:\Documents\e.pdf", Some("d")));
        assert!(!takes(&scope, r"D:\Photos\a.jpg", Some(left)));
        let every = Scope { only: None, ..scope.clone() };
        assert!(takes(&every, r"D:\Documents\e.pdf", Some("d")));
        assert!(!takes(&every, r"F:\Documents\e.pdf", Some(left)));
        assert!(takes(&every, r"F:\Documents\e.pdf", None));
        assert!(!scope.nowhere() && !every.nowhere());
        let none = Scope { only: Some(Vec::new()), ..Scope::default() };
        assert!(none.nowhere());
        assert!(!takes(&none, r"D:\Photos\a.jpg", None));
    }

    /// The settings as they change: the list on with what it shows, carried to its thread (whether nothing can be
    /// shown, told at once). Off, none.
    #[test]
    fn takes_what_the_settings_ask() {
        let mut settings = crate::settings::Settings::default();
        settings.recent_only = true;
        settings.recent_hidden.push(PathBuf::from(r"D:\Private"));
        settings.recent_hidden_drives.push(crate::settings::HiddenDrive { volume: "v".into(), label: "Data".into(), letter: 'E' });
        follow_settings(&settings);
        assert_eq!(*WANTED.0.lock().unwrap(), Some(Scope { hidden: vec![PathBuf::from(r"D:\Private")], drives: vec!["v".into()], only: Some(Vec::new()) }));
        assert!(nowhere());
        settings.recent_folders.push(PathBuf::from(r"D:\Photos"));
        follow_settings(&settings);
        assert!(!nowhere());
        settings.station.recent = false;
        follow_settings(&settings);
        assert_eq!(*WANTED.0.lock().unwrap(), None);
    }

    /// A file kept is shown as the user asks now: not while its drive is left out, and again as it is listed again,
    /// its drive told by the volume its letter last had (an external disk away and left out: its files still not
    /// shown, another given its letter: told by that one, and kept so between runs). One come where nothing is shown
    /// is not kept at all (a thousand photos would else fill the list).
    #[test]
    fn keeps_and_shows_only_what_is_asked() {
        let now = 1_760_800_000_000;
        let places = places();
        let change = |file: u128| Change { file, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now };
        let open = |path: &'static str| move |_: u128| Looked::At(PathBuf::from(path), now - 10, 1, 0);
        let mut kept = Kept::default();
        kept.saw([('D', "vol-d"), ('E', "vol-e")]);
        let every = Scope::default();
        let volumes = kept.volumes.clone();
        let out_by = |scope: &Scope, letter: char| {
            let (scope, volumes) = (scope.clone(), volumes.clone());
            move |path: &Path| left_out(path, &[]) || outside(&scope, &volumes, letter, path)
        };
        assert!(kept.take('D', change(1), now, &places, &out_by(&every, 'D'), open(r"D:\Documents\a.pdf"), |_| None));
        assert!(kept.take('E', change(2), now, &places, &out_by(&every, 'E'), open(r"E:\Photos\b.jpg"), |_| None));
        let paths = |shown: Vec<Came>| shown.into_iter().map(|came| came.path).collect::<Vec<_>>();
        let left = Scope { drives: vec!["vol-e".into()], ..Scope::default() };
        assert_eq!(paths(kept.shown(&left)), [PathBuf::from(r"D:\Documents\a.pdf")]);
        assert_eq!(kept.shown(&every).len(), 2);
        // Come while its drive is left out: not kept.
        assert!(!kept.take('E', change(3), now, &places, &out_by(&left, 'E'), open(r"E:\Photos\c.jpg"), |_| None));
        assert_eq!(kept.files.len(), 2);
        // Only some folders: one come outside them not kept, one in them kept.
        let only = Scope { only: Some(vec![PathBuf::from(r"D:\Documents")]), ..Scope::default() };
        assert!(!kept.take('D', change(4), now, &places, &out_by(&only, 'D'), open(r"D:\Pictures\d.png"), |_| None));
        assert!(kept.take('D', change(5), now, &places, &out_by(&only, 'D'), open(r"D:\Documents\Work\e.pdf"), |_| None));
        assert_eq!(paths(kept.shown(&only)), [PathBuf::from(r"D:\Documents\a.pdf"), PathBuf::from(r"D:\Documents\Work\e.pdf")]);
        // Its drive away (looked for, not seen): told by the volume it had.
        kept.saw([('D', "vol-d")]);
        assert_eq!(kept.shown(&left).len(), 2);
        let again: Kept = serde_json::from_slice(&serde_json::to_vec(&kept).unwrap()).unwrap();
        assert_eq!(again.shown(&left).len(), 2);
        // Another volume given its letter: told by that one.
        kept.saw([('E', "vol-x")]);
        assert_eq!(kept.shown(&left).len(), 3);
    }

    /// A file come that could not be opened, tried again: told by the drive it is on (left out there, not listed).
    #[test]
    fn tries_again_on_the_drive_a_file_is_on() {
        let now = 1_760_800_000_000;
        let places = places();
        let change = |file: u128| Change { file, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now };
        let mut kept = Kept::default();
        assert!(!kept.take('D', change(1), now, &places, &|_| false, |_| Looked::Unknown, |_| None));
        assert!(!kept.take('E', change(1), now, &places, &|_| false, |_| Looked::Unknown, |_| None));
        let open = |drive: char, _: u128| Looked::At(PathBuf::from(format!(r"{drive}:\Documents\one.pdf")), now - 10, 5, 0);
        assert!(kept.retry(now, &places, &|drive, _| drive == 'E', open, |_| None));
        assert_eq!(kept.files.iter().map(|came| came.key.drive).collect::<Vec<_>>(), ['D']);
    }

    /// What was listed from a drive plugged in from outside is let go of, with what waits to be tried there and where
    /// its journal was read up to; the PC's own drives' kept, one once at the same letter too.
    #[test]
    fn lets_go_of_what_came_on_a_drive_from_outside() {
        let now = 1_760_800_000_000;
        let mut kept = Kept::default();
        kept.files = vec![
            Came { key: Key { drive: 'C', file: 1 }, ..come(1, r"C:\Users\a\Downloads\a.zip", now, 5, Kind::Archive) },
            Came { key: Key { drive: 'G', file: 2 }, ..come(2, r"G:\bak\b.zip", now, 5, Kind::Archive) },
            Came { key: Key { drive: 'D', file: 4 }, ..come(4, r"D:\Documents\d.pdf", now, 5, Kind::Document) },
        ];
        kept.later = vec![('G', Change { file: 3, folder: 9, reason: 0x8000_0100, attributes: 0x20, at_ms: now }, 0)];
        kept.read = vec![('C', 1, 100), ('G', 2, 200), ('D', 3, 300)];
        kept.saw([('C', "vol-c"), ('G', "vol-usb"), ('D', "vol-own")]);
        // (D: is now another drive, from outside: what was its own stays.)
        assert!(kept.let_go(&[('G', "vol-usb".into()), ('D', "vol-other".into())]));
        assert_eq!(kept.files.iter().map(|came| came.key.drive).collect::<Vec<_>>(), ['C', 'D']);
        assert!(kept.later.is_empty());
        assert_eq!(kept.read, [('C', 1, 100), ('D', 3, 300)]);
        assert!(!kept.let_go(&[('G', "vol-usb".into())]));

        // Only what waits there, or where it was read up to: let go of, and said so (to be kept so).
        kept.read.push(('G', 2, 200));
        kept.saw([('G', "vol-usb")]);
        assert!(kept.let_go(&[('G', "vol-usb".into())]));
        assert_eq!(kept.read, [('C', 1, 100), ('D', 3, 300)]);
    }

    /// A file come, as the bursts' rule takes it: its folder, its name, when it came (ms), how large it is, what it is.
    fn come(file: u128, path: &str, came_ms: u64, bytes: u64, kind: Kind) -> Came {
        Came { key: Key { drive: 'D', file }, path: PathBuf::from(path), came_ms, bytes, kind, downloaded: false, site: None }
    }

    /// `files` newest first, as the list keeps them.
    fn newest_first(mut files: Vec<Came>) -> Vec<Came> {
        files.sort_by_key(|came| std::cmp::Reverse(came.came_ms));
        files
    }

    /// The rows `listed` makes, each by its files' numbers.
    fn rows_of(files: &[Came], filter: Filter) -> Vec<Vec<u128>> {
        listed(files, filter).into_iter().map(|row| row.into_iter().map(|came| came.key.file).collect()).collect()
    }

    /// Files come into one folder, each begun within five seconds of the one before past the time it took to write
    /// at ten MB a second, five or more: one row, the newest first, standing where the newest would, a file of
    /// another folder come meanwhile a row of its own. A moment later than that, or four: each its own row.
    #[test]
    fn shows_a_burst_as_one_row() {
        let t = 1_760_800_000_000;
        // 4 MB each: the next within 5.4 s.
        let photos = |count: u128, last_gap: u64| -> Vec<Came> {
            let mut at = t;
            (1..=count).map(|file| {
                let came = come(file, &format!(r"D:\Phone\DCIM\{file}.jpg"), at, 4_000_000, Kind::Picture);
                at += if file + 1 == count { last_gap } else { 5_400 };
                came
            }).collect()
        };
        let mut files = photos(5, 5_400);
        files.push(come(9, r"D:\Documents\a.pdf", t + 7_000, 1, Kind::Document));
        files.push(come(10, r"D:\Documents\b.pdf", t + 60_000, 1, Kind::Document));
        let files = newest_first(files);
        assert_eq!(rows_of(&files, Filter::All), vec![vec![10], vec![5, 4, 3, 2, 1], vec![9]]);
        assert_eq!(rows_of(&newest_first(photos(5, 5_401)), Filter::All), vec![vec![5], vec![4], vec![3], vec![2], vec![1]]);
        assert_eq!(rows_of(&newest_first(photos(4, 5_400)), Filter::All), vec![vec![4], vec![3], vec![2], vec![1]]);
    }

    /// A large file keeps the burst for as long as it took to write (a 2 GB video at ten MB a second: 205 s, and the
    /// five seconds), its folder's name told without case, a folder within it a burst of its own.
    #[test]
    fn a_burst_waits_for_a_large_file() {
        let t = 1_760_800_000_000;
        let video = 2_050_000_000;
        let files = |gap: u64| newest_first(vec![
            come(1, r"D:\Phone\DCIM\a.mp4", t, video, Kind::Video),
            come(2, r"D:\phone\dcim\b.jpg", t + gap, 1, Kind::Picture),
            come(3, r"D:\Phone\DCIM\c.jpg", t + gap + 1, 1, Kind::Picture),
            come(4, r"D:\Phone\DCIM\d.jpg", t + gap + 2, 1, Kind::Picture),
            come(5, r"D:\Phone\DCIM\e.jpg", t + gap + 3, 1, Kind::Picture),
            come(6, r"D:\Phone\DCIM\Sub\f.jpg", t + gap + 4, 1, Kind::Picture),
        ]);
        assert_eq!(rows_of(&files(210_000), Filter::All), vec![vec![6], vec![5, 4, 3, 2, 1]]);
        assert_eq!(rows_of(&files(210_001), Filter::All), vec![vec![6], vec![5], vec![4], vec![3], vec![2], vec![1]]);
    }

    /// A burst is found among all the files, then shown as the page's filter takes it: one the filter leaves out does
    /// not break it (a video between documents), and where the filter takes fewer than five of it, each is a row.
    #[test]
    fn a_burst_is_shown_as_the_filter_takes_it() {
        let t = 1_760_800_000_000;
        let files = newest_first(vec![
            come(1, r"D:\Unpacked\a.pdf", t, 1, Kind::Document),
            come(2, r"D:\Unpacked\b.pdf", t + 1_000, 1, Kind::Document),
            come(3, r"D:\Unpacked\c.mp4", t + 2_000, 2_050_000_000, Kind::Video),
            come(4, r"D:\Unpacked\d.pdf", t + 200_000, 1, Kind::Document),
            come(5, r"D:\Unpacked\e.pdf", t + 201_000, 1, Kind::Document),
            come(6, r"D:\Unpacked\f.pdf", t + 202_000, 1, Kind::Document),
            come(7, r"D:\Unpacked\g.png", t + 203_000, 1, Kind::Picture),
        ]);
        assert_eq!(rows_of(&files, Filter::All), vec![vec![7, 6, 5, 4, 3, 2, 1]]);
        assert_eq!(rows_of(&files, Filter::Documents), vec![vec![6, 5, 4, 2, 1]]);
        assert_eq!(rows_of(&files, Filter::Pictures), vec![vec![7], vec![3]]);
    }

    /// A row pressed, dragged or opened finds its files as the list is now: a file its own, files come together all
    /// of them the page's filter takes, the newest first, by any of them (one more come since: with it), none gone.
    #[test]
    fn a_row_finds_its_files() {
        let t = 1_760_800_000_000;
        let mut files: Vec<Came> = (1..=5).map(|file| come(file, &format!(r"D:\Phone\{file}.jpg"), t + file as u64 * 1_000, 1, Kind::Picture)).collect();
        files.push(come(9, r"D:\Documents\a.pdf", t, 1, Kind::Document));
        pretend(newest_first(files.clone()));
        let paths = |names: &[&str]| Some(names.iter().map(|name| PathBuf::from(*name)).collect::<Vec<_>>());
        assert_eq!(paths_of(Entry::File(Key { drive: 'D', file: 9 })), paths(&[r"D:\Documents\a.pdf"]));
        let burst = paths(&[r"D:\Phone\5.jpg", r"D:\Phone\4.jpg", r"D:\Phone\3.jpg", r"D:\Phone\2.jpg", r"D:\Phone\1.jpg"]);
        assert_eq!(paths_of(Entry::Burst(Filter::All, Key { drive: 'D', file: 5 })), burst);
        assert_eq!(paths_of(Entry::Burst(Filter::Pictures, Key { drive: 'D', file: 2 })), burst);
        files.push(come(6, r"D:\Phone\6.jpg", t + 6_000, 1, Kind::Picture));
        pretend(newest_first(files));
        assert_eq!(paths_of(Entry::Burst(Filter::All, Key { drive: 'D', file: 5 })).map(|paths| paths.len()), Some(6));
        assert_eq!(paths_of(Entry::File(Key { drive: 'D', file: 7 })), None);
    }

    /// The list read as Windows keeps the user's files: an ignored test that lists what it finds now (it reads this
    /// PC's drives, as the user), read back from a place within a busy journal (not a record's: the page it is in).
    #[test]
    #[ignore = "reads this PC's drives"]
    fn lists_what_came_lately_here() {
        let places = windows_places();
        let now = crate::station::now_ms();
        let mut kept = Kept::default();
        let tools = Tools::new(&places, looked_at);
        for drive in os::drives().0 {
            let mut from = drive.start(64 << 20);
            let mut batches = 0;
            loop {
                let (to, changes, done) = drive.read(from, BATCH).expect("its journal read");
                for change in together(changes) {
                    kept.take(drive.letter, change, now, &places, &|path| left_out(path, &[]) || tools.hold(path), |file| drive.look(file), |path| std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).ok().and_then(|text| site_of(&text)));
                }
                batches += 1;
                from = to;
                if done {
                    break;
                }
            }
            println!("{}: {batches} batches", drive.letter);
            // Read back from within: the page that place is in, its first record.
            assert!(drive.read(drive.start(4096 + 8), BATCH).is_some(), "{} read from within", drive.letter);
        }
        for came in kept.files.iter().take(40) {
            println!("{:?} {:?} {:?} {}", came.kind, came.site, came.path, crate::ui::text::since(came.came_ms, now, crate::ui::text::Lang::Zh));
        }
        assert!(!kept.files.is_empty());
    }
}
