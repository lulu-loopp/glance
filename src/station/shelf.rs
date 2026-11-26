//! What the station holds: the things put on it, each as what it is (files where they are, or what it keeps a copy of
//! itself: text, a link, a picture), when it was put there, whether it is pinned, and its tags; and the rules it keeps
//! them by.
//!
//! - Pinned, a thing is listed first on its page. Tagged, it is sorted under its tags. The one is nothing to the other.
//! - What is pinned or tagged stays (see `Item::stays`). What is neither goes after a while (`Rules::keep_days`, 7 by
//!   default), counted from when it last was either (see `Item::freed_ms`).
//! - What the station keeps a copy of itself takes no more than `Rules::most_bytes` (2 GB by default): to make room,
//!   what it keeps of the oldest things that do not stay goes first; what stays never goes, and what is put on it is
//!   never dropped unsaid: with no room left even then, it is not taken, and the person is told the station is full.
//!
//! Tags are told apart by their own numbers, never given to another. The files come lately are sorted under the same tags
//! (see `super::recent`).
//!
//! Only the shelf: its files are the folder's (see `super::folder`), its windows the station's thread's.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One thing on the station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Its own number, never given to another on this shelf.
    pub id: u64,
    pub what: What,
    /// When it was put there (ms since 1970).
    pub added_ms: u64,
    #[serde(default)]
    pub pinned: bool,
    /// Copied to the clipboard (in Recently copied), not put there by hand.
    #[serde(default)]
    pub copied: bool,
    /// Not to leave this PC (its copier said so: `CanUploadToCloudClipboard` 0): copied back with that said again, and
    /// never sent to the phone.
    #[serde(default)]
    pub stays_here: bool,
    /// The bytes of what the station keeps of it in its own folder (none for files only pointed to).
    #[serde(default)]
    pub own_bytes: u64,
    /// The device nearby it was sent from (see `nearby`), where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Device>,
    /// When it was last put on the clipboard from here, or copied again (ms since 1970; 0: not since it came).
    #[serde(default, skip_serializing_if = "never")]
    pub used_ms: u64,
    /// Its tags, by their numbers (see `Tag`), in the order they were put on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<u32>,
    /// Holds a file pinned or tagged among the files come lately (sent from a device nearby, kept here: see
    /// `recent::Came::from`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    /// When it last stopped staying (ms since 1970, 0: it never has): its days counted from then.
    #[serde(default, skip_serializing_if = "never")]
    pub freed_ms: u64,
}

/// Whether a time is none (see `Item::used_ms`).
fn never(ms: &u64) -> bool {
    *ms == 0
}

impl Item {
    /// When it was last used: put on the clipboard, or come (ms since 1970).
    pub fn last_ms(&self) -> u64 {
        self.used_ms.max(self.added_ms)
    }

    /// Whether it stays: pinned, tagged, or holding a file that is (see `held`).
    pub fn stays(&self) -> bool {
        self.pinned || !self.tags.is_empty() || self.held
    }

    /// Since when its days are counted, and what is oldest told by (ms since 1970): when it came, or stopped staying.
    fn since_ms(&self) -> u64 {
        self.added_ms.max(self.freed_ms)
    }

    /// `change` made to it at `now_ms`, which may be what it stays by.
    fn change(&mut self, now_ms: u64, change: impl FnOnce(&mut Item)) {
        let stayed = self.stays();
        change(self);
        if stayed && !self.stays() {
            self.freed_ms = now_ms;
        }
    }
}

/// A tag things are sorted by (on the station, copied lately, and files come lately): its own number, and its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub id: u32,
    pub name: String,
}

/// A device nearby files came from, as it called itself, of what kind (LocalSend's `mobile`, `desktop`...), and by the
/// fingerprint of its certificate (its own beyond doubt where it presented the certificate: see `nearby`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub fingerprint: String,
}

/// A file sent from a device nearby: its file's name in the station's own folder, and the name it was sent by (its
/// place in the folder it was sent in, where it was sent in one: "folder/inner/name").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arrived {
    pub file: String,
    pub name: String,
}

impl Arrived {
    /// The folder it was sent in, where it was sent in one: the first part of the name it was sent by.
    pub fn folder(&self) -> Option<&str> {
        self.name.split_once('/').map(|(top, _)| top)
    }
}

/// One of the files sent from devices nearby as they are given out (see `entries`): a file sent by itself, or a folder
/// sent, by its name, with all of its files.
#[derive(Debug, PartialEq)]
pub enum Entry<'a> {
    File(&'a Arrived),
    Folder { name: &'a str, files: Vec<&'a Arrived> },
}

/// `files` as they are given out: each sent by itself as it is, and those sent in a folder as that folder, all of a
/// folder's together where the first of them came.
///
/// ```text
/// a.jpg, 资料/x.pdf, b.jpg, 资料/图/y.png  ->  a.jpg, 资料 (x.pdf, 图/y.png), b.jpg
/// ```
pub fn entries(files: &[Arrived]) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry> = Vec::new();
    for arrived in files {
        let Some(top) = arrived.folder() else {
            entries.push(Entry::File(arrived));
            continue;
        };
        let held = entries.iter_mut().find_map(|entry| match entry {
            Entry::Folder { name, files } if *name == top => Some(files),
            _ => None,
        });
        match held {
            Some(files) => files.push(arrived),
            None => entries.push(Entry::Folder { name: top, files: vec![arrived] }),
        }
    }
    entries
}

/// What a thing on the station is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    /// Files and folders put on it together, where they are (not copied: `kept` names a copy of each kept in the
    /// station's own folder, in the same order, where one is kept).
    Files { paths: Vec<PathBuf>, #[serde(default, skip_serializing_if = "Vec::is_empty")] kept: Vec<String> },
    /// Text, kept as it is.
    Text { text: String },
    /// A web address (not fetched: fetching would tell its site of each one put here).
    Link { url: String },
    /// A picture the station keeps itself (pasted, dragged from a page, sent by the phone): its file's name in its own
    /// folder, and the name it was given (none till it is renamed: see `Shelf::rename`), which it is given out by.
    Picture {
        file: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        name: String,
    },
    /// A file that had no path (dragged from a browser or a mail, its contents given): saved in its own folder as it
    /// came, by its name there and the name it was given. Or text too long to keep on the shelf itself (`words`),
    /// saved so, named by its first words: words still, given back as words.
    Saved {
        file: String,
        name: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        words: bool,
    },
    /// Files sent from a device nearby (see `Item::from`), together: those of one sending, and of those after it from
    /// the same device within a minute. Each kept in its own folder, as it came.
    Came { files: Vec<Arrived> },
}

impl What {
    /// The files of its own folder it keeps (to be removed with it).
    pub fn own_files(&self) -> Vec<&str> {
        match self {
            What::Files { kept, .. } => kept.iter().map(String::as_str).collect(),
            What::Picture { file, .. } | What::Saved { file, .. } => vec![file.as_str()],
            What::Came { files } => files.iter().map(|arrived| arrived.file.as_str()).collect(),
            What::Text { .. } | What::Link { .. } => Vec::new(),
        }
    }

    /// How it is renamed, where it has a name to rename: a file or folder alone, put on it where it is, renamed there;
    /// one the station keeps itself (a picture, a file saved as it came, or what one sending from a device nearby
    /// held: a file, or a folder), given out by a name of its own (see `rename`). Words and addresses have none, nor
    /// files put on it, or sent, together.
    pub fn renamed_by(&self) -> Option<Renamed<'_>> {
        match self {
            What::Files { paths, .. } => match paths.as_slice() {
                [path] => Some(Renamed::There(path)),
                _ => None,
            },
            What::Picture { name, .. } => Some(Renamed::Own { name: name.clone(), folder: false }),
            What::Saved { name, words: false, .. } => Some(Renamed::Own { name: name.clone(), folder: false }),
            What::Came { files } => match entries(files).as_slice() {
                [Entry::File(arrived)] => Some(Renamed::Own { name: arrived.name.clone(), folder: false }),
                [Entry::Folder { name, .. }] => Some(Renamed::Own { name: name.to_string(), folder: true }),
                _ => None,
            },
            What::Text { .. } | What::Link { .. } | What::Saved { words: true, .. } => None,
        }
    }

    /// Given out by `name` from now on, where it is one the station keeps itself (see `renamed_by`): a folder sent, all
    /// its files under it. Whether it is.
    fn rename(&mut self, name: &str) -> bool {
        match self {
            What::Picture { name: was, .. } | What::Saved { name: was, words: false, .. } => *was = name.to_string(),
            What::Came { files } => {
                let one = match entries(files).as_slice() {
                    [Entry::File(_)] => false,
                    [Entry::Folder { .. }] => true,
                    _ => return false,
                };
                for arrived in files.iter_mut() {
                    arrived.name = match arrived.name.split_once('/') {
                        Some((_, under)) if one => format!("{name}/{under}"),
                        _ => name.to_string(),
                    };
                }
            }
            What::Files { .. } | What::Text { .. } | What::Link { .. } | What::Saved { words: true, .. } => return false,
        }
        true
    }

    /// Its files where they are, the file or folder at `from` (and what is in it) now at `to`. Whether any was.
    fn moved(&mut self, from: &Path, to: &Path) -> bool {
        let What::Files { paths, .. } = self else { return false };
        let mut changed = false;
        for path in paths.iter_mut() {
            if let Ok(under) = path.strip_prefix(from) {
                *path = if under.as_os_str().is_empty() { to.to_path_buf() } else { to.join(under) };
                changed = true;
            }
        }
        changed
    }
}

/// How a thing on the station is renamed (see `What::renamed_by`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Renamed<'a> {
    /// The file or folder where it is, renamed there.
    There(&'a Path),
    /// One the station keeps itself, given out by `name` (none yet: a picture never renamed), a `folder` sent.
    Own { name: String, folder: bool },
}

/// Which of what is on the station, or was copied lately, its page shows: all; what is pinned; what is of one tag, by
/// its number; and of what was copied, words (text, and links: a link copied is words too), pictures (what was copied as
/// a picture, a screenshot among them, and picture files: files all of them pictures), files (files and folders copied
/// where they are, picture files too, and files that came with no path).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Pinned,
    Tagged(u32),
    Words,
    Pictures,
    Files,
}

/// How many of what is pinned are listed first where all of a page is (see `listed`).
pub const PINNED_FIRST: usize = 3;

impl Filter {
    /// The kinds of what was copied, as its page offers them.
    pub const KINDS: [Filter; 3] = [Filter::Words, Filter::Pictures, Filter::Files];

    /// The same, as `tags` are now: of a tag that is gone, all.
    pub fn among(self, tags: &[Tag]) -> Filter {
        match self {
            Filter::Tagged(id) if !tags.iter().any(|tag| tag.id == id) => Filter::All,
            filter => filter,
        }
    }

    /// It as a number, each its own.
    pub fn number(self) -> u32 {
        match self {
            Filter::All => 0,
            Filter::Words => 1,
            Filter::Pictures => 2,
            Filter::Files => 3,
            Filter::Pinned => 4,
            Filter::Tagged(tag) => 5 + tag,
        }
    }

    /// Whether `item` is one it shows.
    pub fn takes(self, item: &Item) -> bool {
        match self {
            Filter::Pinned => item.pinned,
            Filter::Tagged(tag) => item.tags.contains(&tag),
            filter => filter.takes_what(&item.what),
        }
    }

    /// Whether `what` is of its kind (what is pinned, or of a tag: of any).
    fn takes_what(self, what: &What) -> bool {
        let picture = |path: &std::path::Path| path.extension().is_some_and(|extension| super::recent::is_picture(&extension.to_string_lossy().to_lowercase()));
        match self {
            Filter::All | Filter::Pinned | Filter::Tagged(_) => true,
            Filter::Words => matches!(what, What::Text { .. } | What::Link { .. } | What::Saved { words: true, .. }),
            Filter::Pictures => match what {
                What::Picture { .. } => true,
                What::Files { paths, .. } => !paths.is_empty() && paths.iter().all(|path| picture(path)),
                What::Saved { name, words, .. } => !words && picture(std::path::Path::new(name)),
                What::Came { files } => !files.is_empty() && entries(files).iter().all(|entry| matches!(entry, Entry::File(arrived) if picture(std::path::Path::new(&arrived.name)))),
                What::Text { .. } | What::Link { .. } => false,
            },
            Filter::Files => matches!(what, What::Files { .. } | What::Saved { words: false, .. } | What::Came { .. }),
        }
    }
}

/// What a page lists, in its order (see `listed`): the pinned things listed first, under a heading of their own, how many
/// are pinned in all (none: no such heading), and the rest. The pinned things not listed first are kept apart (`more`),
/// for where the page is shown whole without its headings (see `every`).
#[derive(Clone, Debug, PartialEq)]
pub struct Listed<T> {
    pub first: Vec<T>,
    pub pinned: usize,
    pub rest: Vec<T>,
    pub more: Vec<T>,
}

impl<T> Listed<T> {
    /// All of a page: what is `pinned` first (the first `PINNED_FIRST` of it, or, `folded`, none), `rest` after.
    pub fn grouped(mut pinned: Vec<T>, rest: Vec<T>, folded: bool) -> Listed<T> {
        let of = pinned.len();
        let more = pinned.split_off(if folded { 0 } else { PINNED_FIRST.min(of) });
        Listed { first: pinned, pinned: of, rest, more }
    }

    /// What is under one filter: no groups.
    pub fn plain(rest: Vec<T>) -> Listed<T> {
        Listed { first: Vec::new(), pinned: 0, rest, more: Vec::new() }
    }

    /// All of the page with no headings: all that is pinned first, the rest after.
    pub fn every(self) -> Vec<T> {
        self.first.into_iter().chain(self.more).chain(self.rest).collect()
    }
}

/// What the station's page lists of `items`, or (`copied`) the page of what was copied lately, under `filter`: with all
/// of it, what is pinned first (the last used first, `folded`: none of them), what is not after; what is pinned alone;
/// of a tag or a kind, by when it came, what is pinned among the rest.
pub fn listed(items: &[Item], copied: bool, filter: Filter, folded: bool) -> Listed<&Item> {
    let newest = |filter: Filter| items.iter().rev().filter(move |item| item.copied == copied && filter.takes(item));
    let pinned = || {
        let mut pinned: Vec<&Item> = newest(Filter::Pinned).collect();
        pinned.sort_by_key(|item| std::cmp::Reverse(item.last_ms()));
        pinned
    };
    match filter {
        Filter::All => Listed::grouped(pinned(), newest(Filter::All).filter(|item| !item.pinned).collect(), folded),
        Filter::Pinned => Listed::plain(pinned()),
        filter => Listed::plain(newest(filter).collect()),
    }
}

/// How long the station keeps what does not stay, and how much of its own it keeps at most.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub keep_days: u32,
    pub most_bytes: u64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { keep_days: 7, most_bytes: 2 << 30 }
    }
}

/// No room for what was to be put on the station, even with all it keeps of what does not stay gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// The things on the station, the oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub items: Vec<Item>,
    /// The number the next thing put on it takes.
    #[serde(default)]
    next: u64,
    /// The tags there are, in their order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    /// The number the next tag takes.
    #[serde(default)]
    next_tag: u32,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// How soon after what came from a device what it sends next joins it (ms).
pub const TOGETHER_MS: u64 = 60 * 1000;

impl Shelf {
    /// Puts `what` on the shelf at `now_ms` (`own_bytes` of it kept in its own folder; see `room_for` first, for that
    /// much): its number.
    pub fn put(&mut self, what: What, now_ms: u64, copied: bool, stays_here: bool, own_bytes: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        self.items.push(Item { id, what, added_ms: now_ms, pinned: false, copied, stays_here, own_bytes, from: None, used_ms: 0, tags: Vec::new(), held: false, freed_ms: 0 });
        id
    }

    /// Files sent from `from`, kept in its own folder (`own_bytes` of them, room made for them first), put on the shelf
    /// at `now_ms`: with what came from that device last, where that came within `TOGETHER_MS` before and does not
    /// stay, else on their own. The number of what holds them.
    pub fn arrived(&mut self, files: Vec<Arrived>, from: Device, now_ms: u64, own_bytes: u64) -> u64 {
        self.came(What::Came { files }, from, now_ms, own_bytes)
    }

    /// Files sent from `from` that went to the folder chosen for them (see `Keep::arrived_into`), put on the shelf at
    /// `now_ms` where they are (each file, and each folder sent: `paths`), none kept of its own: joined as `arrived`
    /// joins them. The number of what holds them.
    pub fn arrived_at(&mut self, paths: Vec<PathBuf>, from: Device, now_ms: u64) -> u64 {
        self.came(What::Files { paths, kept: Vec::new() }, from, now_ms, 0)
    }

    /// What came from `from` (`what`: files kept, or files where they went) put on the shelf at `now_ms`: with what came
    /// from that device last as the same, where that came within `TOGETHER_MS` before and does not stay, else on its
    /// own.
    fn came(&mut self, what: What, from: Device, now_ms: u64, own_bytes: u64) -> u64 {
        let last = self.items.iter().rposition(|item| {
            std::mem::discriminant(&item.what) == std::mem::discriminant(&what) && !item.stays() && item.from.as_ref().is_some_and(|device| device.fingerprint == from.fingerprint) && now_ms.saturating_sub(item.added_ms) <= TOGETHER_MS
        });
        match last {
            // (Joined, it is as new as what joined it: the newest, last.)
            Some(at) => {
                let mut item = self.items.remove(at);
                match (&mut item.what, what) {
                    (What::Came { files: held }, What::Came { files }) => held.extend(files),
                    (What::Files { paths: held, .. }, What::Files { paths, .. }) => {
                        for path in paths {
                            if !held.contains(&path) {
                                held.push(path);
                            }
                        }
                    }
                    _ => unreachable!("joined only with what is of its kind"),
                }
                item.added_ms = now_ms;
                item.own_bytes += own_bytes;
                item.from = Some(from);
                let id = item.id;
                self.items.push(item);
                id
            }
            None => {
                let id = self.put(what, now_ms, false, false, own_bytes);
                self.items.last_mut().expect("just put").from = Some(from);
                id
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Takes `id` off the shelf: what it was (its own files to be removed).
    pub fn take(&mut self, id: u64) -> Option<Item> {
        let at = self.items.iter().position(|item| item.id == id)?;
        Some(self.items.remove(at))
    }

    /// The thing `id` pinned, or not, at `now_ms`.
    pub fn pin(&mut self, id: u64, pinned: bool, now_ms: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.change(now_ms, |item| item.pinned = pinned);
        }
    }

    /// The things that hold a file pinned or tagged among the files come lately as they are at `now_ms` (see
    /// `Item::held`): those `holds` says. Whether any other than before.
    pub fn hold(&mut self, now_ms: u64, holds: impl Fn(&Item) -> bool) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            let held = holds(item);
            changed |= item.held != held;
            item.change(now_ms, |item| item.held = held);
        }
        changed
    }

    /// The thing `id` used at `now_ms`: put on the clipboard from here.
    pub fn used(&mut self, id: u64, now_ms: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.used_ms = now_ms;
        }
    }

    /// The thing `id` copied again at `now_ms`: the newest now, staying here as its copier said this time (see
    /// `Item::stays_here`).
    pub fn copied_again(&mut self, id: u64, stays_here: bool, now_ms: u64) {
        if let Some(at) = self.items.iter().position(|item| item.id == id) {
            let mut item = self.items.remove(at);
            (item.added_ms, item.used_ms, item.stays_here) = (now_ms, now_ms, stays_here);
            self.items.push(item);
        }
    }

    /// The things `ids` of the tag `tag`, or not of it any more, at `now_ms`.
    pub fn tag(&mut self, ids: &[u64], tag: u32, on: bool, now_ms: u64) {
        if !self.tags.iter().any(|kept| kept.id == tag) {
            return;
        }
        for item in self.items.iter_mut().filter(|item| ids.contains(&item.id)) {
            item.change(now_ms, |item| {
                if !on {
                    item.tags.retain(|of| *of != tag);
                } else if !item.tags.contains(&tag) {
                    item.tags.push(tag);
                }
            });
        }
    }

    /// A tag named `name` (the one there is of that name, where there is one): its number. None for a name of nothing
    /// but spaces.
    pub fn new_tag(&mut self, name: &str) -> Option<u32> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(tag) = self.tags.iter().find(|tag| tag.name == name) {
            return Some(tag.id);
        }
        let id = self.next_tag;
        self.next_tag += 1;
        self.tags.push(Tag { id, name: name.to_string() });
        Some(id)
    }

    /// The tag `id` named `name` from now on: left as it is for a name of nothing but spaces, or one another has.
    pub fn rename_tag(&mut self, id: u32, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.tags.iter().any(|tag| tag.name == name) {
            return;
        }
        if let Some(tag) = self.tags.iter_mut().find(|tag| tag.id == id) {
            tag.name = name.to_string();
        }
    }

    /// The tag named `was`, just made with the things `ids` of it, named `name` from now on (see `rename_tag`); where
    /// another tag has that name, those things of that one instead and the one just made gone, as a tag asked to be
    /// made by that name is that one (see `new_tag`): its number then.
    pub fn rename_new_tag(&mut self, was: &str, name: &str, ids: &[u64], now_ms: u64) -> Option<u32> {
        let made = self.tags.iter().find(|tag| tag.name == was)?.id;
        let Some(other) = self.tags.iter().find(|tag| tag.id != made && tag.name == name.trim()).map(|tag| tag.id) else {
            self.rename_tag(made, name);
            return None;
        };
        self.tag(ids, other, true, now_ms);
        self.remove_tag(made, now_ms);
        Some(other)
    }

    /// `id`, one it keeps itself, given out by `name` from now on (see `What::renamed_by`). Whether it is.
    pub fn rename(&mut self, id: u64, name: &str) -> bool {
        self.items.iter_mut().find(|item| item.id == id).is_some_and(|item| item.what.rename(name))
    }

    /// The file or folder at `from` renamed to `to` (by Glance, or seen so): what holds it, or what is in it, holds
    /// it there. Whether anything does.
    pub fn moved(&mut self, from: &Path, to: &Path) -> bool {
        self.items.iter_mut().fold(false, |changed, item| item.what.moved(from, to) || changed)
    }

    /// The tag `id` gone at `now_ms`, what was of it left.
    pub fn remove_tag(&mut self, id: u32, now_ms: u64) {
        self.tags.retain(|tag| tag.id != id);
        for item in &mut self.items {
            item.change(now_ms, |item| item.tags.retain(|of| *of != id));
        }
    }

    /// The bytes it keeps of its own, of all on it.
    pub fn own_bytes(&self) -> u64 {
        self.items.iter().map(|item| item.own_bytes).sum()
    }

    /// Those of `of` that do not stay, past the `most` newest of them (see `Item::since_ms`), by their numbers.
    pub fn past(&self, most: usize, of: impl Fn(&Item) -> bool) -> Vec<u64> {
        let mut loose: Vec<&Item> = self.items.iter().filter(|item| !item.stays() && of(item)).collect();
        loose.sort_by_key(|item| item.since_ms());
        loose[..loose.len().saturating_sub(most)].iter().map(|item| item.id).collect()
    }

    /// Takes off what has been on it longer than `rules` keep it and does not stay: what went.
    pub fn expire(&mut self, rules: Rules, now_ms: u64) -> Vec<Item> {
        let keep = u64::from(rules.keep_days) * DAY_MS;
        let (gone, kept) = std::mem::take(&mut self.items).into_iter().partition(|item| !item.stays() && now_ms.saturating_sub(item.since_ms()) >= keep);
        self.items = kept;
        gone
    }

    /// The most it can make room for within `rules` (see `room_for`): all but what those that stay keep of their own.
    pub fn room(&self, rules: Rules) -> u64 {
        rules.most_bytes.saturating_sub(self.items.iter().filter(|item| item.stays()).map(|item| item.own_bytes).sum())
    }

    /// Makes room for `bytes` more of its own within `rules`: the oldest things that do not stay and keep something of
    /// their own taken off till there is (what went); else, with no room even with all of those gone, nothing taken
    /// off, and `Full`.
    pub fn room_for(&mut self, bytes: u64, rules: Rules) -> Result<Vec<Item>, Full> {
        if bytes > self.room(rules) {
            return Err(Full);
        }
        let mut over = (self.own_bytes() + bytes).saturating_sub(rules.most_bytes);
        let mut gone = Vec::new();
        while over > 0 {
            // (There is one: what does not stay holds the rest, at least `over`.)
            let loose = self.items.iter().enumerate().filter(|(_, item)| !item.stays() && item.own_bytes > 0);
            let (at, _) = loose.min_by_key(|(_, item)| item.since_ms()).expect("something that does not stay holds the rest");
            let item = self.items.remove(at);
            over = over.saturating_sub(item.own_bytes);
            gone.push(item);
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> What {
        What::Text { text: t.into() }
    }

    fn picture(name: &str) -> What {
        What::Picture { file: name.into(), name: String::new() }
    }

    #[test]
    fn what_was_copied_is_sorted_into_words_pictures_and_files() {
        let link = What::Link { url: "https://glancepc.com".into() };
        let files = |paths: &[&str]| What::Files { paths: paths.iter().map(PathBuf::from).collect(), kept: Vec::new() };
        let saved = |name: &str| What::Saved { file: "1".into(), name: name.into(), words: false };
        let all = [
            text("hello"),
            link,
            picture("1.png"),
            files(&[r"D:\Photos\a.JPG", r"D:\Photos\b.png"]),
            files(&[r"D:\Photos\a.jpg", r"D:\Notes\b.txt"]),
            files(&[r"D:\Photos"]),
            saved("bill.pdf"),
            saved("cat.webp"),
            What::Saved { file: "2".into(), name: "很长的一段文字.txt".into(), words: true },
        ];
        let taken = |filter: Filter| all.iter().enumerate().filter(|(_, what)| filter.takes_what(what)).map(|(at, _)| at).collect::<Vec<_>>();
        assert_eq!(taken(Filter::All), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // (Text too long for the shelf, saved as a file, is words.)
        assert_eq!(taken(Filter::Words), [0, 1, 8]);
        // Picture files are pictures and files both (files with one not a picture, a folder, files alone).
        assert_eq!(taken(Filter::Pictures), [2, 3, 7]);
        assert_eq!(taken(Filter::Files), [3, 4, 5, 6, 7]);
    }

    /// What has a name is renamed: a file or folder alone where it is, there; one kept here (a picture, a file saved as
    /// it came, a file or a folder sent alone) by a name of its own, a folder's files under it. Words, addresses, files
    /// together and sendings of several have none. A file or folder renamed where it is is followed by what holds it,
    /// or what is in it, and nothing else.
    #[test]
    fn what_has_a_name_is_renamed() {
        let arrived = |name: &str| Arrived { file: format!("n1-{name}"), name: name.into() };
        let files = |paths: &[&str]| What::Files { paths: paths.iter().map(PathBuf::from).collect(), kept: Vec::new() };
        let mut shelf = Shelf::default();
        let ids = [
            files(&[r"D:\a.jpg"]),
            picture("1.png"),
            What::Saved { file: "2".into(), name: "bill.pdf".into(), words: false },
            What::Came { files: vec![arrived("x.jpg")] },
            What::Came { files: vec![arrived("资料/x.pdf"), arrived("资料/图/y.png")] },
            text("a"),
            What::Link { url: "https://glancepc.com".into() },
            files(&[r"D:\a.jpg", r"D:\b.jpg"]),
            What::Came { files: vec![arrived("x.jpg"), arrived("y.jpg")] },
            What::Saved { file: "3".into(), name: "很长.txt".into(), words: true },
        ]
        .map(|what| shelf.put(what, 0, false, false, 0));
        let by = |shelf: &Shelf, at: usize| shelf.get(ids[at]).unwrap().what.renamed_by().map(|renamed| match renamed {
            Renamed::There(path) => path.display().to_string(),
            Renamed::Own { name, folder } => format!("{name}{}", if folder { "/" } else { "" }),
        });
        let named: Vec<_> = (0..ids.len()).map(|at| by(&shelf, at)).collect();
        assert_eq!(named, [Some(r"D:\a.jpg".into()), Some(String::new()), Some("bill.pdf".into()), Some("x.jpg".into()), Some("资料/".into()), None, None, None, None, None]);
        for (at, name) in [(1, "白板.png"), (2, "账单.pdf"), (3, "海.jpg"), (4, "旧资料")] {
            assert!(shelf.rename(ids[at], name));
            assert_eq!(by(&shelf, at).unwrap().trim_end_matches('/'), name);
        }
        let What::Came { files: sent } = &shelf.get(ids[4]).unwrap().what else { unreachable!() };
        assert_eq!(sent.iter().map(|one| one.name.as_str()).collect::<Vec<_>>(), ["旧资料/x.pdf", "旧资料/图/y.png"]);
        // (Kept by the names they were kept by.)
        assert_eq!(sent[0].file, "n1-资料/x.pdf");
        for at in [0, 5, 7, 8, 9] {
            assert!(!shelf.rename(ids[at], "b"));
        }
        // Renamed where they are.
        let put = shelf.put(files(&[r"D:\照片\c.jpg", r"D:\照片2\d.jpg"]), 0, false, false, 0);
        assert!(shelf.moved(Path::new(r"D:\a.jpg"), Path::new(r"D:\海.jpg")));
        assert!(shelf.moved(Path::new(r"D:\照片"), Path::new(r"D:\旅行")));
        assert!(!shelf.moved(Path::new(r"D:\无"), Path::new(r"D:\有")));
        let paths = |id: u64| match &shelf.get(id).unwrap().what {
            What::Files { paths, .. } => paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        assert_eq!(paths(ids[0]), [r"D:\海.jpg"]);
        assert_eq!(paths(ids[7]), [r"D:\海.jpg", r"D:\b.jpg"]);
        assert_eq!(paths(put), [r"D:\旅行\c.jpg", r"D:\照片2\d.jpg"]);
    }

    /// A tag just made, named as it is written in where it shows: renamed so; named as another tag is, its things that
    /// one's instead and it gone (as a tag asked to be made by that name is that one). Named as nothing (spaces), or as
    /// it was, it stays as it is.
    #[test]
    fn a_tag_just_made_named_as_another_joins_it() {
        let mut shelf = Shelf::default();
        let ids = [text("a"), text("b")].map(|what| shelf.put(what, 0, false, false, 0));
        let work = shelf.new_tag("工作").unwrap();
        let made = shelf.new_tag("新建标签").unwrap();
        shelf.tag(&ids, made, true, 0);
        assert_eq!(shelf.rename_new_tag("新建标签", "  ", &ids, 0), None);
        assert_eq!(shelf.rename_new_tag("新建标签", "新建标签", &ids, 0), None);
        assert_eq!(shelf.tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(), ["工作", "新建标签"]);
        assert_eq!(shelf.rename_new_tag("新建标签", " 工作 ", &ids, 0), Some(work));
        assert_eq!(shelf.tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(), ["工作"]);
        assert!(ids.iter().all(|id| shelf.get(*id).unwrap().tags == [work]));
        let made = shelf.new_tag("新建标签").unwrap();
        assert_eq!(shelf.rename_new_tag("新建标签", "旅行", &[], 0), None);
        assert_eq!(shelf.tags.iter().find(|tag| tag.id == made).unwrap().name, "旅行");
        assert_eq!(shelf.rename_new_tag("无此标签", "旅行", &[], 0), None);
    }

    /// With all that was copied, what is pinned is listed first, the three last used, and only there. Folded, none of
    /// them. What is pinned is listed by when it was last used. Of a tag or of a kind, all of it by when it came, pinned
    /// or not. What was put on the station is on its own page, and listed the same.
    #[test]
    fn what_is_pinned_is_listed_first() {
        let mut shelf = Shelf::default();
        let ids: Vec<u64> = (0..7).map(|at| shelf.put(text(&at.to_string()), at, true, false, 0)).collect();
        let put = [8, 9].map(|at| shelf.put(text("put there"), at, false, false, 0));
        for at in [0, 1, 2, 4] {
            shelf.pin(ids[at], true, 0);
        }
        let of = |items: &[&Item]| items.iter().map(|item| item.id).collect::<Vec<_>>();
        let listed = |shelf: &Shelf, filter: Filter, folded: bool| {
            let listed = listed(&shelf.items, true, filter, folded);
            (of(&listed.first), listed.pinned, of(&listed.rest))
        };
        assert_eq!(listed(&shelf, Filter::All, false), (vec![4, 2, 1], 4, vec![6, 5, 3]));
        assert_eq!(listed(&shelf, Filter::All, true), (vec![], 4, vec![6, 5, 3]));
        // The oldest of them used: first of them.
        shelf.used(0, 20);
        assert_eq!(listed(&shelf, Filter::All, false), (vec![0, 4, 2], 4, vec![6, 5, 3]));
        assert_eq!(listed(&shelf, Filter::Pinned, false), (vec![], 0, vec![0, 4, 2, 1]));
        assert_eq!(listed(&shelf, Filter::Words, false), (vec![], 0, vec![6, 5, 4, 3, 2, 1, 0]));
        let work = shelf.new_tag("work").unwrap();
        shelf.tag(&[1, 3], work, true, 0);
        assert_eq!(listed(&shelf, Filter::Tagged(work), false), (vec![], 0, vec![3, 1]));
        assert_eq!(listed(&shelf, Filter::All, false), (vec![0, 4, 2], 4, vec![6, 5, 3]));
        let page = |shelf: &Shelf| {
            let listed = super::listed(&shelf.items, false, Filter::All, false);
            (of(&listed.first), listed.pinned, of(&listed.rest))
        };
        assert_eq!(page(&shelf), (vec![], 0, vec![put[1], put[0]]));
        shelf.pin(put[0], true, 0);
        assert_eq!(page(&shelf), (vec![put[0]], 1, vec![put[1]]));
        // With nothing pinned, no heading of it.
        (0..7).for_each(|at| shelf.pin(ids[at], false, 0));
        assert_eq!(listed(&shelf, Filter::All, false), (vec![], 0, vec![6, 5, 4, 3, 2, 1, 0]));
    }

    /// Tags and pins are nothing to one another, on what was copied and what was put on the station alike. A tag gone
    /// leaves what was of it. A name there is already is that tag's, and no other may take it.
    #[test]
    fn a_tag_pins_nothing_and_goes_without_what_it_is_on() {
        let mut shelf = Shelf::default();
        let copied = shelf.put(text("a"), 0, true, false, 0);
        let other = shelf.put(text("b"), 1, true, false, 0);
        let put = shelf.put(text("c"), 2, false, false, 0);
        let one = shelf.new_tag(" one ").unwrap();
        assert_eq!((shelf.new_tag("one"), shelf.new_tag("  ")), (Some(one), None));
        let two = shelf.new_tag("two").unwrap();
        shelf.tag(&[copied, other, put], one, true, 3);
        shelf.tag(&[copied], two, true, 3);
        // (Put on it again, it is where it was.)
        shelf.tag(&[copied], one, true, 3);
        let of = |shelf: &Shelf, id: u64| shelf.get(id).map(|item| (item.pinned, item.tags.clone())).unwrap();
        assert_eq!((of(&shelf, copied), of(&shelf, other), of(&shelf, put)), ((false, vec![one, two]), (false, vec![one]), (false, vec![one])));
        shelf.pin(other, true, 3);
        assert_eq!(of(&shelf, other), (true, vec![one]));
        shelf.pin(other, false, 3);
        assert_eq!(of(&shelf, other), (false, vec![one]));
        shelf.rename_tag(two, "one");
        shelf.rename_tag(one, " first ");
        assert_eq!(shelf.tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(), ["first", "two"]);
        shelf.remove_tag(one, 4);
        assert_eq!((of(&shelf, copied), of(&shelf, put)), ((false, vec![two]), (false, vec![])));
        assert_eq!(Filter::Tagged(one).among(&shelf.tags), Filter::All);
        assert_eq!(Filter::Tagged(two).among(&shelf.tags), Filter::Tagged(two));
        // (Its number never another's.)
        assert_ne!(shelf.new_tag("one"), Some(one));
        let read: Shelf = serde_json::from_str(&serde_json::to_string(&shelf).unwrap()).unwrap();
        assert_eq!(read, shelf);
    }

    /// What is pinned, tagged, or holds a file that is, stays. What no longer is goes after its days from then.
    #[test]
    fn what_is_neither_pinned_nor_tagged_goes_after_its_days() {
        let rules = Rules::default();
        let mut shelf = Shelf::default();
        let old = shelf.put(text("old"), 0, false, false, 0);
        let pinned = shelf.put(text("pinned"), 0, false, false, 0);
        let tagged = shelf.put(text("tagged"), 0, true, false, 0);
        let held = shelf.put(text("held"), 0, false, false, 0);
        let new = shelf.put(text("new"), 6 * DAY_MS, false, false, 0);
        shelf.pin(pinned, true, 0);
        let tag = shelf.new_tag("t").unwrap();
        shelf.tag(&[tagged], tag, true, 0);
        assert!(shelf.hold(0, |item| item.id == held) && !shelf.hold(0, |item| item.id == held));
        let ids = |items: &[Item]| items.iter().map(|item| item.id).collect::<Vec<_>>();
        assert_eq!(ids(&shelf.expire(rules, 7 * DAY_MS)), [old]);
        assert_eq!(ids(&shelf.items), [pinned, tagged, held, new]);
        // Kept longer, as set.
        let mut longer = shelf.clone();
        assert!(longer.expire(Rules { keep_days: 30, ..rules }, 13 * DAY_MS).is_empty());
        // Pinned and tagged both, one of the two gone: it stays still.
        let mut both = shelf.clone();
        both.tag(&[pinned], tag, true, 0);
        both.pin(pinned, false, 20 * DAY_MS);
        assert!(!ids(&both.expire(rules, 40 * DAY_MS)).contains(&pinned));
        shelf.pin(pinned, false, 20 * DAY_MS);
        shelf.remove_tag(tag, 21 * DAY_MS);
        shelf.hold(22 * DAY_MS, |_| false);
        assert_eq!(ids(&shelf.expire(rules, 27 * DAY_MS - 1)), [new]);
        assert_eq!(ids(&shelf.expire(rules, 27 * DAY_MS)), [pinned]);
        assert_eq!(ids(&shelf.expire(rules, 28 * DAY_MS)), [tagged]);
        assert_eq!(ids(&shelf.expire(rules, 29 * DAY_MS)), [held]);
    }

    /// Past the most kept, the oldest of those that do not stay, by when they came or stopped staying.
    #[test]
    fn the_oldest_past_the_most_go() {
        let mut shelf = Shelf::default();
        let ids: Vec<u64> = (0..5).map(|at| shelf.put(text(&at.to_string()), at, true, false, 0)).collect();
        shelf.pin(ids[0], true, 0);
        shelf.pin(ids[1], true, 0);
        shelf.pin(ids[1], false, 9);
        assert_eq!(shelf.past(2, |item| item.copied), [ids[2], ids[3]]);
        assert!(shelf.past(2, |item| !item.copied).is_empty());
    }

    #[test]
    fn room_is_made_from_the_oldest_that_do_not_stay() {
        let rules = Rules { keep_days: 7, most_bytes: 100 };
        let mut shelf = Shelf::default();
        let first = shelf.put(picture("a"), 0, false, false, 40);
        let pinned = shelf.put(picture("b"), 1, false, false, 30);
        shelf.pin(pinned, true, 0);
        let second = shelf.put(picture("c"), 2, false, false, 20);
        // (Pointing to files keeps nothing of its own: never taken off for room.)
        let files = shelf.put(What::Files { paths: vec!["C:\\a.txt".into()], kept: Vec::new() }, 0, false, false, 0);
        // 90 held: 10 more fit.
        assert_eq!(shelf.room_for(10, rules), Ok(Vec::new()));
        // 30 more: the oldest that does not stay goes.
        let gone = shelf.room_for(30, rules).unwrap();
        assert_eq!(gone.iter().map(|item| item.id).collect::<Vec<_>>(), [first]);
        assert!(shelf.get(files).is_some() && shelf.get(second).is_some());
    }

    #[test]
    fn what_stays_is_never_taken_off_for_room() {
        let rules = Rules { keep_days: 7, most_bytes: 100 };
        let mut shelf = Shelf::default();
        assert_eq!(shelf.room(rules), 100);
        let pinned = shelf.put(picture("a"), 0, false, false, 50);
        shelf.pin(pinned, true, 0);
        let tagged = shelf.put(picture("t"), 0, false, false, 30);
        let tag = shelf.new_tag("t").unwrap();
        shelf.tag(&[tagged], tag, true, 0);
        let loose = shelf.put(picture("b"), 1, false, false, 10);
        // More than what stays leaves: full, and nothing taken off.
        assert_eq!(shelf.room_for(30, rules), Err(Full));
        assert_eq!(shelf.items.len(), 3);
        // What fits beside what stays: the loose one goes.
        assert_eq!(shelf.room_for(20, rules).unwrap().iter().map(|item| item.id).collect::<Vec<_>>(), [loose]);
    }

    #[test]
    fn numbers_are_never_given_again() {
        let mut shelf = Shelf::default();
        let a = shelf.put(text("a"), 0, false, false, 0);
        shelf.take(a);
        let b = shelf.put(text("b"), 0, false, false, 0);
        assert_ne!(a, b);
        // Kept as written and read back.
        let read: Shelf = serde_json::from_str(&serde_json::to_string(&shelf).unwrap()).unwrap();
        assert_eq!(read, shelf);
        let mut read = read;
        assert_ne!(read.put(text("c"), 0, false, false, 0), b);
    }
}
