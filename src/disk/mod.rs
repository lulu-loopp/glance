//! What a drive holds, as the disk organizer shows it (see `ui::disk`): read from the drive's search index (see
//! `search`), which already knows every file on it, its size and when it was written, so nothing is scanned.
//!
//! A drive is surveyed whole once (`Survey::take`): every folder's size with all it holds, how much of that is of
//! each kind, when anything in it was written last, its largest file (and its largest written over a year ago), and
//! whether it holds anything not to be removed. The folder shown is then copied out of the index as far as the window
//! can draw it (`Tree`), so the map is laid out and moved without the index (see `map`); the lists beside it are read
//! from the index for that folder, as filtered (`Detail`). Each is made on a thread of its own, the index held from
//! changing meanwhile, and made again as the drive changes (see `search::read_since`).

use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};

use crate::search::index::{flag, Index, ROOT};

pub mod act;
pub mod map;

/// A map and a set hashed as `Quick` hashes: what a drive's every folder and file is looked up in as it is surveyed
/// and listed (by its place, its extension).
pub type QuickMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<Quick>>;
pub type QuickSet<K> = std::collections::HashSet<K, BuildHasherDefault<Quick>>;

/// A hash quick to take of a number or a short name (as the Rust compiler's own is taken): each word of it added to
/// what was taken, mixed by a multiplication. Not to be used of what anyone outside may choose.
#[derive(Clone, Copy, Default)]
pub struct Quick(u64);

impl Quick {
    fn add(&mut self, word: u64) {
        self.0 = (self.0 ^ word).wrapping_mul(0xF135_7AEA_2E62_A9C5);
    }
}

impl Hasher for Quick {
    fn write(&mut self, bytes: &[u8]) {
        let mut words = bytes.chunks_exact(8);
        for word in &mut words {
            self.add(u64::from_le_bytes(word.try_into().unwrap()));
        }
        let mut last = [0; 8];
        let rest = words.remainder();
        last[..rest.len()].copy_from_slice(rest);
        self.add(u64::from_le_bytes(last) ^ (bytes.len() as u64) << 56);
    }

    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }

    fn write_u32(&mut self, n: u32) {
        self.add(n as u64);
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    /// (The multiplication leaves its best mixed bits highest: turned to where a table takes its buckets from.)
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }
}

/// What a file is for, as the map colours it: as Explorer's own icons colour videos, pictures, music and documents,
/// and Fluent's colours archives and disk images, programs and code, and games; the system's, and the rest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Video,
    Picture,
    Music,
    Document,
    Archive,
    Program,
    Game,
    System,
    Other,
}

/// Every kind, in the order the legend lists them.
pub const KINDS: [Kind; 9] = [Kind::Video, Kind::Picture, Kind::Music, Kind::Document, Kind::Archive, Kind::Program, Kind::Game, Kind::System, Kind::Other];

impl Kind {
    pub fn at(self) -> usize {
        self as usize
    }
}

/// The extensions of each kind told by its extension alone (games and the system's are told by where they are).
const EXTENSIONS: [(Kind, &[&str]); 7] = [
    (Kind::Video, &["mp4", "mkv", "mov", "avi", "wmv", "webm", "flv", "m4v", "mpg", "mpeg", "m2ts", "mts", "3gp", "vob", "rmvb", "rm", "f4v", "ogv"]),
    (Kind::Picture, &["jpg", "jpeg", "png", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "avif", "jxl", "raw", "arw", "cr2", "cr3", "nef", "dng", "orf", "rw2", "raf", "psd", "svg", "ico"]),
    (Kind::Music, &["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma", "ape", "aiff", "aif", "mid", "midi", "dsf", "dff"]),
    (Kind::Document, &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "epub", "mobi", "azw3", "csv", "log", "tex", "xps", "one", "pages", "key", "numbers"]),
    (Kind::Archive, &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz4", "iso", "img", "vhd", "vhdx", "vmdk", "vdi", "qcow2", "wim", "esd", "cab", "dmg", "mrimg", "vmgs", "vmem", "vmsn", "tib", "gho"]),
    (
        Kind::Program,
        &[
            "exe", "dll", "msi", "msix", "msixbundle", "appx", "appxbundle", "sys", "lib", "a", "so", "dylib", "jar", "class", "pdb", "obj", "o", "rlib", "rmeta", "pyc", "pyd", "whl", "js", "mjs", "cjs", "ts",
            "tsx", "jsx", "py", "rs", "c", "cc", "cpp", "h", "hpp", "cs", "java", "go", "kt", "swift", "json", "yaml", "yml", "toml", "xml", "wasm", "apk", "nupkg", "ps1", "bat", "cmd", "sh", "node", "ipynb",
        ],
    ),
    (Kind::Game, &["pak", "vpk", "bsa", "ba2", "uasset", "umap", "ucas", "utoc"]),
];

/// The kind of the file named `name` (case folded), by its extension.
pub fn kind_of(name: &str) -> Kind {
    // (Every file of a drive is told so as it is surveyed and as it is listed: the extensions looked up at once.)
    static KIND_OF: std::sync::OnceLock<QuickMap<&'static str, Kind>> = std::sync::OnceLock::new();
    let kinds = KIND_OF.get_or_init(|| EXTENSIONS.iter().flat_map(|(kind, extensions)| extensions.iter().map(move |extension| (*extension, *kind))).collect());
    kinds.get(extension_of(name)).copied().unwrap_or(Kind::Other)
}

/// The extension of the file named `name` (case folded), as the list of extensions groups files: none where it has none
/// (or what follows its last dot is too long to be one).
pub fn extension_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() && extension.len() <= 12 && !extension.contains(' ') => extension,
        _ => "",
    }
}

/// The folders whose names say a game library is in them (case folded): what is in them is a game's.
const GAME_LIBRARIES: [&str; 9] = ["steamapps", "steamlibrary", "epic games", "xboxgames", "riot games", "gog games", "ea games", "wegameapps", "games"];

/// What a folder is to the organizer: as any, one not to be removed whole (the drive's root, the system's and the
/// user's own folders: what is in them may be), the system's (nothing in it removed), the Recycle Bin, or one whose
/// names this user may not list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Plain,
    Kept,
    System,
    Bin,
    Unread,
}

/// The places on this PC that are not to be removed whole: Windows' folder (what is in it is the system's, but its
/// Temp), and the folders Windows and each user keep (Program Files, the users' profiles and their own folders);
/// and the OneDrive folders, whose files may be left in the cloud alone (see `act::release`).
#[derive(Clone, Debug, Default)]
pub struct Places {
    pub windows: Option<PathBuf>,
    pub kept: Vec<PathBuf>,
    pub onedrive: Vec<PathBuf>,
}

impl Places {
    /// As this PC has them.
    pub fn of_this_pc() -> Places {
        use windows::Win32::UI::Shell::{
            FOLDERID_AppDataProgramData, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData, FOLDERID_LocalAppDataLow, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile,
            FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_UserProfiles, FOLDERID_Videos, FOLDERID_Windows, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
        };
        let known = |id: &windows::core::GUID| unsafe {
            let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
            let read = path.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
            read.map(PathBuf::from)
        };
        let kept = [
            &FOLDERID_ProgramFiles,
            &FOLDERID_ProgramFilesX86,
            &FOLDERID_ProgramData,
            &FOLDERID_UserProfiles,
            &FOLDERID_Profile,
            &FOLDERID_Desktop,
            &FOLDERID_Documents,
            &FOLDERID_Downloads,
            &FOLDERID_Pictures,
            &FOLDERID_Videos,
            &FOLDERID_Music,
            &FOLDERID_RoamingAppData,
            &FOLDERID_LocalAppData,
            &FOLDERID_LocalAppDataLow,
            &FOLDERID_AppDataProgramData,
        ]
        .into_iter()
        .filter_map(known)
        .collect();
        let onedrive = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().filter_map(std::env::var_os).map(PathBuf::from).fold(Vec::new(), |mut all: Vec<PathBuf>, path| {
            if !all.contains(&path) {
                all.push(path);
            }
            all
        });
        Places { windows: known(&FOLDERID_Windows), kept, onedrive }
    }
}

/// The path `path` takes from the drive's root `root` ("C:\\"), where it is on that drive.
fn on_drive<'a>(path: &'a Path, root: &str) -> Option<&'a Path> {
    let text = path.to_str()?;
    (text.len() >= root.len() && text[..root.len()].eq_ignore_ascii_case(root)).then(|| Path::new(&text[root.len()..]))
}

/// A file time (100 ns since 1601) `days` before `now`.
fn days_before(now: i64, days: i64) -> i64 {
    now - days * 864_000_000_000
}

/// Now as a file time.
pub fn now() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (since.as_secs() as i64 + 11_644_473_600) * 10_000_000 + since.subsec_nanos() as i64 / 100
}

/// What a folder holds at any depth (on this PC: what is kept in the cloud alone apart).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held {
    pub bytes: u64,
    pub files: u64,
    /// What its OneDrive files kept in the cloud alone come to (on no disk here).
    pub cloud: u64,
    /// When anything in it was written last (a file time).
    pub written: i64,
    /// How much of it is of each kind (`Kind::at`).
    pub kinds: [u64; 9],
    /// Its largest file that may be removed, and its largest of those written over a year before it was surveyed:
    /// what tells whether any file in it passes a filter of a size at least (see `Filter::passes_within`).
    pub largest: u64,
    pub largest_old: u64,
    /// Whether it holds anything not to be removed (the system's, what may not be listed, the Recycle Bin), or is
    /// itself not to be removed whole.
    pub guarded: bool,
}

impl Held {
    /// The kind most of it is.
    pub fn kind(&self) -> Kind {
        KINDS.iter().copied().max_by_key(|kind| (self.kinds[kind.at()], std::cmp::Reverse(kind.at()))).unwrap_or(Kind::Other)
    }

    /// A file of it (its size and when it was written, `stat`), told as `is`, counted in; `old_before` when a file is
    /// over a year old.
    fn add_file(&mut self, stat: crate::search::index::Stat, is: FileIs, old_before: i64) {
        self.bytes += stat.size;
        self.files += 1;
        self.written = self.written.max(stat.written);
        self.kinds[is.kind.at()] += stat.size;
        self.guarded |= is.system;
        if !is.system {
            self.largest = self.largest.max(stat.size);
            if stat.written < old_before {
                self.largest_old = self.largest_old.max(stat.size);
            }
        }
    }

    /// A file alone (see `add_file`).
    fn of_file(stat: crate::search::index::Stat, is: FileIs, old_before: i64) -> Held {
        let mut held = Held::default();
        held.add_file(stat, is, old_before);
        held
    }

    fn add(&mut self, other: &Held, files_too: bool) {
        self.bytes += other.bytes;
        self.files += other.files;
        self.cloud += other.cloud;
        self.written = self.written.max(other.written);
        for (mine, theirs) in self.kinds.iter_mut().zip(other.kinds) {
            *mine += theirs;
        }
        if files_too {
            self.largest = self.largest.max(other.largest);
            self.largest_old = self.largest_old.max(other.largest_old);
        }
        self.guarded |= other.guarded;
    }
}

/// A folder as surveyed: what it holds, what it is, the folder it is in, and how the files directly in it are told.
#[derive(Clone, Copy, Debug)]
pub struct Folder {
    pub held: Held,
    pub role: Role,
    pub parent: u32,
    pub within: Within,
}

/// What a file is to the organizer: its kind, and whether it is the system's (never removed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileIs {
    pub kind: Kind,
    pub system: bool,
}

/// What a folder's files are told as, from the folders above it: as any, a game's, or the system's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Within {
    Plain,
    Game,
    System,
}

/// A drive surveyed (see `Survey::take`).
pub struct Survey {
    /// Its root ("C:\\").
    pub root: String,
    /// Every folder on it the user may list (each after the one it is in), and where each is among them, by its place
    /// in the index (see `folder`).
    folders: Vec<Folder>,
    slots: QuickMap<u32, u32>,
    /// What the drive holds and has free, as Windows tells it, and how much of what it holds is in no file listed
    /// (the file system's own, what may not be listed).
    pub total: u64,
    pub free: u64,
    pub other: u64,
    /// The Recycle Bin's folder, where the drive has one, and Windows' own, where it is on it.
    pub bin: Option<u32>,
    pub windows: Option<u32>,
    /// The folders under OneDrive.
    pub onedrive: QuickSet<u32>,
    /// When it was surveyed, and before when a file was written "over a year ago".
    pub at: i64,
    pub old_before: i64,
}

/// How a file of `name` (folded), with `flags`, in a folder whose things are told as `within`, is told.
fn file_is(name: &str, flags: u16, within: Within) -> FileIs {
    told_as(kind_of(name), flags, within)
}

/// How a file of the kind its extension tells (`by_name`), with `flags`, in a folder whose things are told as
/// `within`, is told.
fn told_as(by_name: Kind, flags: u16, within: Within) -> FileIs {
    if within == Within::System || flags & flag::SYSTEM != 0 {
        return FileIs { kind: Kind::System, system: true };
    }
    FileIs { kind: if within == Within::Game { Kind::Game } else { by_name }, system: false }
}

/// What the files of an extension come to as the lists are read: their bytes and count, how much is of each kind,
/// where they light the map, and the kind the extension itself tells.
#[derive(Default)]
struct OfExtension {
    bytes: u64,
    files: u64,
    kinds: [u64; 9],
    lit: Lit,
    by_name: Option<Kind>,
}

impl Survey {
    /// The drive at `root` whose index is `index` surveyed, the places on this PC told by `places`, as Windows tells
    /// its size (`total`, `free`), at `now` (a file time).
    pub fn take(index: &Index, root: &str, places: &Places, (total, free): (u64, u64), now: i64) -> Survey {
        let place_of = |path: &Path| on_drive(path, root).and_then(|rest| if rest.as_os_str().is_empty() { Some(ROOT) } else { index.place_of(rest) });
        let windows = places.windows.as_deref().and_then(place_of);
        let onedrive_roots: QuickSet<u32> = places.onedrive.iter().filter_map(|path| place_of(path)).collect();
        let mut kept: QuickSet<u32> = places.kept.iter().filter_map(|path| place_of(path)).chain(onedrive_roots.iter().copied()).collect();
        // (Each user's profile: what is in the drive's Users folder.)
        if let Some(users) = index.place_of(Path::new("Users")) {
            index.each_in(users, |place, entry| {
                if entry.folder() && entry.flags & flag::LINK == 0 {
                    kept.insert(place);
                }
            });
        }
        let mut bin = None;
        index.each_in(ROOT, |place, entry| {
            if entry.folder() && index.folded_name(place) == "$recycle.bin" {
                bin = Some(place);
            }
        });
        let old_before = days_before(now, 365);
        let mut folders: Vec<Folder> = Vec::new();
        let mut slots: QuickMap<u32, u32> = QuickMap::default();
        let mut onedrive = QuickSet::default();
        // Each folder found, from the root down (each told how its files are told, and whether it is under OneDrive),
        // its own files counted as it is; then each, the last found first, counted into the one it is in (found before
        // it: where that is among them kept in `above`). (Windows' own folder's files are the system's, and all in it
        // but its Temp; a game library's are a game's.)
        let mut above: Vec<u32> = Vec::new();
        let mut stack = vec![(ROOT, Within::Plain, false, 0)];
        while let Some((folder, within, under_onedrive, up)) = stack.pop() {
            let flags = index.entry(folder).flags;
            let role = if folder == ROOT || kept.contains(&folder) || Some(folder) == windows {
                Role::Kept
            } else if Some(folder) == bin {
                Role::Bin
            } else if flags & flag::UNREAD != 0 {
                Role::Unread
            } else if within == Within::System {
                Role::System
            } else {
                Role::Plain
            };
            let under_onedrive = under_onedrive || onedrive_roots.contains(&folder);
            if under_onedrive {
                onedrive.insert(folder);
            }
            let mut held = Held { guarded: role != Role::Plain, ..Held::default() };
            let slot = folders.len() as u32;
            index.each_in(folder, |place, entry| {
                if entry.folder() {
                    if entry.flags & flag::LINK != 0 {
                        return;
                    }
                    let name = index.folded_name(place);
                    let inner = if Some(place) == windows {
                        Within::System
                    } else if Some(folder) == windows && name == "temp" {
                        Within::Plain
                    } else if within == Within::Plain && GAME_LIBRARIES.contains(&name) {
                        Within::Game
                    } else {
                        within
                    };
                    stack.push((place, inner, under_onedrive, slot));
                    return;
                }
                let stat = index.stat(place);
                if entry.flags & flag::CLOUD != 0 {
                    held.cloud += stat.size;
                    return;
                }
                held.add_file(stat, file_is(index.folded_name(place), entry.flags, within), old_before);
            });
            folders.push(Folder { held, role, parent: index.entry(folder).parent, within });
            slots.insert(folder, slot);
            above.push(up);
        }
        for slot in (1..folders.len()).rev() {
            let Folder { held, role, .. } = folders[slot];
            // (What is in the Recycle Bin is no file to pick by a filter anywhere above it.)
            folders[above[slot] as usize].held.add(&held, role != Role::Bin);
        }
        let counted = folders.first().map_or(0, |root| root.held.bytes);
        let used = total.saturating_sub(free);
        Survey { root: root.to_string(), other: used.saturating_sub(counted), folders, slots, total, free, bin, windows, onedrive, at: now, old_before }
    }

    pub fn folder(&self, place: u32) -> Option<&Folder> {
        self.slots.get(&place).map(|&slot| &self.folders[slot as usize])
    }

    /// How many folders it surveyed.
    pub fn folders(&self) -> usize {
        self.folders.len()
    }

    /// Whether `place` (a folder's) is in the Recycle Bin, or is it.
    pub fn in_bin(&self, mut place: u32) -> bool {
        let Some(bin) = self.bin else { return false };
        loop {
            if place == bin {
                return true;
            }
            match self.folder(place) {
                Some(folder) if place != ROOT => place = folder.parent,
                _ => return false,
            }
        }
    }

    /// The folders from the root down to `place`, it last.
    pub fn chain(&self, place: u32) -> Vec<u32> {
        let mut chain = vec![place];
        let mut at = place;
        while at != ROOT {
            let Some(folder) = self.folder(at) else { break };
            at = folder.parent;
            chain.push(at);
        }
        chain.reverse();
        chain
    }
}

/// What files pass: written over a year ago or not, and of a size at least (none: any).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Filter {
    pub old: bool,
    pub least: u64,
}

impl Filter {
    pub fn on(&self) -> bool {
        self.old || self.least > 0
    }

    /// Whether a file of `bytes` written at `written` passes, the system's never (it is not one to remove).
    pub fn passes(&self, bytes: u64, written: i64, system: bool, old_before: i64) -> bool {
        !system && bytes >= self.least && (!self.old || written < old_before)
    }

    /// Whether any file in what holds `largest` and `largest_old` (see `Held`) passes.
    pub fn passes_within(&self, largest: u64, largest_old: u64) -> bool {
        let most = if self.old { largest_old } else { largest };
        most > 0 && most >= self.least
    }
}

/// The age a file written at `written` is in, as the list by time groups them (`AGES`), at `now`.
pub fn age_of(written: i64, now: i64) -> usize {
    const DAYS: [i64; 5] = [7, 30, 182, 365, 1095];
    DAYS.iter().position(|&days| written >= days_before(now, days)).unwrap_or(DAYS.len())
}

/// How many ages the list by time has.
pub const AGES: usize = 6;

/// What lights some of the map: a kind (from the legend), an extension or an age (from their lists).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Light {
    Kind(Kind),
    Extension(String),
    Age(usize),
}

impl Light {
    /// Whether a file named `name` (folded) of `kind`, written at `written`, is lit, at `now`.
    pub fn lights(&self, name: &str, kind: Kind, written: i64, now: i64) -> bool {
        match self {
            Light::Kind(lit) => kind == *lit,
            Light::Extension(extension) => extension_of(name) == extension,
            Light::Age(age) => age_of(written, now) == *age,
        }
    }
}

/// What a node of a `Tree` is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    File { system: bool },
    Folder(Role),
    /// What was too small to draw in a folder, together (see `Tree`).
    Rest,
    /// What the drive holds that is in no file listed (see `Survey::other`): shown at its root only.
    Unlisted,
}

/// A file or folder of the folder shown, as far down as it can be drawn: or what was too small to, together.
#[derive(Clone, Debug)]
pub struct Node {
    /// Its place in the index (a folder's, for its rest: `NONE` for what is in no file).
    pub place: u32,
    pub name: String,
    /// Its name folded (a file's: its extension and kind are told from it).
    pub folded: String,
    pub what: What,
    pub held: Held,
    /// What it holds that is drawn, the largest first: a folder's (none where it is too small to draw what is in it).
    pub children: Vec<usize>,
    /// Whether what it holds is drawn (else it is drawn as the one kind it mostly is).
    pub opened: bool,
    /// A rest's: how many things are in it, and whether each is a file.
    pub items: u64,
    pub only_files: bool,
}

impl Node {
    pub fn folder(&self) -> bool {
        matches!(self.what, What::Folder(_))
    }

    pub fn kind(&self) -> Kind {
        self.held.kind()
    }

    fn of(place: u32, what: What, held: Held) -> Node {
        Node { place, name: String::new(), folded: String::new(), what, held, children: Vec::new(), opened: false, items: 0, only_files: true }
    }
}

/// The folder shown and what it holds, as far down as a map of `area` square DIPs could draw (anything drawn smaller
/// than a DIP square is put together with others like it into its folder's rest; a folder drawn smaller than
/// `OPENED_LEAST` square DIPs is drawn as one), copied out of the index: what the map is laid out from.
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Each node's index, by its place (a folder's rest apart, by the folder's: see `rests`).
    pub by_place: QuickMap<u32, usize>,
    pub rests: QuickMap<u32, usize>,
    /// The area it was copied for.
    pub area: f32,
}

/// How large (square DIPs) a folder must be drawn for what is in it to be drawn: five DIPs square, the least a piece
/// of a folder is drawn (see `map`).
pub const OPENED_LEAST: f32 = 25.0;

impl Tree {
    /// `focus` of the drive `survey` surveyed, whose index is `index`, for a map of `area` square DIPs.
    pub fn take(index: &Index, survey: &Survey, focus: u32, area: f32) -> Tree {
        let mut tree = Tree { nodes: Vec::new(), by_place: QuickMap::default(), rests: QuickMap::default(), area };
        let Some(top) = survey.folder(focus) else { return tree };
        let unlisted = if focus == ROOT { survey.other } else { 0 };
        let per_dip = (top.held.bytes + unlisted) as f64 / area.max(1.0) as f64;
        let least = per_dip.ceil() as u64;
        let opened_least = (per_dip * OPENED_LEAST as f64).ceil() as u64;
        let mut first = Node::of(focus, What::Folder(top.role), top.held);
        first.name = if focus == ROOT { survey.root.trim_end_matches('\\').to_string() } else { index.name(focus).to_string_lossy().into_owned() };
        first.folded = index.folded_name(focus).to_string();
        first.held.bytes += unlisted;
        tree.nodes.push(first);
        tree.by_place.insert(focus, 0);
        let mut open = vec![0];
        while let Some(at) = open.pop() {
            let folder = tree.nodes[at].place;
            let within = survey.folder(folder).map_or(Within::Plain, |folder| folder.within);
            let mut children = Vec::new();
            let mut rest = Node::of(folder, What::Rest, Held::default());
            index.each_in(folder, |place, entry| {
                let node = if entry.folder() {
                    let Some(inner) = survey.folder(place).filter(|_| entry.flags & flag::LINK == 0) else { return };
                    Node::of(place, What::Folder(inner.role), inner.held)
                } else {
                    if entry.flags & flag::CLOUD != 0 {
                        return;
                    }
                    let is = file_is(index.folded_name(place), entry.flags, within);
                    Node::of(place, What::File { system: is.system }, Held::of_file(index.stat(place), is, survey.old_before))
                };
                if node.held.bytes == 0 {
                    return;
                }
                if node.held.bytes < least {
                    rest.held.add(&node.held, true);
                    rest.items += 1;
                    rest.only_files &= !node.folder();
                } else {
                    children.push(node);
                }
            });
            if at == 0 && unlisted > 0 {
                let mut held = Held { bytes: unlisted, guarded: true, ..Held::default() };
                held.kinds[Kind::System.at()] = unlisted;
                children.push(Node::of(crate::search::index::NONE, What::Unlisted, held));
            }
            if rest.items > 0 {
                children.push(rest);
            }
            children.sort_by(|a, b| b.held.bytes.cmp(&a.held.bytes).then((a.what == What::Rest).cmp(&(b.what == What::Rest))).then(a.place.cmp(&b.place)));
            let mut kids = Vec::with_capacity(children.len());
            for mut child in children {
                match child.what {
                    What::Rest => {
                        tree.rests.insert(folder, tree.nodes.len());
                    }
                    What::Unlisted => {}
                    _ => {
                        child.name = index.name(child.place).to_string_lossy().into_owned();
                        child.folded = index.folded_name(child.place).to_string();
                        tree.by_place.insert(child.place, tree.nodes.len());
                        if child.folder() && child.held.bytes >= opened_least {
                            open.push(tree.nodes.len());
                        }
                    }
                }
                kids.push(tree.nodes.len());
                tree.nodes.push(child);
            }
            tree.nodes[at].children = kids;
            tree.nodes[at].opened = true;
        }
        tree
    }

    pub fn root(&self) -> Option<&Node> {
        self.nodes.first()
    }

    /// The node what is at the end of `chain` (the places from the folder shown down to it) is drawn in: its own, or
    /// the rest of the folder it is in, or the folder drawn as one that it is in.
    pub fn drawn_in(&self, chain: &[u32]) -> Option<usize> {
        let mut drawn = *self.by_place.get(chain.first()?)?;
        for pair in chain.windows(2) {
            if !self.nodes[drawn].opened {
                break;
            }
            match self.by_place.get(&pair[1]) {
                Some(&at) => drawn = at,
                None => return Some(self.rests.get(&pair[0]).copied().unwrap_or(drawn)),
            }
        }
        Some(drawn)
    }
}

/// What a file of `stat`, told as `is`, holds (see `Held`), surveyed with files written before `old_before` old.
/// A file as the list of the largest shows it.
#[derive(Clone, Debug)]
pub struct Row {
    pub place: u32,
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub written: i64,
    pub kind: Kind,
}

/// An extension as the list by type shows it: how much its files come to, how many there are, the kind most of them
/// are.
#[derive(Clone, Debug)]
pub struct Extension {
    pub extension: String,
    pub bytes: u64,
    pub files: u64,
    pub kind: Kind,
}

/// What the lists beside the map show of the folder shown, filtered and lit: its largest files that pass and are lit
/// (as many as `ROWS`, the system's never), how many those are and what they come to, the extensions and the ages of
/// the files that pass, and the folders (by their places) holding a file lit by an extension or an age, and the
/// folders whose rest holds one. And for each extension listed and each age, the folders drawn on the map that hold a
/// file of it that passes (and the rests that do): the map lit by a row of those lists while the pointer is on it.
#[derive(Clone, Debug, Default)]
pub struct Detail {
    pub rows: Vec<Row>,
    pub passing: (u64, u64),
    pub extensions: Vec<Extension>,
    pub ages: [(u64, u64); AGES],
    pub lit_folders: QuickSet<u32>,
    pub lit_rests: QuickSet<u32>,
    pub by_extension: QuickMap<String, Lit>,
    pub by_age: [Lit; AGES],
}

/// The folders drawn on the map holding a file lit, and the folders whose rest holds one (by their places).
#[derive(Clone, Debug, Default)]
pub struct Lit {
    pub folders: QuickSet<u32>,
    pub rests: QuickSet<u32>,
    /// The folder a file was last marked in, and whether its rest was marked then: the files of a folder come
    /// together, and once one is marked another marks nothing more (unless it marks the rest).
    last: Option<(u32, bool)>,
}

impl Lit {
    /// A file lit in `folder` (`drawn` telling how that is drawn, `drawn_up` the nearest folder drawn it is in or is,
    /// `up` each drawn folder's own), the file itself drawn or not: the folders drawn from `drawn_up` up told as
    /// holding one (those above one told already are), and the rest it is drawn in.
    fn mark(&mut self, up: &QuickMap<u32, u32>, folder: u32, drawn: Drawn, drawn_up: u32, file_drawn: bool) {
        let rest = match drawn {
            Drawn::Opened if !file_drawn => Some(folder),
            Drawn::InRestOf(owner) => Some(owner),
            _ => None,
        };
        let rest_marked = matches!(self.last, Some((last, marked)) if last == folder && marked);
        if self.last.is_some_and(|(last, _)| last == folder) && (rest.is_none() || rest_marked) {
            return;
        }
        let mut at = drawn_up;
        while self.folders.insert(at) {
            match up.get(&at) {
                Some(&above) => at = above,
                None => break,
            }
        }
        if let Some(rest) = rest {
            self.rests.insert(rest);
        }
        self.last = Some((folder, rest_marked || rest.is_some()));
    }
}

/// How many of the largest files the list holds.
pub const ROWS: usize = 80;
/// How many extensions the list by type holds.
pub const EXTENSIONS_LISTED: usize = 40;

impl Detail {
    /// The lists for `focus` of the drive `survey` surveyed (its index `index`), filtered by `filter` and lit by
    /// `light`, its map drawn from `tree`. (What is in the Recycle Bin only where it is the folder shown, or in it.)
    /// Where a filter is on, or a type or a time lights the map, the folders holding a file that passes and is lit are
    /// told (and the rests holding one): a kind alone lights the map by what its blocks mostly are.
    pub fn take(index: &Index, survey: &Survey, focus: u32, filter: Filter, light: Option<&Light>, tree: &Tree) -> Detail {
        let mut detail = Detail::default();
        let in_bin = survey.in_bin(focus);
        let marks = filter.on() || matches!(light, Some(Light::Extension(_) | Light::Age(_)));
        // (The largest kept as they come, each with its kind: the least of them let go of as a larger one comes.)
        let mut largest: std::collections::BinaryHeap<std::cmp::Reverse<(u64, u32, usize)>> = std::collections::BinaryHeap::new();
        let mut extensions: QuickMap<&str, OfExtension> = QuickMap::default();
        // Each folder drawn's own (by their places): a file lit told of in each folder drawn it is in.
        let mut up: QuickMap<u32, u32> = QuickMap::default();
        for node in &tree.nodes {
            for &child in &node.children {
                if matches!(tree.nodes[child].what, What::Folder(_)) {
                    up.insert(tree.nodes[child].place, node.place);
                }
            }
        }
        let mut stack = vec![(focus, Drawn::of(tree, focus, None), focus)];
        while let Some((folder, drawn, drawn_up)) = stack.pop() {
            if !in_bin && Some(folder) == survey.bin {
                continue;
            }
            let Some(within) = survey.folder(folder).map(|folder| folder.within) else { continue };
            index.each_in(folder, |place, entry| {
                if entry.folder() {
                    if entry.flags & flag::LINK == 0 {
                        let drawn_up = if tree.by_place.contains_key(&place) { place } else { drawn_up };
                        stack.push((place, Drawn::of(tree, place, Some((folder, drawn))), drawn_up));
                    }
                    return;
                }
                let stat = index.stat(place);
                if entry.flags & flag::CLOUD != 0 || stat.size == 0 {
                    return;
                }
                let name = index.folded_name(place);
                // (Its extension looked up once: the kind it tells kept with what its files come to.)
                let extension = extensions.entry(extension_of(name)).or_default();
                let by_name = *extension.by_name.get_or_insert_with(|| kind_of(name));
                let is = told_as(by_name, entry.flags, within);
                if filter.on() && !filter.passes(stat.size, stat.written, is.system, survey.old_before) {
                    return;
                }
                extension.bytes += stat.size;
                extension.files += 1;
                extension.kinds[is.kind.at()] += stat.size;
                let age_at = age_of(stat.written, survey.at);
                let age = &mut detail.ages[age_at];
                age.0 += 1;
                age.1 += stat.size;
                let file_drawn = tree.by_place.contains_key(&place);
                extension.lit.mark(&up, folder, drawn, drawn_up, file_drawn);
                detail.by_age[age_at].mark(&up, folder, drawn, drawn_up, file_drawn);
                if !light.is_none_or(|light| light.lights(name, is.kind, stat.written, survey.at)) {
                    return;
                }
                if marks {
                    detail.mark(survey, focus, folder, drawn, file_drawn);
                }
                if is.system {
                    return;
                }
                detail.passing.0 += 1;
                detail.passing.1 += stat.size;
                let row = std::cmp::Reverse((stat.size, place, is.kind.at()));
                if largest.len() < ROWS {
                    largest.push(row);
                } else if let Some(mut least) = largest.peek_mut() {
                    if row < *least {
                        *least = row;
                    }
                }
            });
        }
        let mut rows: Vec<(u64, u32, usize)> = largest.into_iter().map(|std::cmp::Reverse(row)| row).collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        detail.rows = rows
            .into_iter()
            .filter_map(|(bytes, place, kind)| Some(Row { place, name: index.name(place).to_string_lossy().into_owned(), path: index.path(place, &survey.root)?, bytes, written: index.stat(place).written, kind: KINDS[kind] }))
            .collect();
        // (An extension met only by files filtered out is none to list.)
        let mut extensions: Vec<(Extension, Lit)> = extensions
            .into_iter()
            .filter(|(_, of)| of.files > 0)
            .map(|(extension, of)| (Extension { extension: extension.to_string(), bytes: of.bytes, files: of.files, kind: Held { kinds: of.kinds, ..Held::default() }.kind() }, of.lit))
            .collect();
        extensions.sort_by(|a, b| b.0.bytes.cmp(&a.0.bytes).then(a.0.extension.cmp(&b.0.extension)));
        extensions.truncate(EXTENSIONS_LISTED);
        let (extensions, lit): (Vec<Extension>, Vec<Lit>) = extensions.into_iter().unzip();
        detail.by_extension = extensions.iter().map(|listed| listed.extension.clone()).zip(lit).collect();
        detail.extensions = extensions;
        detail
    }

    /// A file lit in `folder` (itself drawn, or not), `drawn` telling how `folder` is drawn: each folder from it up to
    /// the one shown (`focus`) told as holding one (those above one told already are), and the rest it is drawn in.
    fn mark(&mut self, survey: &Survey, focus: u32, folder: u32, drawn: Drawn, file_drawn: bool) {
        let mut at = folder;
        while self.lit_folders.insert(at) && at != focus && at != ROOT {
            match survey.folder(at) {
                Some(above) => at = above.parent,
                None => break,
            }
        }
        match drawn {
            Drawn::Opened if !file_drawn => {
                self.lit_rests.insert(folder);
            }
            Drawn::InRestOf(owner) => {
                self.lit_rests.insert(owner);
            }
            _ => {}
        }
    }
}

/// How a folder is drawn on the map (see `Tree`): what it holds drawn, or itself as one, or in the rest of the folder
/// drawn whose (as all in it then), or in one drawn as one (as all in it then).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drawn {
    Opened,
    Whole,
    InRestOf(u32),
    InWhole,
}

impl Drawn {
    /// How `place` is drawn, the folder it is in (and how that is drawn) told where it is not the one shown.
    fn of(tree: &Tree, place: u32, above: Option<(u32, Drawn)>) -> Drawn {
        let own = tree.by_place.get(&place).map(|&at| if tree.nodes[at].opened { Drawn::Opened } else { Drawn::Whole });
        match (above, own) {
            (None, Some(own)) | (Some((_, Drawn::Opened)), Some(own)) => own,
            (None, None) => Drawn::InWhole,
            (Some((folder, Drawn::Opened)), None) => Drawn::InRestOf(folder),
            (Some((_, Drawn::InRestOf(owner))), _) => Drawn::InRestOf(owner),
            (Some(_), _) => Drawn::InWhole,
        }
    }
}

#[cfg(test)]
mod tests;
