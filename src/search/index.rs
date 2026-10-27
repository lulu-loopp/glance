//! A drive's files and folders, every name in each folder the user can list: one entry for each name (a file with
//! several names, hard links, has an entry for each), each knowing its folder. Their names case folded lie in one run,
//! each ended by a 0 (which no name holds), searched whole at once; a name is kept as written apart only where it is
//! not as folded. Entries are only ever added: one gone, or renamed, is marked dead (a renamed one added again) and the
//! dead are cleared out now and then.

use super::readings;
use memchr::memmem::Finder;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

/// The drive's root folder's place.
pub const ROOT: u32 = 0;

/// No place: the root's folder.
pub const NONE: u32 = u32::MAX;

/// Where an entry's name is as written in `Index::folded` (case folded, it is the same).
const AS_FOLDED: u32 = u32::MAX;

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
}

/// A file or folder (16 bytes).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Entry {
    /// Its folder's place.
    pub parent: u32,
    /// Where its name, case folded, starts in `Index::folded`.
    at: u32,
    /// Where its name as written is in `Index::written`, and how long it is, where it is not as folded.
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

pub struct Index {
    entries: Vec<Entry>,
    folded: Vec<u8>,
    written: Vec<u8>,
    /// The folders' places by their file ids, and their ids by their places (in the order of the places; entries
    /// only being added, an id is added after those before).
    folders: HashMap<u64, u32>,
    ids: Vec<(u32, u64)>,
    /// The entries whose names hold Chinese characters.
    chinese: Vec<u32>,
    dead: usize,
    /// Whether its folders are known by their file ids (an NTFS drive's), or else by their names alone, each
    /// given an id of its own from `made`.
    by_id: bool,
    made: u64,
}

/// The first id given a folder by `Index` (where the drive's are not kept).
const MADE: u64 = 1 << 63;

impl Index {
    fn empty(by_id: bool) -> Self {
        Index { entries: Vec::new(), folded: Vec::new(), written: Vec::new(), folders: HashMap::new(), ids: Vec::new(), chinese: Vec::new(), dead: 0, by_id, made: MADE }
    }

    /// An index of a drive whose root folder has `root_id` (a drive whose folders are known `by_id`, else by their
    /// names), with nothing in it yet.
    pub fn new(root_id: u64, by_id: bool) -> Self {
        let mut index = Index::empty(by_id);
        index.add(NONE, &[], flag::FOLDER);
        index.set_id(ROOT, root_id);
        index
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many names it holds (the live entries but the root).
    pub fn names(&self) -> usize {
        self.entries.len() - self.dead - 1
    }

    fn add(&mut self, parent: u32, name: &[u16], flags: u16) -> u32 {
        let place = self.entries.len() as u32;
        let name = OsString::from_wide(name);
        let lossy = name.to_string_lossy();
        let at = self.folded.len();
        for c in lossy.chars().flat_map(char::to_lowercase) {
            self.folded.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        }
        let as_written = name.as_encoded_bytes();
        let (written, written_len) = if self.folded[at..] == *as_written {
            (AS_FOLDED, 0)
        } else {
            self.written.extend_from_slice(as_written);
            ((self.written.len() - as_written.len()) as u32, as_written.len() as u16)
        };
        self.folded.push(0);
        let mut flags = flags;
        if lossy.chars().any(readings::read) {
            flags |= flag::CHINESE;
            self.chinese.push(place);
        }
        self.entries.push(Entry { parent, at: at as u32, written, written_len, flags });
        place
    }

    fn make_id(&mut self) -> u64 {
        self.made += 1;
        self.made
    }

    pub fn by_id(&self) -> bool {
        self.by_id
    }

    fn set_id(&mut self, place: u32, id: u64) {
        self.folders.insert(id, place);
        self.ids.push((place, id));
    }

    /// A folder's file id.
    pub fn id(&self, place: u32) -> Option<u64> {
        self.ids.binary_search_by_key(&place, |&(place, _)| place).ok().map(|at| self.ids[at].1)
    }

    /// The live folder of this file id.
    pub fn folder(&self, id: u64) -> Option<u32> {
        self.folders.get(&id).copied().filter(|&place| self.entries[place as usize].alive())
    }

    /// An entry's name as written.
    pub fn name(&self, place: u32) -> &OsStr {
        let entry = &self.entries[place as usize];
        if entry.written == AS_FOLDED {
            OsStr::new(self.folded_name(place))
        } else {
            let bytes = &self.written[entry.written as usize..][..entry.written_len as usize];
            // (Bytes as `as_encoded_bytes` gave them.)
            unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }
        }
    }

    /// An entry's name case folded.
    pub fn folded_name(&self, place: u32) -> &str {
        let at = self.entries[place as usize].at as usize;
        let end = self.entries.get(place as usize + 1).map_or(self.folded.len(), |next| next.at as usize) - 1;
        // (Folded from a str.)
        std::str::from_utf8(&self.folded[at..end]).unwrap()
    }

    /// An entry's whole path, from `root` (the drive's, "C:\"); none where a folder on the way is unknown or gone.
    pub fn path(&self, place: u32, root: &str) -> Option<PathBuf> {
        let mut parts = Vec::new();
        let mut at = place;
        while at != ROOT {
            let entry = self.entries.get(at as usize)?;
            if !entry.alive() || parts.len() > 4096 {
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
            let found = self.entries.iter().enumerate().find(|(place, entry)| entry.alive() && entry.folder() && entry.parent == folder && self.name(*place as u32) == part.as_os_str());
            match found {
                Some((place, _)) => folder = place as u32,
                None => break,
            }
        }
        folder
    }

    pub fn set_flag(&mut self, place: u32, flag: u16, on: bool) {
        let flags = &mut self.entries[place as usize].flags;
        *flags = if on { *flags | flag } else { *flags & !flag };
    }

    fn kill(&mut self, place: u32) {
        let entry = &mut self.entries[place as usize];
        if !entry.alive() {
            return;
        }
        entry.flags |= flag::DEAD;
        self.dead += 1;
        if entry.folder() {
            if let Some(id) = self.id(place) {
                if self.folders.get(&id) == Some(&place) {
                    self.folders.remove(&id);
                }
            }
        }
    }

    /// The live entries in each of `folders`.
    pub fn children(&self, folders: &HashSet<u32>) -> HashMap<u32, Vec<u32>> {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (place, entry) in self.entries.iter().enumerate() {
            if entry.alive() && folders.contains(&entry.parent) {
                children.entry(entry.parent).or_default().push(place as u32);
            }
        }
        children
    }

    /// `folder` as it is listed now, against `before`, its entries when the round began: names gone marked dead
    /// (a folder only once the round is finished, as it may have moved), new ones added, folders found that are to
    /// be listed noted in `changes`.
    pub fn list(&mut self, folder: u32, items: &[Item], before: &[u32], changes: &mut Changes) {
        let mut by_name: HashMap<OsString, u32> = before.iter().filter(|&&place| self.entries[place as usize].alive()).map(|&place| (self.name(place).to_owned(), place)).collect();
        for item in items {
            let name = OsString::from_wide(&item.name);
            let flags = item.flags();
            // (Where the drive keeps no ids, a folder is the one of its name.)
            let id = if self.by_id { item.id } else { by_name.get(&name).and_then(|&child| self.id(child)).unwrap_or_else(|| self.make_id()) };
            if let Some(child) = by_name.remove(&name) {
                let was = self.entries[child as usize];
                let same = match (was.folder(), item.folder()) {
                    (true, true) => self.id(child) == Some(id),
                    (false, false) => true,
                    _ => false,
                };
                if same {
                    let kept = was.flags & (flag::FOLDER | flag::CHINESE | flag::UNREAD);
                    self.entries[child as usize].flags = kept | flags;
                    // (A link before, its names are listed now.)
                    if was.flags & flag::LINK != 0 && item.followed() {
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
                    let place = self.add(folder, &item.name, flags | (self.entries[old as usize].flags & flag::UNREAD));
                    self.set_id(place, id);
                    self.kill(old);
                    changes.moved.insert(old, place);
                    continue;
                }
            }
            let place = self.add(folder, &item.name, flags);
            if item.folder() {
                self.set_id(place, id);
                if item.followed() {
                    changes.to_list.push((place, id));
                }
            }
        }
        for (_, place) in by_name {
            if self.entries[place as usize].folder() {
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
            for entry in &mut self.entries {
                if entry.alive() && changes.moved.contains_key(&entry.parent) {
                    entry.parent = moved(entry.parent);
                }
            }
        }
        let mut gone: HashSet<u32> = HashSet::new();
        for place in changes.missing {
            if self.entries[place as usize].alive() {
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
            for place in 0..self.entries.len() as u32 {
                let entry = self.entries[place as usize];
                if entry.alive() && folders.contains(&entry.parent) {
                    self.kill(place);
                    if entry.folder() {
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
            if self.entries[place as usize].alive() {
                self.kill(place);
                if self.entries[place as usize].folder() {
                    gone.insert(place);
                }
            }
        }
        self.kill_within(gone);
        self.set_flag(folder, flag::UNREAD, true);
    }

    /// Whether enough is dead to clear it out.
    pub fn untidy(&self) -> bool {
        self.dead > 65_536 && self.dead * 4 > self.entries.len()
    }

    /// The dead cleared out, the live ones' places changed (none is kept across this).
    pub fn tidy(&mut self) {
        // (Whatever a dead folder held is dead too: the live are all reached from the root.)
        let mut placed = vec![NONE; self.entries.len()];
        let mut kept = 0u32;
        for (place, entry) in self.entries.iter().enumerate() {
            if entry.alive() {
                placed[place] = kept;
                kept += 1;
            }
        }
        let old = std::mem::replace(self, Index::empty(self.by_id));
        self.made = old.made;
        self.entries.reserve(kept as usize);
        for (place, entry) in old.entries.iter().enumerate() {
            if placed[place] == NONE {
                continue;
            }
            let new = self.entries.len() as u32;
            let at = self.folded.len() as u32;
            self.folded.extend_from_slice(old.folded_name(place as u32).as_bytes());
            self.folded.push(0);
            let written = if entry.written == AS_FOLDED {
                AS_FOLDED
            } else {
                self.written.extend_from_slice(&old.written[entry.written as usize..][..entry.written_len as usize]);
                (self.written.len() - entry.written_len as usize) as u32
            };
            // (A folder may be placed after what it holds, moved: each entry's folder is found by its old place.)
            let parent = if entry.parent == NONE { NONE } else { placed[entry.parent as usize] };
            self.entries.push(Entry { parent, at, written, written_len: entry.written_len, flags: entry.flags });
            if entry.flags & flag::CHINESE != 0 {
                self.chinese.push(new);
            }
            if entry.folder() {
                if let Some(id) = old.id(place as u32) {
                    self.set_id(new, id);
                }
            }
        }
        self.entries.shrink_to_fit();
        self.folded.shrink_to_fit();
        self.written.shrink_to_fit();
    }

    /// The places of the live entries whose folded names hold `words` (folded), in order, looked for by `threads`
    /// threads at once, each through its share of the names.
    pub fn holding(&self, words: &str, threads: usize) -> Vec<u32> {
        let finder = Finder::new(words.as_bytes());
        let mut bounds = vec![0usize];
        for share in 1..threads.max(1) {
            let from = self.folded.len() * share / threads;
            let start = self.folded[from..].iter().position(|&b| b == 0).map_or(self.folded.len(), |end| from + end + 1);
            if start > *bounds.last().unwrap() {
                bounds.push(start);
            }
        }
        bounds.push(self.folded.len());
        let look = |from: usize, to: usize| {
            let mut found = Vec::new();
            let mut at = from;
            while let Some(hit) = finder.find(&self.folded[at..to]) {
                let place = self.entries.partition_point(|entry| entry.at as usize <= at + hit) - 1;
                if self.entries[place].alive() {
                    found.push(place as u32);
                }
                at = self.entries.get(place + 1).map_or(to, |next| next.at as usize);
            }
            found
        };
        if bounds.len() == 2 {
            return look(0, self.folded.len());
        }
        std::thread::scope(|scope| {
            let looks: Vec<_> = bounds.windows(2).map(|share| scope.spawn(move || look(share[0], share[1]))).collect();
            looks.into_iter().flat_map(|looked| looked.join().unwrap()).collect()
        })
    }

    /// The places of the live entries whose names hold Chinese characters.
    pub fn chinese(&self) -> impl Iterator<Item = u32> + '_ {
        self.chinese.iter().copied().filter(|&place| self.entries[place as usize].alive())
    }

    /// It as kept on disk: how many of each it holds, then each (all little-endian).
    pub fn to_bytes(&self, out: &mut Vec<u8>) {
        for count in [self.entries.len(), self.folded.len(), self.written.len(), self.ids.len()] {
            out.extend_from_slice(&(count as u64).to_le_bytes());
        }
        out.reserve(self.entries.len() * 16 + self.folded.len() + self.written.len() + self.ids.len() * 12);
        for entry in &self.entries {
            out.extend_from_slice(&entry.parent.to_le_bytes());
            out.extend_from_slice(&entry.at.to_le_bytes());
            out.extend_from_slice(&entry.written.to_le_bytes());
            out.extend_from_slice(&entry.written_len.to_le_bytes());
            out.extend_from_slice(&entry.flags.to_le_bytes());
        }
        out.extend_from_slice(&self.folded);
        out.extend_from_slice(&self.written);
        for &(place, id) in &self.ids {
            out.extend_from_slice(&place.to_le_bytes());
            out.extend_from_slice(&id.to_le_bytes());
        }
    }

    /// It as `to_bytes` kept it, each part checked to be what it can be (none where one is not).
    pub fn from_bytes(bytes: &[u8]) -> Option<Index> {
        let mut at = 0usize;
        let mut take = |len: usize| {
            let part = bytes.get(at..at.checked_add(len)?)?;
            at += len;
            Some(part)
        };
        let mut counts = [0usize; 4];
        for count in &mut counts {
            *count = usize::try_from(u64::from_le_bytes(take(8)?.try_into().ok()?)).ok()?;
        }
        let [entries, folded, written, ids] = counts;
        let u32_at = |part: &[u8], at: usize| u32::from_le_bytes(part[at..at + 4].try_into().unwrap());
        let u16_at = |part: &[u8], at: usize| u16::from_le_bytes(part[at..at + 2].try_into().unwrap());
        let entries: Vec<Entry> = take(entries.checked_mul(16)?)?
            .chunks_exact(16)
            .map(|part| Entry { parent: u32_at(part, 0), at: u32_at(part, 4), written: u32_at(part, 8), written_len: u16_at(part, 12), flags: u16_at(part, 14) })
            .collect();
        let folded = take(folded)?.to_vec();
        let written = take(written)?.to_vec();
        let ids: Vec<(u32, u64)> = take(ids.checked_mul(12)?)?.chunks_exact(12).map(|part| (u32_at(part, 0), u64::from_le_bytes(part[4..12].try_into().unwrap()))).collect();
        if at != bytes.len() || entries.is_empty() || entries[ROOT as usize].parent != NONE {
            return None;
        }
        // Names: each where the one before ended, ended by a 0 and nowhere else, as UTF-8; as written within what
        // is kept; folders within the entries.
        std::str::from_utf8(&folded).ok()?;
        let mut next = 0usize;
        for (place, entry) in entries.iter().enumerate() {
            let end = entries.get(place + 1).map_or(folded.len(), |after| after.at as usize);
            if entry.at as usize != next || end <= next || end > folded.len() || folded[end - 1] != 0 || folded[next..end - 1].contains(&0) {
                return None;
            }
            next = end;
            if entry.written != AS_FOLDED && (entry.written as usize).checked_add(entry.written_len as usize)? > written.len() {
                return None;
            }
            if place != ROOT as usize && entry.parent as usize >= entries.len() {
                return None;
            }
        }
        if next != folded.len() || ids.windows(2).any(|two| two[0].0 >= two[1].0) || ids.iter().any(|&(place, _)| place as usize >= entries.len()) {
            return None;
        }
        let mut index = Index { entries, folded, written, ids, ..Index::empty(true) };
        for (place, entry) in index.entries.iter().enumerate() {
            if !entry.alive() {
                index.dead += 1;
            } else if entry.flags & flag::CHINESE != 0 {
                index.chinese.push(place as u32);
            }
        }
        for &(place, id) in &index.ids {
            if index.entries[place as usize].alive() {
                index.folders.insert(id, place);
            }
        }
        Some(index)
    }

    /// About how much memory it holds.
    pub fn bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.folded.capacity()
            + self.written.capacity()
            + self.folders.capacity() * (size_of::<(u64, u32)>() + 1)
            + self.ids.capacity() * size_of::<(u32, u64)>()
            + self.chinese.capacity() * 4
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
        let mut paths: Vec<String> = (1..index.entries().len() as u32).filter(|&place| index.entries()[place as usize].alive()).filter_map(|place| index.path(place, "C:\\")).map(|path| path.display().to_string()).collect();
        paths.sort();
        paths
    }

    fn place_of(index: &Index, path: &str) -> u32 {
        (0..index.entries().len() as u32).find(|&place| index.entries()[place as usize].alive() && index.path(place, "C:\\").is_some_and(|at| at.display().to_string() == path)).unwrap()
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

    #[test]
    fn folders_listed_again_change_as_the_drive_did() {
        let mut index = drive();
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Docs\\Old", "C:\\Docs\\Old\\a.txt", "C:\\Docs\\Plan.txt", "C:\\Music", "C:\\Music\\歌.mp3", "C:\\boot.ini"]);
        assert!(index.entries()[place_of(&index, "C:\\boot.ini") as usize].flags & flag::HIDDEN != 0);
        assert!(index.entries()[place_of(&index, "C:\\Music\\歌.mp3") as usize].flags & flag::CHINESE != 0);

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
    }

    #[test]
    fn denied_folders_hold_nothing() {
        let mut index = drive();
        let docs = place_of(&index, "C:\\Docs");
        let before = index.children(&HashSet::from([docs]));
        index.denied(docs, &before[&docs]);
        assert_eq!(paths(&index), ["C:\\Docs", "C:\\Music", "C:\\Music\\歌.mp3", "C:\\boot.ini"]);
        assert!(index.entries()[docs as usize].flags & flag::UNREAD != 0);
    }

    #[test]
    fn tidied_and_kept_it_is_the_same() {
        let mut index = drive();
        // (Older is placed after what it holds.)
        let (docs, music) = (place_of(&index, "C:\\Docs"), place_of(&index, "C:\\Music"));
        round(&mut index, &[(docs, vec![item("Plan.txt", 20, FILE)]), (music, vec![item("歌.mp3", 22, FILE), item("Older", 21, FOLDER)])]);
        let was = paths(&index);
        index.tidy();
        assert_eq!(paths(&index), was);
        assert_eq!(index.entries().len(), index.names() + 1);
        assert_eq!(index.folder(21), Some(place_of(&index, "C:\\Music\\Older")));
        assert_eq!(index.chinese().count(), 1);

        let mut bytes = Vec::new();
        index.to_bytes(&mut bytes);
        let read = Index::from_bytes(&bytes).unwrap();
        assert_eq!(paths(&read), was);
        assert_eq!(read.folder(21), index.folder(21));
        assert_eq!(read.name(place_of(&read, "C:\\Music\\Older")), "Older");
        assert!(Index::from_bytes(&bytes[..bytes.len() - 1]).is_none());
        // (A name's end moved: the names are not where they were.)
        bytes[32 + 16 + 4] ^= 1;
        assert!(Index::from_bytes(&bytes).is_none());
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
        assert_eq!(index.name(index.holding("file 4999.txt", 4)[0]), "File 4999.TXT");
    }
}
