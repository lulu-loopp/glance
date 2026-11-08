//! A drive's files and folders, every name in each folder the user can list: one entry for each name (a file with
//! several names, hard links, has an entry for each), each knowing its folder. Their names case folded lie in one run,
//! each ended by a 0 (which no name holds), searched whole at once; a name is kept as written apart only where it is
//! not as folded.
//!
//! What was last kept on disk is read where it lies (see `store`: its file mapped into memory, which is not Glance's
//! own: Windows reads in what a search goes through, and lets it go again as other programs want the memory), and
//! what changed since is kept in memory over it: the entries added since, after its own, their names in runs of their
//! own, and its own entries as they are now where they changed. Entries are only ever added: one gone, or renamed, is
//! marked dead (a renamed one added again), and the dead are left out as it is kept anew (see `write`).

use super::readings;
use super::store::Bytes;
use memchr::memmem::Finder;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

/// The drive's root folder's place.
pub const ROOT: u32 = 0;

/// No place: the root's folder.
pub const NONE: u32 = u32::MAX;

/// Where an entry's name is as written, where it is as folded.
const AS_FOLDED: u32 = u32::MAX;

/// How deep in folders a name is kept (a path deeper is none Windows' own programs open).
const DEEPEST: u16 = 4096;

/// What an entry is.
pub mod flag {
    pub const FOLDER: u16 = 1;
    /// Gone, or renamed (and added again under its new name).
    pub const DEAD: u16 = 2;
    /// A folder whose names could not be listed (the user may not, or it failed).
    pub const UNREAD: u16 = 4;
    /// A folder that is a link to another place (a junction, a symbolic link, another drive mounted there), not
    /// followed.
    pub const LINK: u16 = 8;
    /// Its name holds Chinese characters (found by their pinyin too).
    pub const CHINESE: u16 = 16;
    pub const HIDDEN: u16 = 32;
    pub const SYSTEM: u16 = 64;
    /// A folder to be listed: found and not yet listed, changed and put off, or its listing failed. Kept with the
    /// index, so that it is listed as Glance starts again, if not before (a drive's first listing, cut short, goes on
    /// from there: see `super::super::sweep`).
    pub const STALE: u16 = 128;
    /// All there are.
    pub const ALL: u16 = 255;
}

/// A file or folder (16 bytes, as it is kept on disk too).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Entry {
    /// Its folder's place.
    pub parent: u32,
    /// Where its name, case folded, starts in its run of names.
    at: u32,
    /// Where its name as written (UTF-16, little-endian) starts in its run of them, and how many bytes long it is,
    /// where it is not as folded.
    written: u32,
    written_len: u16,
    pub flags: u16,
}

impl Entry {
    pub fn alive(&self) -> bool {
        self.flags & flag::DEAD == 0
    }

    pub fn folder(&self) -> bool {
        self.flags & flag::FOLDER != 0
    }
}

/// A name in a folder as listed: its file id, attributes and (for a link) what kind of link it is.
pub struct Item {
    pub name: Vec<u16>,
    pub id: u64,
    pub attributes: u32,
    pub tag: u32,
}

const ATTRIBUTE_HIDDEN: u32 = 0x2;
const ATTRIBUTE_SYSTEM: u32 = 0x4;
const ATTRIBUTE_DIRECTORY: u32 = 0x10;
const ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

impl Item {
    pub fn folder(&self) -> bool {
        self.attributes & ATTRIBUTE_DIRECTORY != 0
    }

    /// Whether the folder's own names are listed: not a link to another place (a cloud drive's folder, held here
    /// till it is opened, is followed: its names are its own).
    pub fn followed(&self) -> bool {
        self.folder() && (self.attributes & ATTRIBUTE_REPARSE_POINT == 0 || cloud(self.tag))
    }

    fn flags(&self) -> u16 {
        let mut flags = 0;
        if self.folder() {
            flags |= flag::FOLDER;
            if !self.followed() {
                flags |= flag::LINK;
            }
        }
        if self.attributes & ATTRIBUTE_HIDDEN != 0 {
            flags |= flag::HIDDEN;
        }
        if self.attributes & ATTRIBUTE_SYSTEM != 0 {
            flags |= flag::SYSTEM;
        }
        flags
    }
}

/// Whether a link's kind is a cloud drive's file or folder held here (IO_REPARSE_TAG_CLOUD and its kinds).
fn cloud(tag: u32) -> bool {
    tag & 0xFFFF_0FFF == 0x9000_001A
}

/// What one round of listing folders again changed, to be finished once all of them are listed: folders moved
/// (their old entries' children go to the new), and folders not found where they were (gone, unless they were moved).
#[derive(Default)]
pub struct Changes {
    moved: HashMap<u32, u32>,
    missing: Vec<u32>,
    /// Folders found that are to be listed, by their places and ids.
    pub to_list: Vec<(u32, u64)>,
}

/// The index as it was last kept, where it lies (see `store`): read in place, never copied. Each part's place in it
/// (in bytes from its start), and how many it holds.
struct Base {
    bytes: Bytes,
    entries: (usize, usize),
    folded: (usize, usize),
    written: (usize, usize),
    /// The folders' places (in order), their ids (in the same order), and the order of those by id.
    places: (usize, usize),
    ids: usize,
    by_id: usize,
    chinese: (usize, usize),
}

/// The bytes at `at` as `count` of `T`, where they lie (none where they are not where a `T` may be).
fn cast<T: Copy>(bytes: &[u8], at: usize, count: usize) -> Option<&[T]> {
    let part = bytes.get(at..at.checked_add(count.checked_mul(size_of::<T>())?)?)?;
    // (Little-endian as it is kept, as Windows' processors are.)
    (part.as_ptr() as usize).is_multiple_of(align_of::<T>()).then(|| unsafe { std::slice::from_raw_parts(part.as_ptr().cast::<T>(), count) })
}

impl Base {
    fn entries(&self) -> &[Entry] {
        cast(self.bytes.slice(), self.entries.0, self.entries.1).unwrap()
    }

    fn folded(&self) -> &[u8] {
        &self.bytes.slice()[self.folded.0..][..self.folded.1]
    }

    fn written(&self) -> &[u8] {
        &self.bytes.slice()[self.written.0..][..self.written.1]
    }

    fn places(&self) -> &[u32] {
        cast(self.bytes.slice(), self.places.0, self.places.1).unwrap()
    }

    fn ids(&self) -> &[u64] {
        cast(self.bytes.slice(), self.ids, self.places.1).unwrap()
    }

    fn by_id(&self) -> &[u32] {
        cast(self.bytes.slice(), self.by_id, self.places.1).unwrap()
    }

    fn chinese(&self) -> &[u32] {
        cast(self.bytes.slice(), self.chinese.0, self.chinese.1).unwrap()
    }

    /// The place of the folder whose id is `id`.
    fn folder(&self, id: u64) -> Option<u32> {
        let (ids, by_id) = (self.ids(), self.by_id());
        let at = by_id.binary_search_by_key(&id, |&k| ids[k as usize]).ok()?;
        Some(self.places()[by_id[at] as usize])
    }

    /// The id of the folder at `place`.
    fn id(&self, place: u32) -> Option<u64> {
        let at = self.places().binary_search(&place).ok()?;
        Some(self.ids()[at])
    }
}

pub struct Index {
    base: Option<Base>,
    /// The base's entries as they are now, where they changed (gone, moved, marked).
    changed: HashMap<u32, Entry>,
    /// The entries added since, placed after the base's, and their runs of names.
    entries: Vec<Entry>,
    folded: Vec<u8>,
    written: Vec<u8>,
    /// The folders' places by their ids, where that changed since the base (none: no folder has the id any more);
    /// and the ids of the folders added since, by their places (in order).
    folders: HashMap<u64, Option<u32>>,
    ids: Vec<(u32, u64)>,
    /// The entries added since whose names hold Chinese characters.
    chinese: Vec<u32>,
    /// Entries that may be in a folder other than the one the base says (added since, or moved), by that folder:
    /// each where it went, and perhaps since gone on (each entry is looked at as it is now).
    adopted: HashMap<u32, Vec<u32>>,
    dead: usize,
    /// Whether its folders are known by their file ids (an NTFS drive's), or else by their names alone, each
    /// given an id of its own from `made`.
    by_id: bool,
    made: u64,
}

/// The first id given a folder by `Index` (where the drive's are not kept).
const MADE: u64 = 1 << 63;

/// The entries of a run of names (`entries`, the first of them at `first`) whose names (`run`) hold what `finder`
/// looks for, and are `alive`: looked for by `threads` threads at once, each through its share of the names.
fn look_through(finder: &Finder, run: &[u8], entries: &[Entry], first: u32, threads: usize, alive: &(dyn Fn(u32) -> bool + Sync)) -> Vec<u32> {
    let mut bounds = vec![0usize];
    for share in 1..threads.max(1) {
        let from = run.len() * share / threads;
        let start = run[from..].iter().position(|&b| b == 0).map_or(run.len(), |end| from + end + 1);
        if start > *bounds.last().unwrap() {
            bounds.push(start);
        }
    }
    bounds.push(run.len());
    let look = |from: usize, to: usize| {
        let mut found = Vec::new();
        let mut at = from;
        while let Some(hit) = finder.find(&run[at..to]) {
            let place = entries.partition_point(|entry| entry.at as usize <= at + hit) - 1;
            if alive(first + place as u32) {
                found.push(first + place as u32);
            }
            at = entries.get(place + 1).map_or(to, |next| next.at as usize);
        }
        found
    };
    if bounds.len() == 2 {
        return look(0, run.len());
    }
    std::thread::scope(|scope| {
        let looks: Vec<_> = bounds.windows(2).map(|share| scope.spawn(move || look(share[0], share[1]))).collect();
        looks.into_iter().flat_map(|looked| looked.join().unwrap()).collect()
    })
}

/// The name of the entry at `place` of `entries` in its run of names `run`, case folded.
fn folded_in<'a>(entries: &[Entry], run: &'a [u8], place: usize) -> &'a str {
    let at = entries[place].at as usize;
    let end = entries.get(place + 1).map_or(run.len(), |next| next.at as usize) - 1;
    // (Folded from a str, or checked to be UTF-8 as it was read.)
    unsafe { std::str::from_utf8_unchecked(&run[at..end]) }
}

impl Index {
    fn empty(by_id: bool) -> Self {
        Index { base: None, changed: HashMap::new(), entries: Vec::new(), folded: Vec::new(), written: Vec::new(), folders: HashMap::new(), ids: Vec::new(), chinese: Vec::new(), adopted: HashMap::new(), dead: 0, by_id, made: MADE }
    }

    /// An index of a drive whose root folder has `root_id` (a drive whose folders are known `by_id`, else by their
    /// names), with nothing in it yet: its root to be listed.
    pub fn new(root_id: u64, by_id: bool) -> Self {
        let mut index = Index::empty(by_id);
        index.add(NONE, &[], flag::FOLDER | flag::STALE);
        index.set_id(ROOT, root_id);
        index
    }

    fn base_len(&self) -> u32 {
        self.base.as_ref().map_or(0, |base| base.entries.1 as u32)
    }

    /// How many entries it has (the dead too). (Never none: its root is one.)
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u32 {
        self.base_len() + self.entries.len() as u32
    }

    /// The entry at `place`, as it is now.
    pub fn entry(&self, place: u32) -> Entry {
        let base_len = self.base_len();
        match &self.base {
            Some(base) if place < base_len => match self.changed.is_empty() {
                true => base.entries()[place as usize],
                false => self.changed.get(&place).copied().unwrap_or_else(|| base.entries()[place as usize]),
            },
            _ => self.entries[(place - base_len) as usize],
        }
    }

    fn set_entry(&mut self, place: u32, entry: Entry) {
        if self.entry(place).parent != entry.parent {
            self.adopted.entry(entry.parent).or_default().push(place);
        }
        let base_len = self.base_len();
        if place < base_len {
            self.changed.insert(place, entry);
        } else {
            self.entries[(place - base_len) as usize] = entry;
        }
    }

    /// How many names it holds (the live entries but the root).
    pub fn names(&self) -> usize {
        self.len() as usize - self.dead - 1
    }

    fn add(&mut self, parent: u32, name: &[u16], flags: u16) -> u32 {
        let place = self.len();
        let name = OsString::from_wide(name);
        let lossy = name.to_string_lossy();
        let at = self.folded.len();
        for c in lossy.chars().flat_map(char::to_lowercase) {
            self.folded.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        }
        let (written, written_len) = if name.to_str().is_some_and(|name| self.folded[at..] == *name.as_bytes()) {
            (AS_FOLDED, 0)
        } else {
            let from = self.written.len();
            self.written.extend(name.encode_wide().flat_map(u16::to_le_bytes));
            (from as u32, (self.written.len() - from) as u16)
        };
        self.folded.push(0);
        let mut flags = flags;
        if lossy.chars().any(readings::read) {
            flags |= flag::CHINESE;
            self.chinese.push(place);
        }
        self.entries.push(Entry { parent, at: at as u32, written, written_len, flags });
        self.adopted.entry(parent).or_default().push(place);
        place
    }

    fn make_id(&mut self) -> u64 {
        self.made += 1;
        self.made
    }

    pub fn by_id(&self) -> bool {
        self.by_id
    }

    /// The folder added at `place` known by `id`.
    fn set_id(&mut self, place: u32, id: u64) {
        self.folders.insert(id, Some(place));
        self.ids.push((place, id));
    }

    /// A folder's file id.
    pub fn id(&self, place: u32) -> Option<u64> {
        match &self.base {
            Some(base) if place < self.base_len() => base.id(place),
            _ => self.ids.binary_search_by_key(&place, |&(place, _)| place).ok().map(|at| self.ids[at].1),
        }
    }

    /// The folder this file id is the id of, live or dead.
    fn known(&self, id: u64) -> Option<u32> {
        match self.folders.get(&id) {
            Some(&place) => place,
            None => self.base.as_ref()?.folder(id),
        }
    }

    /// The live folder of this file id.
    pub fn folder(&self, id: u64) -> Option<u32> {
        self.known(id).filter(|&place| self.entry(place).alive())
    }

    /// The entry at `place` in its run of entries, the run of its names as folded and as written, and its place in it.
    fn run(&self, place: u32) -> (&[Entry], &[u8], &[u8], usize) {
        let base_len = self.base_len();
        match &self.base {
            Some(base) if place < base_len => (base.entries(), base.folded(), base.written(), place as usize),
            _ => (&self.entries, &self.folded, &self.written, (place - base_len) as usize),
        }
    }

    /// An entry's name as written.
    pub fn name(&self, place: u32) -> Cow<'_, OsStr> {
        let (entries, folded, written, at) = self.run(place);
        let entry = &entries[at];
        if entry.written == AS_FOLDED {
            Cow::Borrowed(OsStr::new(folded_in(entries, folded, at)))
        } else {
            let bytes = &written[entry.written as usize..][..entry.written_len as usize];
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            Cow::Owned(OsString::from_wide(&wide))
        }
    }

    /// An entry's name case folded.
    pub fn folded_name(&self, place: u32) -> &str {
        let (entries, folded, _, at) = self.run(place);
        folded_in(entries, folded, at)
    }

    /// An entry's whole path, from `root` (the drive's, "C:\"); none where a folder on the way is unknown or gone.
    pub fn path(&self, place: u32, root: &str) -> Option<PathBuf> {
        let mut parts = Vec::new();
        let mut at = place;
        while at != ROOT {
            if at >= self.len() {
                return None;
            }
            let entry = self.entry(at);
            if !entry.alive() || parts.len() > DEEPEST as usize {
                return None;
            }
            parts.push(self.name(at));
            at = entry.parent;
        }
        let mut path = PathBuf::from(root);
        path.extend(parts.iter().rev());
        Some(path)
    }

    /// The folder at `path` (from the root, by its names: on a drive whose folders are known so), or else the last
    /// folder on the way to it that is known.
    pub fn folder_at(&self, path: &std::path::Path) -> u32 {
        let mut folder = ROOT;
        for part in path.components() {
            let found = self.held_by(folder).into_iter().find(|&place| self.entry(place).folder() && self.name(place) == part.as_os_str());
            match found {
                Some(place) => folder = place,
                None => break,
            }
        }
        folder
    }

    pub fn set_flag(&mut self, place: u32, flag: u16, on: bool) {
        let mut entry = self.entry(place);
        let flags = if on { entry.flags | flag } else { entry.flags & !flag };
        if flags != entry.flags {
            entry.flags = flags;
            self.set_entry(place, entry);
        }
    }

    fn kill(&mut self, place: u32) {
        let mut entry = self.entry(place);
        if !entry.alive() {
            return;
        }
        entry.flags |= flag::DEAD;
        self.set_entry(place, entry);
        self.dead += 1;
        // (Its id no folder's any more, unless it is another's now: moved, it was given to where it went first.)
        if entry.folder() {
            if let Some(id) = self.id(place) {
                if self.known(id) == Some(place) {
                    self.folders.insert(id, None);
                }
            }
        }
    }

    /// The live entries in each of `folders`.
    pub fn children(&self, folders: &HashSet<u32>) -> HashMap<u32, Vec<u32>> {
        folders
            .iter()
            .filter_map(|&folder| {
                let held = self.held_by(folder);
                (!held.is_empty()).then_some((folder, held))
            })
            .collect()
    }

    /// The live entries in `folder`, in order: those the base had in it and those that may have come since, each as
    /// it is now. (Looked up, never looked for through all the entries.)
    fn held_by(&self, folder: u32) -> Vec<u32> {
        let came = self.adopted.get(&folder).into_iter().flatten().copied();
        let mut held: Vec<u32> = self
            .kept_in(folder)
            .chain(came)
            .filter(|&place| {
                let entry = self.entry(place);
                entry.alive() && entry.parent == folder
            })
            .collect();
        held.sort_unstable();
        held.dedup();
        held
    }

    /// The base's entries that were in `folder` as it was kept: together, as the base's entries (but the root, the
    /// first) are in order of their folders' places (see `write`).
    fn kept_in(&self, folder: u32) -> std::ops::Range<u32> {
        let Some(base) = &self.base else { return 0..0 };
        let entries = &base.entries()[1..];
        let from = entries.partition_point(|entry| entry.parent < folder);
        let to = from + entries[from..].partition_point(|entry| entry.parent == folder);
        from as u32 + 1..to as u32 + 1
    }

    /// `folder` as it is listed now, against `before`, its entries when the round began: names gone marked dead
    /// (a folder only once the round is finished, as it may have moved), new ones added, folders found that are to
    /// be listed noted in `changes` (and marked so till they are).
    pub fn list(&mut self, folder: u32, items: &[Item], before: &[u32], changes: &mut Changes) {
        let mut by_name: HashMap<OsString, u32> = before.iter().filter(|&&place| self.entry(place).alive()).map(|&place| (self.name(place).into_owned(), place)).collect();
        for item in items {
            let name = OsString::from_wide(&item.name);
            let flags = item.flags();
            // (Where the drive keeps no ids, a folder is the one of its name.)
            let id = if self.by_id { item.id } else { by_name.get(&name).and_then(|&child| self.id(child)).unwrap_or_else(|| self.make_id()) };
            if let Some(child) = by_name.remove(&name) {
                let was = self.entry(child);
                let same = match (was.folder(), item.folder()) {
                    (true, true) => self.id(child) == Some(id),
                    (false, false) => true,
                    _ => false,
                };
                if same {
                    let kept = was.flags & (flag::FOLDER | flag::CHINESE | flag::UNREAD | flag::STALE);
                    if kept | flags != was.flags {
                        self.set_entry(child, Entry { flags: kept | flags, ..was });
                    }
                    // (A link before, its names are listed now.)
                    if was.flags & flag::LINK != 0 && item.followed() {
                        self.set_flag(child, flag::STALE, true);
                        changes.to_list.push((child, id));
                    }
                    continue;
                }
                if was.folder() {
                    changes.missing.push(child);
                } else {
                    self.kill(child);
                }
            }
            if item.folder() && self.by_id {
                if let Some(old) = self.folder(id) {
                    // Moved here, or renamed: its entries go with it.
                    let place = self.add(folder, &item.name, flags | (self.entry(old).flags & (flag::UNREAD | flag::STALE)));
                    self.set_id(place, id);
                    self.kill(old);
                    changes.moved.insert(old, place);
                    continue;
                }
            }
            let place = self.add(folder, &item.name, if item.followed() { flags | flag::STALE } else { flags });
            if item.folder() {
                self.set_id(place, id);
                if item.followed() {
                    changes.to_list.push((place, id));
                }
            }
        }
        for (_, place) in by_name {
            if self.entry(place).folder() {
                changes.missing.push(place);
            } else {
                self.kill(place);
            }
        }
    }

    /// The round finished: moved folders' entries put in them where they are now, and folders gone marked dead
    /// with all they held.
    pub fn finish(&mut self, changes: Changes) {
        if !changes.moved.is_empty() {
            let moved = |mut place: u32| {
                while let Some(&to) = changes.moved.get(&place) {
                    place = to;
                }
                place
            };
            for (_, held) in self.children(&changes.moved.keys().copied().collect()) {
                for place in held {
                    let entry = self.entry(place);
                    self.set_entry(place, Entry { parent: moved(entry.parent), ..entry });
                }
            }
        }
        let mut gone: HashSet<u32> = HashSet::new();
        for place in changes.missing {
            if self.entry(place).alive() {
                self.kill(place);
                gone.insert(place);
            }
        }
        self.kill_within(gone);
    }

    /// Everything within `folders` (marked dead already) marked dead.
    pub fn kill_within(&mut self, mut folders: HashSet<u32>) {
        while !folders.is_empty() {
            let mut within = HashSet::new();
            for (_, held) in self.children(&folders) {
                for place in held {
                    let folder = self.entry(place).folder();
                    self.kill(place);
                    if folder {
                        within.insert(place);
                    }
                }
            }
            folders = within;
        }
    }

    /// `folder` could not be listed as the user may not: what it held is not shown any more.
    pub fn denied(&mut self, folder: u32, before: &[u32]) {
        let mut gone = HashSet::new();
        for &place in before {
            if self.entry(place).alive() {
                self.kill(place);
                if self.entry(place).folder() {
                    gone.insert(place);
                }
            }
        }
        self.kill_within(gone);
        self.set_flag(folder, flag::UNREAD, true);
    }

    /// Whether enough is dead to be left out (see `write`).
    pub fn untidy(&self) -> bool {
        self.dead > 65_536 && self.dead * 4 > self.len() as usize
    }

    /// The places of the live entries whose folded names hold `words` (folded), in order, looked for by `threads`
    /// threads at once, each through its share of the names.
    pub fn holding(&self, words: &str, threads: usize) -> Vec<u32> {
        let finder = Finder::new(words.as_bytes());
        let alive = |place: u32| self.entry(place).alive();
        let mut found = match &self.base {
            Some(base) => look_through(&finder, base.folded(), base.entries(), 0, threads, &alive),
            None => Vec::new(),
        };
        found.extend(look_through(&finder, &self.folded, &self.entries, self.base_len(), threads, &alive));
        found
    }

    /// The places of the live entries whose names hold Chinese characters.
    pub fn chinese(&self) -> impl Iterator<Item = u32> + '_ {
        let base = self.base.as_ref().map_or(&[][..], Base::chinese);
        base.iter().chain(&self.chinese).copied().filter(|&place| self.entry(place).alive())
    }

    /// About how much memory it holds of its own (what is mapped in of the base is not its own: see the top).
    pub fn held(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.folded.capacity()
            + self.written.capacity()
            + self.changed.capacity() * (size_of::<(u32, Entry)>() + 1)
            + self.folders.capacity() * (size_of::<(u64, Option<u32>)>() + 1)
            + self.ids.capacity() * size_of::<(u32, u64)>()
            + self.chinese.capacity() * 4
            + self.adopted.capacity() * (size_of::<(u32, Vec<u32>)>() + 1)
            + self.adopted.values().map(|places| places.capacity() * 4).sum::<usize>()
    }

    /// How deep each entry is (the root 0, what is in it 1, ...), `LEFT` for those left out as it is kept: the dead,
    /// what is in a dead folder (or one unknown), and what lies deeper than `DEEPEST`.
    fn depths(&self) -> Vec<u16> {
        const UNKNOWN: u16 = u16::MAX;
        let len = self.len();
        let mut depth = vec![UNKNOWN; len as usize];
        let mut chain = Vec::new();
        for place in 0..len {
            let mut at = place;
            let top = loop {
                if depth[at as usize] != UNKNOWN {
                    break depth[at as usize];
                }
                let entry = self.entry(at);
                if at == ROOT {
                    depth[0] = 0;
                    break 0;
                }
                // (A folder that is in itself, at last, is that deep too.)
                if !entry.alive() || entry.parent >= len || chain.len() >= DEEPEST as usize {
                    depth[at as usize] = LEFT;
                    break LEFT;
                }
                chain.push(at);
                at = entry.parent;
            };
            let mut deep = top;
            for &place in chain.iter().rev() {
                deep = if deep >= DEEPEST { LEFT } else { deep + 1 };
                depth[place as usize] = deep;
            }
            chain.clear();
        }
        depth
    }

    /// It as kept on disk (see `read`): the dead left out, and the rest placed anew, each after its folder (by how
    /// deep it is, then by its folder's new place, then as it was: the entries of a folder together, and all but the
    /// root in order of their folders' places). How many entries, bytes of names folded, bytes of names as written, folders with
    /// ids and names holding Chinese there are, then the entries, the names folded, the names as written, the
    /// folders' places, their ids, their order by id, and the places of those holding Chinese: each part from a
    /// multiple of 8 bytes, all little-endian, written as it is made (never all of it at once in memory).
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        let depth = self.depths();
        // The places anew: the old places in their new order, and the new place of each old one.
        let mut order = Vec::new();
        // Where each depth's entries start in it (and the last ends).
        let bounds = {
            let mut starts = vec![0u32; DEEPEST as usize + 2];
            for &deep in &depth {
                if deep != LEFT {
                    starts[deep as usize + 1] += 1;
                }
            }
            for at in 1..starts.len() {
                starts[at] += starts[at - 1];
            }
            let bounds = starts.clone();
            order.resize(starts[DEEPEST as usize + 1] as usize, 0u32);
            for (place, &deep) in depth.iter().enumerate() {
                if deep != LEFT {
                    order[starts[deep as usize] as usize] = place as u32;
                    starts[deep as usize] += 1;
                }
            }
            bounds
        };
        drop(depth);
        // Each depth's entries by their folders' new places (those of the depth before, placed already), then as they
        // were; and placed so.
        let parents: Vec<u32> = (0..self.len()).map(|place| self.entry(place).parent).collect();
        let mut placed = vec![NONE; self.len() as usize];
        for deep in 0..=DEEPEST as usize {
            let (from, to) = (bounds[deep] as usize, bounds[deep + 1] as usize);
            let run = &mut order[from..to];
            run.sort_by_key(|&old| if old == ROOT { 0 } else { placed[parents[old as usize] as usize] });
            for (new, &old) in (from..).zip(run.iter()) {
                placed[old as usize] = new as u32;
            }
        }
        drop(parents);
        // The folders kept with their ids (each id one folder's: the one it is known for), and those holding Chinese.
        let mut ids: Vec<(u32, u64)> = Vec::new();
        let mut chinese = 0usize;
        let (mut folded, mut written) = (0usize, 0usize);
        for &old in &order {
            let entry = self.entry(old);
            let (entries, _, _, at) = self.run(old);
            folded += self.folded_name(old).len() + 1;
            if entries[at].written != AS_FOLDED {
                written += entries[at].written_len as usize;
            }
            if entry.flags & flag::CHINESE != 0 {
                chinese += 1;
            }
            if entry.folder() {
                if let Some(id) = self.id(old).filter(|&id| self.known(id) == Some(old)) {
                    ids.push((placed[old as usize], id));
                }
            }
        }
        if folded > u32::MAX as usize || written > u32::MAX as usize {
            return Err(io::Error::other("too many names to keep"));
        }
        let mut out = Counted { out, wrote: 0 };
        for count in [order.len(), folded, written, ids.len(), chinese] {
            out.write_all(&(count as u64).to_le_bytes())?;
        }
        // The entries.
        let (mut name_at, mut written_at) = (0u32, 0u32);
        for &old in &order {
            let entry = self.entry(old);
            let (entries, _, _, at) = self.run(old);
            let parent = if old == ROOT { NONE } else { placed[entry.parent as usize] };
            let (written_from, written_len) = match entries[at].written {
                AS_FOLDED => (AS_FOLDED, 0),
                _ => (written_at, entries[at].written_len),
            };
            written_at += u32::from(written_len);
            for part in [parent, name_at, written_from] {
                out.write_all(&part.to_le_bytes())?;
            }
            out.write_all(&written_len.to_le_bytes())?;
            out.write_all(&(entry.flags & !flag::DEAD).to_le_bytes())?;
            name_at += self.folded_name(old).len() as u32 + 1;
        }
        // The names, folded, then as written.
        for &old in &order {
            out.write_all(self.folded_name(old).as_bytes())?;
            out.write_all(&[0])?;
        }
        out.pad()?;
        for &old in &order {
            let (entries, _, run, at) = self.run(old);
            let entry = entries[at];
            if entry.written != AS_FOLDED {
                out.write_all(&run[entry.written as usize..][..entry.written_len as usize])?;
            }
        }
        out.pad()?;
        // The folders: their places, their ids, their order by id.
        for &(place, _) in &ids {
            out.write_all(&place.to_le_bytes())?;
        }
        out.pad()?;
        for &(_, id) in &ids {
            out.write_all(&id.to_le_bytes())?;
        }
        let mut by_id: Vec<u32> = (0..ids.len() as u32).collect();
        by_id.sort_unstable_by_key(|&at| ids[at as usize].1);
        for at in by_id {
            out.write_all(&at.to_le_bytes())?;
        }
        out.pad()?;
        // Those holding Chinese.
        for (new, &old) in order.iter().enumerate() {
            if self.entry(old).flags & flag::CHINESE != 0 {
                out.write_all(&(new as u32).to_le_bytes())?;
            }
        }
        out.pad()?;
        Ok(())
    }

    /// It as `write` kept it in `bytes`, from `at` to `end` (where its parts are, a multiple of 8 bytes from where
    /// `bytes` start), of a drive whose folders are known `by_id`, given ids up to `made`: each part checked to be
    /// what it can be, and all of them to agree (none where one is not).
    pub fn read(bytes: Bytes, at: usize, end: usize, by_id: bool, made: u64) -> Option<Index> {
        let slice = bytes.slice();
        let mut counts = [0usize; 5];
        for (n, count) in counts.iter_mut().enumerate() {
            *count = usize::try_from(u64::from_le_bytes(slice.get(at + n * 8..at + n * 8 + 8)?.try_into().ok()?)).ok()?;
        }
        let [entries, folded, written, folders, chinese] = counts;
        if entries == 0 || entries >= NONE as usize || folded > u32::MAX as usize || written > u32::MAX as usize || folders > entries || chinese > entries {
            return None;
        }
        // Where each part is.
        let mut next = at + 40;
        let mut part = |len: usize| -> Option<usize> {
            let from = next;
            next = from.checked_add(len)?.next_multiple_of(8);
            Some(from)
        };
        let base = Base {
            entries: (part(entries.checked_mul(16)?)?, entries),
            folded: (part(folded)?, folded),
            written: (part(written)?, written),
            places: (part(folders * 4)?, folders),
            ids: part(folders * 8)?,
            by_id: part(folders * 4)?,
            chinese: (part(chinese * 4)?, chinese),
            bytes,
        };
        if next != end || end > base.bytes.slice().len() {
            return None;
        }
        let (all, names, run) = (cast::<Entry>(base.bytes.slice(), base.entries.0, entries)?, base.folded(), base.written());
        let (places, ids, order, held) = (cast::<u32>(base.bytes.slice(), base.places.0, folders)?, cast::<u64>(base.bytes.slice(), base.ids, folders)?, cast::<u32>(base.bytes.slice(), base.by_id, folders)?, cast::<u32>(base.bytes.slice(), base.chinese.0, chinese)?);
        // Names: each where the one before ended, ended by a 0 and nowhere else, as UTF-8; as written within what is
        // kept, whole UTF-16 units. Each entry after its folder, which is a folder; none dead, the root first.
        std::str::from_utf8(names).ok()?;
        let root = all[ROOT as usize];
        if root.parent != NONE || !root.folder() || root.flags & flag::LINK != 0 {
            return None;
        }
        let mut name_at = 0usize;
        let mut holding = 0usize;
        for (place, entry) in all.iter().enumerate() {
            let name_end = all.get(place + 1).map_or(names.len(), |after| after.at as usize);
            if entry.at as usize != name_at || name_end <= name_at || name_end > names.len() || names[name_end - 1] != 0 || names[name_at..name_end - 1].contains(&0) {
                return None;
            }
            name_at = name_end;
            if entry.written != AS_FOLDED && (entry.written_len == 0 || entry.written_len % 2 != 0 || (entry.written as usize).checked_add(entry.written_len as usize)? > run.len()) {
                return None;
            }
            if entry.written == AS_FOLDED && entry.written_len != 0 {
                return None;
            }
            if entry.flags & !flag::ALL != 0 || !entry.alive() {
                return None;
            }
            if place != ROOT as usize && (entry.parent as usize >= place || !all[entry.parent as usize].folder()) {
                return None;
            }
            // (Those of a folder together, in order of their folders: see `kept_in`.)
            if place > 1 && entry.parent < all[place - 1].parent {
                return None;
            }
            if entry.flags & flag::CHINESE != 0 {
                holding += 1;
            }
        }
        if name_at != names.len() || holding != chinese {
            return None;
        }
        // Folders: in order of their places, each a folder; by id, in order of their ids, each id one folder's.
        if places.windows(2).any(|two| two[0] >= two[1]) || places.iter().any(|&place| place as usize >= entries || !all[place as usize].folder()) {
            return None;
        }
        if order.iter().any(|&at| at as usize >= folders) || order.windows(2).any(|two| ids[two[0] as usize] >= ids[two[1] as usize]) {
            return None;
        }
        if !by_id && ids.iter().any(|&id| id > made) {
            return None;
        }
        if held.windows(2).any(|two| two[0] >= two[1]) || held.iter().any(|&place| place as usize >= entries || all[place as usize].flags & flag::CHINESE == 0) {
            return None;
        }
        Some(Index { base: Some(base), made, ..Index::empty(by_id) })
    }

    /// The ids given folders so far (where the drive keeps none).
    pub fn made(&self) -> u64 {
        self.made
    }
}

/// How deep an entry left out as it is kept is (see `Index::depths`).
const LEFT: u16 = u16::MAX - 1;

/// What is written, and how much of it.
struct Counted<'a, W: Write> {
    out: &'a mut W,
    wrote: usize,
}

impl<W: Write> Counted<'_, W> {
    /// Zeros up to the next multiple of 8 bytes.
    fn pad(&mut self) -> io::Result<()> {
        let more = self.wrote.next_multiple_of(8) - self.wrote;
        self.write_all(&[0; 8][..more])
    }
}

impl<W: Write> Write for Counted<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let wrote = self.out.write(bytes)?;
        self.wrote += wrote;
        Ok(wrote)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: u32 = 0x20;
    const FOLDER: u32 = ATTRIBUTE_DIRECTORY;

    fn item(name: &str, id: u64, attributes: u32) -> Item {
        Item { name: name.encode_utf16().collect(), id, attributes, tag: 0 }
    }

    /// One round of listing `folders` again, each as `listed` says it is now (as `walk::round` does it).
    fn round(index: &mut Index, folders: &[(u32, Vec<Item>)]) -> Vec<(u32, u64)> {
        let before = index.children(&folders.iter().map(|(folder, _)| *folder).collect());
        let mut changes = Changes::default();
        for (folder, items) in folders {
            index.list(*folder, items, before.get(folder).map_or(&[], Vec::as_slice), &mut changes);
        }
        let to_list = std::mem::take(&mut changes.to_list);
        index.finish(changes);
        to_list
    }

    fn paths(index: &Index) -> Vec<String> {
        let mut paths: Vec<String> = (1..index.len()).filter(|&place| index.entry(place).alive()).filter_map(|place| index.path(place, "C:\\")).map(|path| path.display().to_string()).collect();
        paths.sort();
        paths
    }

    fn place_of(index: &Index, path: &str) -> u32 {
        (0..index.len()).find(|&place| index.entry(place).alive() && index.path(place, "C:\\").is_some_and(|at| at.display().to_string() == path)).unwrap()
    }

    /// `index` as kept, and read again.
    fn kept(index: &Index) -> Index {
        let mut bytes = Vec::new();
        index.write(&mut bytes).unwrap();
        let end = bytes.len();
        Index::read(Bytes::owned(&bytes), 0, end, index.by_id, index.made).unwrap()
    }

    /// C:\Docs\Plan.txt, C:\Docs\Old\a.txt, C:\Music\歌.mp3.
    fn drive() -> Index {
        let mut index = Index::new(5, true);
        let found = round(&mut index, &[(ROOT, vec![item("Docs", 10, FOLDER), item("Music", 11, FOLDER), item("boot.ini", 12, FILE | ATTRIBUTE_HIDDEN)])]);
        assert_eq!(found.len(), 2);
        let docs = place_of(&index, "C:\\Docs");
        let music = place_of(&index, "C:\\Music");
        round(&mut index, &[(docs, vec![item("Plan.txt", 20, FILE), item("Old", 21, FOLDER)]), (music, vec![item("歌.mp3", 22, FILE)])]);
        let old = place_of(&index, "C:\\Docs\\Old");
        round(&mut index, &[(old, vec![item("a.txt", 30, FILE)])]);
        index
    }

    /// The drive's folders changed as each round says, on `index` (as built, or as kept and read again: the same).
    fn folders_listed_again_change_as_the_drive_did(mut index: Index) {
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\Old", "C:\\Docs\\Old\\a.txt", "C:\\Docs\\Plan.txt", "C:\\Music", "C:\\Music\\歌.mp3", "C:\\boot.ini"]);
        assert!(index.entry(place_of(&index, "C:\\boot.ini")).flags & flag::HIDDEN != 0);
        assert!(index.entry(place_of(&index, "C:\\Music\\歌.mp3")).flags & flag::CHINESE != 0);

        // Old moved into Music as Older, Plan.txt renamed, Music's song deleted: Old's file goes with it.
        let (docs, music) = (place_of(&index, "C:\\Docs"), place_of(&index, "C:\\Music"));
        let found = round(&mut index, &[(docs, vec![item("plan v2.txt", 20, FILE)]), (music, vec![item("Older", 21, FOLDER)])]);
        assert!(found.is_empty(), "a folder moved keeps what it holds, and is not listed again");
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\plan v2.txt", "C:\\Music", "C:\\Music\\Older", "C:\\Music\\Older\\a.txt", "C:\\boot.ini"]);
        assert_eq!(index.folder(21), Some(place_of(&index, "C:\\Music\\Older")));

        // Moved in a round listing only where it went (its old folder listed after): the same.
        let (docs, music) = (place_of(&index, "C:\\Docs"), place_of(&index, "C:\\Music"));
        round(&mut index, &[(docs, vec![item("plan v2.txt", 20, FILE), item("Older", 21, FOLDER)]), (music, vec![])]);
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\Older", "C:\\Docs\\Older\\a.txt", "C:\\Docs\\plan v2.txt", "C:\\Music", "C:\\boot.ini"]);

        // A folder deleted, and another made with its name: what it held is gone, the new one is listed.
        let docs = place_of(&index, "C:\\Docs");
        let found = round(&mut index, &[(docs, vec![item("plan v2.txt", 20, FILE), item("Older", 40, FOLDER)])]);
        assert_eq!(found, [(place_of(&index, "C:\\Docs\\Older"), 40)]);
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\Older", "C:\\Docs\\plan v2.txt", "C:\\Music", "C:\\boot.ini"]);
        assert_eq!(index.folder(21), None);

        // Docs gone whole.
        round(&mut index, &[(ROOT, vec![item("Music", 11, FOLDER), item("boot.ini", 12, FILE)])]);
        assert_eq!(paths(&index), ["C:\\Music", "C:\\boot.ini"]);
        assert_eq!(index.names(), 2);
        assert_eq!(index.folder(10), None);
        assert_eq!(index.folder(11), Some(place_of(&index, "C:\\Music")));
    }

    #[test]
    fn folders_listed_again_change_as_the_drive_did_in_memory() {
        folders_listed_again_change_as_the_drive_did(drive());
    }

    /// What changes since it was kept is kept over it, and found as the rest is.
    #[test]
    fn folders_listed_again_change_as_the_drive_did_over_what_was_kept() {
        let index = kept(&drive());
        assert!(index.entries.is_empty(), "all of it read where it lies");
        folders_listed_again_change_as_the_drive_did(index);
    }

    /// What each folder holds, looked through every entry.
    fn held_looked_through(index: &Index, folders: &HashSet<u32>) -> HashMap<u32, Vec<u32>> {
        let mut held: HashMap<u32, Vec<u32>> = HashMap::new();
        for place in 0..index.len() {
            let entry = index.entry(place);
            if entry.alive() && folders.contains(&entry.parent) {
                held.entry(entry.parent).or_default().push(place);
            }
        }
        held
    }

    /// What a folder holds is looked up, not looked for through every entry (each round of listing asked it of the
    /// whole drive, a million entries, every second something changed): the same as looked through, in memory, as
    /// kept (each folder's entries together there, though added to it at different times), and over what was kept.
    #[test]
    fn what_a_folder_holds_is_looked_up_as_it_is_now() {
        let mut index = Index::new(5, true);
        round(&mut index, &[(ROOT, vec![item("A", 10, FOLDER), item("B", 11, FOLDER)])]);
        let (a, b) = (place_of(&index, "C:\\A"), place_of(&index, "C:\\B"));
        round(&mut index, &[(a, vec![item("a1", 20, FILE), item("Deep", 21, FOLDER)]), (b, vec![item("b1", 22, FILE)])]);
        // (a2 added after b1: A's entries are not together in memory.)
        round(&mut index, &[(a, vec![item("a1", 20, FILE), item("Deep", 21, FOLDER), item("a2", 23, FILE)])]);
        let deep = place_of(&index, "C:\\A\\Deep");
        round(&mut index, &[(deep, vec![item("d1", 30, FILE)])]);
        let all = |index: &Index| (0..index.len()).filter(|&place| index.entry(place).alive() && index.entry(place).folder()).collect::<HashSet<u32>>();
        assert_eq!(index.children(&all(&index)), held_looked_through(&index, &all(&index)));

        let mut index = kept(&index);
        let parents: Vec<u32> = (1..index.len()).map(|place| index.entry(place).parent).collect();
        assert!(parents.windows(2).all(|two| two[0] <= two[1]), "kept in order of their folders");
        assert_eq!(index.children(&all(&index)), held_looked_through(&index, &all(&index)));

        // Over what was kept: Deep moved into B, a1 gone, b2 added, then B deleted with all it holds.
        let (a, b) = (place_of(&index, "C:\\A"), place_of(&index, "C:\\B"));
        round(&mut index, &[(a, vec![item("a2", 23, FILE)]), (b, vec![item("b1", 22, FILE), item("Deep", 21, FOLDER), item("b2", 24, FILE)])]);
        assert_eq!(paths(&index), ["C:\\A", "C:\\A\\a2", "C:\\B", "C:\\B\\Deep", "C:\\B\\Deep\\d1", "C:\\B\\b1", "C:\\B\\b2"]);
        assert_eq!(index.children(&all(&index)), held_looked_through(&index, &all(&index)));
        assert_eq!(index.folder_at(std::path::Path::new("B\\Deep")), place_of(&index, "C:\\B\\Deep"));
        round(&mut index, &[(ROOT, vec![item("A", 10, FOLDER)])]);
        assert_eq!(paths(&index), ["C:\\A", "C:\\A\\a2"]);
        assert_eq!(index.children(&all(&index)), held_looked_through(&index, &all(&index)));
        assert_eq!(paths(&kept(&index)), ["C:\\A", "C:\\A\\a2"]);
    }

    #[test]
    fn a_drive_keeping_no_ids_knows_its_folders_by_name() {
        let mut index = Index::new(5, false);
        round(&mut index, &[(ROOT, vec![item("Docs", 0, FOLDER)])]);
        let docs = place_of(&index, "C:\\Docs");
        round(&mut index, &[(docs, vec![item("a.txt", 0, FILE)])]);
        let found = round(&mut index, &[(ROOT, vec![item("Docs", 0, FOLDER), item("New", 0, FOLDER)])]);
        assert_eq!(found.len(), 1, "only the new folder is listed");
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\a.txt", "C:\\New"]);
        assert_eq!(index.folder_at(std::path::Path::new("Docs")), docs);
        assert_ne!(index.id(docs), index.id(place_of(&index, "C:\\New")));
        // Kept and read again, the ids given before are not given again.
        let mut index = kept(&index);
        let docs = place_of(&index, "C:\\Docs");
        round(&mut index, &[(docs, vec![item("a.txt", 0, FILE), item("Sub", 0, FOLDER)])]);
        let ids: HashSet<u64> = ["C:\\Docs", "C:\\New", "C:\\Docs\\Sub"].iter().filter_map(|path| index.id(place_of(&index, path))).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn denied_folders_hold_nothing() {
        let mut index = drive();
        let docs = place_of(&index, "C:\\Docs");
        let before = index.children(&HashSet::from([docs]));
        index.denied(docs, &before[&docs]);
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Music", "C:\\Music\\歌.mp3", "C:\\boot.ini"]);
        assert!(index.entry(docs).flags & flag::UNREAD != 0);
    }

    /// Kept, the dead are left out, each entry is placed after its folder (one moved was placed after what it holds),
    /// names are as written, and what was read is the same as what was kept, but where its parts do not agree.
    #[test]
    fn kept_and_read_it_is_the_same() {
        let mut index = drive();
        let (docs, music) = (place_of(&index, "C:\\Docs"), place_of(&index, "C:\\Music"));
        round(&mut index, &[(docs, vec![item("Plan.txt", 20, FILE)]), (music, vec![item("歌.mp3", 22, FILE), item("Older", 21, FOLDER)])]);
        let was = paths(&index);
        assert!(index.entry(place_of(&index, "C:\\Music\\Older\\a.txt")).parent > place_of(&index, "C:\\Music\\Older\\a.txt"));
        let read = kept(&index);
        assert_eq!(paths(&read), was);
        assert_eq!(read.len() as usize, read.names() + 1);
        assert!((1..read.len()).all(|place| read.entry(place).parent < place));
        assert_eq!(read.folder(21), Some(place_of(&read, "C:\\Music\\Older")));
        assert_eq!(read.name(place_of(&read, "C:\\Music\\Older")), OsStr::new("Older"));
        assert_eq!(read.name(place_of(&read, "C:\\Docs\\Plan.txt")), OsStr::new("Plan.txt"));
        assert_eq!(read.chinese().count(), 1);
        // Kept again (read where it lies, and changed over it), the same again.
        assert_eq!(paths(&kept(&read)), was);

        let mut bytes = Vec::new();
        index.write(&mut bytes).unwrap();
        let end = bytes.len();
        assert!(Index::read(Bytes::owned(&bytes), 0, end - 8, true, 0).is_none(), "cut short");
        // (A name's end moved: the names are not where they were.)
        let mut moved = bytes.clone();
        moved[40 + 16 + 4] ^= 1;
        assert!(Index::read(Bytes::owned(&moved), 0, end, true, 0).is_none());
        // (An entry placed before its folder.)
        let mut before = bytes.clone();
        before[40 + 16..40 + 20].copy_from_slice(&3u32.to_le_bytes());
        assert!(Index::read(Bytes::owned(&before), 0, end, true, 0).is_none());
    }

    /// A name that is not whole Unicode (an unpaired surrogate, as NTFS allows) is kept as it is written.
    #[test]
    fn names_are_kept_as_written() {
        let mut index = Index::new(5, true);
        let odd = Item { name: vec![0x41, 0xD800, 0x62], id: 9, attributes: FILE, tag: 0 };
        let mut changes = Changes::default();
        index.list(ROOT, &[odd, item("ÄBC.txt", 10, FILE)], &[], &mut changes);
        index.finish(changes);
        let read = kept(&index);
        let names: Vec<Vec<u16>> = (1..read.len()).map(|place| read.name(place).encode_wide().collect()).collect();
        assert!(names.contains(&vec![0x41, 0xD800, 0x62]));
        assert!(names.contains(&"ÄBC.txt".encode_utf16().collect()));
    }

    #[test]
    fn names_are_found_on_any_number_of_threads() {
        let mut index = Index::new(5, true);
        let items: Vec<Item> = (0..5000).map(|n| item(&format!("File {n}.TXT"), n, FILE)).collect();
        round(&mut index, &[(ROOT, items)]);
        let one = index.holding("12", 1);
        assert_eq!(one.len(), (0..5000).filter(|n| n.to_string().contains("12")).count());
        for threads in [2, 3, 7, 16] {
            assert_eq!(index.holding("12", threads), one);
        }
        assert_eq!(index.holding("file 4999.txt", 4).len(), 1);
        assert_eq!(index.name(index.holding("file 4999.txt", 4)[0]), OsStr::new("File 4999.TXT"));
        // Kept, and more added over it: found in both.
        let mut index = kept(&index);
        let more: Vec<Item> = (0..5000).map(|n| item(&format!("File {n}.TXT"), n, FILE)).chain((5000..5100).map(|n| item(&format!("File {n}.TXT"), n, FILE))).collect();
        round(&mut index, &[(ROOT, more)]);
        assert_eq!(index.holding("file 50", 4).len(), (0..5100).filter(|n| n.to_string().starts_with("50")).count());
    }
}
